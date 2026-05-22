//! Pairwise Compare backend types.

#![allow(missing_docs)]

use crate::backends::{BackendCallOutcome, BackendError, CostEstimate, RunMode};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use crate::handlers::util::image_arg_to_url;
use super::identity_check::IdentityCheckRequest;
use super::instruction_edit::InstructionEditRequest;
use super::low_denoise::LowDenoiseImg2ImgRequest;
use super::upscale::cosine_similarity;
use super::instruction_edit::finding_to_kontext_instruction;
use super::instruction_edit::region_to_instruction_hint;
use super::bg_remove::BgRemoveRequest;
use super::face_detect::{FaceDetectRequest, FaceDetectResult, FaceDetection};
use super::ocr::{OcrRequest, OcrResult, OcrDetection};
use super::vision_verify::FindingStatus;
use super::vision_verify::Finding;

/// One VISTA-style pairwise comparison request — the VLM grades two
/// candidate stills against the brief across four dimensions and picks
/// a winner per dimension. The pair-aggregator (`variants::aggregate_pair`)
/// then turns the dim-level verdicts into a single pair verdict.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PairwiseCompareRequest {
    /// Image URL of candidate A (`https://…` or `data:` URL). Caller
    /// routes local paths through `image_arg_to_url` first.
    pub image_a_url: String,
    /// Image URL of candidate B — same constraints.
    pub image_b_url: String,
    /// Excerpt from the brief that names the subject, the desired mood,
    /// the audience. Kept short — the VLM doesn't need the whole brief.
    pub brief_excerpt: String,
    /// The specific shot's prompt — what this frame is supposed to show.
    pub shot_prompt: String,
}

impl PairwiseCompareRequest {
    /// Build from two image URLs + brief context + shot prompt.
    pub fn new(
        image_a_url: impl Into<String>,
        image_b_url: impl Into<String>,
        brief_excerpt: impl Into<String>,
        shot_prompt: impl Into<String>,
    ) -> Self {
        Self {
            image_a_url: image_a_url.into(),
            image_b_url: image_b_url.into(),
            brief_excerpt: brief_excerpt.into(),
            shot_prompt: shot_prompt.into(),
        }
    }
}

/// VISTA pairwise call result — mirrors `variants::PairJudgments` but
/// kept in the backends layer so adapters don't need to know about the
/// variant orchestration module.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PairwiseCompareResult {
    /// Provider identifier (`fal-pairwise-compare`).
    pub provider: String,
    /// Verdict on subject fidelity — `"A"` | `"B"` | `"tie"`.
    pub subject_fidelity: String,
    /// Verdict on composition.
    pub composition: String,
    /// Verdict on lighting + color.
    pub lighting_color: String,
    /// Verdict on production polish.
    pub production: String,
    /// Free-form rationale (one sentence per dimension, max four).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub rationale: String,
}

/// Cluster trait shared by every pairwise-comparison adapter.
pub trait PairwiseCompareBackend {
    /// Provider name.
    fn name(&self) -> &'static str;
    /// Cost estimate for one pair comparison.
    fn estimate_cost(&self, request: &PairwiseCompareRequest) -> CostEstimate;
    /// Run the pair comparison. Returns the four per-dim verdicts.
    fn compare(
        &self,
        request: &PairwiseCompareRequest,
        mode: RunMode,
    ) -> Result<BackendCallOutcome<PairwiseCompareResult>, BackendError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_round_trips() {
        let req = BgRemoveRequest::new("https://example.com/car.jpg");
        let json = serde_json::to_string(&req).unwrap();
        let back: BgRemoveRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(back.image, "https://example.com/car.jpg");
    }

    #[test]
    fn identity_check_request_round_trips() {
        let req = IdentityCheckRequest::new(
            "https://example.com/ref.jpg",
            "https://example.com/cand.jpg",
        );
        let json = serde_json::to_string(&req).unwrap();
        let back: IdentityCheckRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(back.reference_url, "https://example.com/ref.jpg");
        assert_eq!(back.candidate_url, "https://example.com/cand.jpg");
    }

    #[test]
    fn ocr_result_round_trips() {
        let result = OcrResult {
            provider: "roboflow-doctr".into(),
            detections: vec![
                OcrDetection {
                    text: "STOP".into(),
                    bbox: Some([10, 20, 100, 40]),
                    confidence: Some(0.91),
                },
                OcrDetection {
                    text: "ONE WAY".into(),
                    bbox: None,
                    confidence: None,
                },
            ],
            combined_text: "STOP\nONE WAY".into(),
        };
        let json = serde_json::to_string(&result).unwrap();
        let back: OcrResult = serde_json::from_str(&json).unwrap();
        assert_eq!(back.provider, "roboflow-doctr");
        assert_eq!(back.detections.len(), 2);
        assert_eq!(back.detections[0].text, "STOP");
        assert_eq!(back.detections[0].bbox, Some([10, 20, 100, 40]));
        assert!(back.detections[1].bbox.is_none());
        assert_eq!(back.combined_text, "STOP\nONE WAY");
    }

    #[test]
    fn cosine_similarity_identical_is_one() {
        let v = vec![0.2f32, -0.5, 0.8, 0.1];
        let s = cosine_similarity(&v, &v);
        assert!((s - 1.0).abs() < 1e-5, "got {s}");
    }

    #[test]
    fn cosine_similarity_orthogonal_is_zero() {
        let a = vec![1.0f32, 0.0];
        let b = vec![0.0f32, 1.0];
        assert!(cosine_similarity(&a, &b).abs() < 1e-6);
    }

    #[test]
    fn cosine_similarity_handles_zero_and_mismatch() {
        assert_eq!(cosine_similarity(&[], &[]), 0.0);
        assert_eq!(cosine_similarity(&[1.0, 2.0], &[1.0]), 0.0);
        assert_eq!(cosine_similarity(&[0.0, 0.0], &[1.0, 1.0]), 0.0);
    }

    fn fail(criterion: &str, reason: &str) -> Finding {
        Finding {
            criterion: criterion.into(),
            status: FindingStatus::Fail,
            reason: reason.into(),
        }
    }

    #[test]
    fn finding_pass_or_warn_returns_none() {
        let pass = Finding {
            criterion: "subject is correct".into(),
            status: FindingStatus::Pass,
            reason: "matches".into(),
        };
        assert!(finding_to_kontext_instruction(&pass).is_none());
        let warn = Finding {
            criterion: "subject is correct".into(),
            status: FindingStatus::Warn,
            reason: "uncertain".into(),
        };
        assert!(finding_to_kontext_instruction(&warn).is_none());
    }

    #[test]
    fn finding_misspelling_becomes_replace_instruction() {
        let f = fail("logo reads PORSCHE", "PORZCHE instead of PORSCHE");
        let out = finding_to_kontext_instruction(&f).unwrap();
        assert!(out.contains("replace 'PORZCHE' with 'PORSCHE'"), "got: {out}");
        assert!(out.ends_with(", leave everything else unchanged"));
    }

    #[test]
    fn finding_misspelling_with_quotes_strips_them() {
        let f = fail("badge text correct", "'BUY NWO' instead of 'BUY NOW'");
        let out = finding_to_kontext_instruction(&f).unwrap();
        assert!(out.contains("replace 'BUY NWO' with 'BUY NOW'"), "got: {out}");
    }

    #[test]
    fn finding_banned_element_visible_becomes_removal() {
        let f = fail("no license plate", "license plate visible on the rear bumper");
        let out = finding_to_kontext_instruction(&f).unwrap();
        assert!(out.contains("remove the license plate"), "got: {out}");
        assert!(out.ends_with(", leave everything else unchanged"));
    }

    #[test]
    fn finding_baked_in_watermark_becomes_removal() {
        let f = fail("no baked-in text or watermarks", "baked-in watermark in lower right");
        let out = finding_to_kontext_instruction(&f).unwrap();
        assert!(out.contains("remove the"), "got: {out}");
        assert!(out.contains("baked-in text or watermarks") || out.contains("watermark"));
    }

    #[test]
    fn finding_bystanders_present_becomes_removal() {
        let f = fail("no bystanders visible", "two bystanders present in background");
        let out = finding_to_kontext_instruction(&f).unwrap();
        assert!(out.contains("remove the bystanders visible"), "got: {out}");
    }

    #[test]
    fn finding_freeform_reason_falls_back_to_repair_clause() {
        let f = fail(
            "subject is a green Porsche 911 GT3",
            "color appears teal rather than the requested green",
        );
        let out = finding_to_kontext_instruction(&f).unwrap();
        assert!(out.contains("fix this") || out.contains("edit the image"), "got: {out}");
        assert!(out.ends_with(", leave everything else unchanged"));
    }

    #[test]
    fn finding_empty_reason_uses_criterion() {
        let f = fail("subject is a green car", "");
        let out = finding_to_kontext_instruction(&f).unwrap();
        assert!(out.contains("subject is a green car"));
    }

    #[test]
    fn region_hint_upper_left_corner() {
        let h = region_to_instruction_hint([10, 10, 50, 50], 900, 600);
        assert_eq!(h, "in the upper-left region of the image");
    }

    #[test]
    fn region_hint_lower_right_corner() {
        let h = region_to_instruction_hint([700, 500, 100, 100], 900, 600);
        assert_eq!(h, "in the lower-right region of the image");
    }

    #[test]
    fn region_hint_dead_center_is_special_cased() {
        let h = region_to_instruction_hint([400, 250, 100, 100], 900, 600);
        assert_eq!(h, "in the center of the image");
    }

    #[test]
    fn instruction_edit_request_round_trips() {
        let req = InstructionEditRequest::new(
            "https://x/y.png",
            "replace red can with blue can, leave everything else unchanged",
        );
        let json = serde_json::to_string(&req).unwrap();
        let back: InstructionEditRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(back.source_image_url, "https://x/y.png");
        assert!(back.instruction.starts_with("replace red can"));
        assert!(back.mask_url.is_none());
    }

    #[test]
    fn face_detect_request_round_trips() {
        let req = FaceDetectRequest::new("https://x/y.png").with_min_confidence(0.7);
        let json = serde_json::to_string(&req).unwrap();
        let back: FaceDetectRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(back.image_url, "https://x/y.png");
        assert!((back.min_confidence - 0.7).abs() < 1e-6);
    }

    #[test]
    fn face_detect_result_round_trips() {
        let r = FaceDetectResult {
            provider: "roboflow-face-detection-mik1i".into(),
            image_width: 400,
            image_height: 352,
            detections: vec![FaceDetection {
                bbox: [130, 8, 141, 212],
                confidence: 0.89,
            }],
        };
        let json = serde_json::to_string(&r).unwrap();
        let back: FaceDetectResult = serde_json::from_str(&json).unwrap();
        assert_eq!(back.detections.len(), 1);
        assert_eq!(back.detections[0].bbox, [130, 8, 141, 212]);
    }

    #[test]
    fn low_denoise_img2img_request_round_trips() {
        let req = LowDenoiseImg2ImgRequest::new(
            "https://x/y.png",
            "portrait of a person, detailed skin texture",
            0.2,
        );
        let json = serde_json::to_string(&req).unwrap();
        let back: LowDenoiseImg2ImgRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(back.image_url, "https://x/y.png");
        assert!((back.strength - 0.2).abs() < 1e-6);
        assert!(back.num_inference_steps.is_none());
    }
}

