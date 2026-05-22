//! Fal Flux Pro Kontext Max adapter — `InstructionEdit` cluster.
//!
//! Wraps `fal-ai/flux-pro/kontext/max` — surgical, instruction-driven
//! image edits. When a generated shot is 90% right and one element is
//! wrong ("badge wrong color", "license plate present", "PORZCHE
//! misspelling"), Kontext rewrites just the offending element from a
//! natural-language instruction instead of regenerating the whole
//! frame.
//!
//! Wire schema (confirmed via live probe 2026-05-18 against fal.run):
//!   request:  prompt (str), image_url (str), guidance_scale (>=1, opt),
//!             num_inference_steps (<=50, opt), seed (u64, opt)
//!   response: { images: [{ url, content_type, width, height }], seed, ... }
//!
//! The endpoint does NOT take a `mask_url` — it's instruction-only.
//! Region constraints are expressed inside the instruction text itself
//! (e.g. "in the upper-right quadrant …"). The CLI's `--region` flag
//! translates a bbox into prompt language. We still expose `mask_url`
//! on the request struct as a transparent passthrough so a future
//! Kontext variant (or inpaint sibling) that does accept it doesn't
//! force a trait change.
//!
//! Cost: ~$0.04-0.08 per call (instruction-edit price band).

use crate::backends::cache::{utc_now_iso8601, AssetCache, Manifest};
use crate::backends::util::pick_image_ext_from_mime;
use crate::backends::fal::FalClient;
use crate::backends::image::{
    ImageResult, InstructionEditBackend, InstructionEditRequest, CLUSTER_INSTRUCTION_EDIT,
};
use crate::backends::{
    check_budget, mode_label, BackendCallOutcome, BackendError, CostEstimate, RunMode,
};
use serde::{Deserialize, Serialize};

/// Provider id.
pub const PROVIDER: &str = "fal-flux-kontext-max";

/// Fal model path.
pub const MODEL_PATH: &str = "fal-ai/flux-pro/kontext/max";

/// Per-call cost estimate (USD). Conservative — actual list is $0.04
/// on the cheap end; bumped to $0.08 to keep budget checks honest when
/// fal periodically reprices.
pub const PRICE_PER_CALL_USD: f32 = 0.08;

/// Fal endpoint clamp — `num_inference_steps` cannot exceed 50.
pub const MAX_INFERENCE_STEPS: u32 = 50;

/// Fal endpoint clamp — `guidance_scale` must be >= 1.
pub const MIN_GUIDANCE_SCALE: f32 = 1.0;

/// Fal Flux Kontext Max adapter.
#[derive(Debug, Clone)]
pub struct FalKontextMaxAdapter {
    client: FalClient,
}

impl FalKontextMaxAdapter {
    /// Build from a pre-constructed client.
    pub fn new(client: FalClient) -> Self {
        Self { client }
    }
}

impl InstructionEditBackend for FalKontextMaxAdapter {
    fn name(&self) -> &'static str {
        PROVIDER
    }

    fn estimate_cost(&self, _: &InstructionEditRequest) -> CostEstimate {
        CostEstimate {
            provider: PROVIDER.into(),
            cost_usd: PRICE_PER_CALL_USD,
            explanation: format!("${PRICE_PER_CALL_USD:.4}/call (conservative)"),
        }
    }

    fn instruction_edit(
        &self,
        request: &InstructionEditRequest,
        mode: RunMode,
    ) -> Result<BackendCallOutcome<ImageResult>, BackendError> {
        if request.instruction.trim().is_empty() {
            return Err(BackendError::InvalidRequest("instruction is empty".into()));
        }
        if request.source_image_url.trim().is_empty() {
            return Err(BackendError::InvalidRequest(
                "source_image_url is empty".into(),
            ));
        }
        if let Some(g) = request.guidance_scale {
            if !(g >= MIN_GUIDANCE_SCALE) {
                return Err(BackendError::InvalidRequest(format!(
                    "guidance_scale {g} below endpoint minimum {MIN_GUIDANCE_SCALE}"
                )));
            }
        }
        if let Some(steps) = request.num_inference_steps {
            if steps > MAX_INFERENCE_STEPS {
                return Err(BackendError::InvalidRequest(format!(
                    "num_inference_steps {steps} above endpoint maximum {MAX_INFERENCE_STEPS}"
                )));
            }
        }

        let estimate = self.estimate_cost(request);
        check_budget(&estimate, mode)?;

        let request_hash = AssetCache::request_hash(PROVIDER, CLUSTER_INSTRUCTION_EDIT, request)?;
        let cache = self.client.cache();

        if let Some(manifest) = cache.hit(PROVIDER, &request_hash)? {
            let response: ImageResult = serde_json::from_value(manifest.response.clone())
                .map_err(|e| BackendError::Cache(format!("decode cached response: {e}")))?;
            return Ok(BackendCallOutcome {
                response,
                provider: PROVIDER.into(),
                request_hash,
                cached: true,
                cost_estimate_usd: 0.0,
                mode: mode_label(mode),
            });
        }

        if !mode.is_live() {
            let response = ImageResult {
                provider: PROVIDER.into(),
                image_path: cache.asset_path(PROVIDER, &request_hash, "png"),
                image_bytes: 0,
                width: 0,
                height: 0,
                mime: "image/png".into(),
            };
            return Ok(BackendCallOutcome {
                response,
                provider: PROVIDER.into(),
                request_hash,
                cached: false,
                cost_estimate_usd: estimate.cost_usd,
                mode: mode_label(mode),
            });
        }

        let body = KontextBody {
            prompt: request.instruction.clone(),
            image_url: request.source_image_url.clone(),
            mask_url: request.mask_url.clone(),
            guidance_scale: request.guidance_scale,
            num_inference_steps: request.num_inference_steps,
            seed: request.seed,
        };
        let parsed: KontextResponse = self.client.post_sync(MODEL_PATH, &body)?;
        let img = parsed
            .images
            .into_iter()
            .next()
            .ok_or_else(|| BackendError::Decode("kontext returned no images".into()))?;
        let image_bytes_raw = self.client.fetch_asset(&img.url)?;
        let ext = pick_image_ext_from_mime(img.content_type.as_deref());
        let image_path = cache.write_asset(PROVIDER, &request_hash, ext, &image_bytes_raw)?;
        let image_bytes = image_bytes_raw.len() as u64;

        let result = ImageResult {
            provider: PROVIDER.into(),
            image_path: image_path.clone(),
            image_bytes,
            width: img.width.unwrap_or(0),
            height: img.height.unwrap_or(0),
            mime: img.content_type.unwrap_or_else(|| "image/png".into()),
        };

        let manifest = Manifest {
            version: 1,
            provider: PROVIDER.into(),
            cluster: CLUSTER_INSTRUCTION_EDIT.into(),
            request_hash: request_hash.clone(),
            request: serde_json::to_value(request).map_err(|e| {
                BackendError::Cache(format!("serialize request for cache: {e}"))
            })?,
            response: serde_json::to_value(&result).map_err(|e| {
                BackendError::Cache(format!("serialize response for cache: {e}"))
            })?,
            cost_estimate_usd: estimate.cost_usd,
            asset_path: Some(image_path.display().to_string()),
            created_at: utc_now_iso8601(),
        };
        cache.store(&manifest)?;

        Ok(BackendCallOutcome {
            response: result,
            provider: PROVIDER.into(),
            request_hash,
            cached: false,
            cost_estimate_usd: estimate.cost_usd,
            mode: mode_label(mode),
        })
    }
}

#[derive(Debug, Serialize)]
struct KontextBody {
    prompt: String,
    image_url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    mask_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    guidance_scale: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    num_inference_steps: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    seed: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct KontextResponse {
    images: Vec<KontextImage>,
}

#[derive(Debug, Deserialize)]
struct KontextImage {
    url: String,
    #[serde(default)]
    content_type: Option<String>,
    #[serde(default)]
    width: Option<u32>,
    #[serde(default)]
    height: Option<u32>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fresh_cache() -> std::path::PathBuf {
        let tmp = std::env::temp_dir().join(format!(
            "wavelet-fal-kontext-{}",
            AssetCache::request_hash("kontext", "kontext", &"x").unwrap()
        ));
        let _ = std::fs::remove_dir_all(&tmp);
        tmp
    }

    fn sample_req() -> InstructionEditRequest {
        InstructionEditRequest::new(
            "https://example.com/shot.jpg",
            "replace the red can with a blue can, leave everything else unchanged",
        )
    }

    #[test]
    fn body_shape_with_mask_serializes_all_fields() {
        let body = KontextBody {
            prompt: "x".into(),
            image_url: "https://a/y.png".into(),
            mask_url: Some("https://a/m.png".into()),
            guidance_scale: Some(3.5),
            num_inference_steps: Some(28),
            seed: Some(42),
        };
        let v = serde_json::to_value(&body).unwrap();
        assert_eq!(v["prompt"], "x");
        assert_eq!(v["image_url"], "https://a/y.png");
        assert_eq!(v["mask_url"], "https://a/m.png");
        assert_eq!(v["guidance_scale"], 3.5);
        assert_eq!(v["num_inference_steps"], 28);
        assert_eq!(v["seed"], 42);
    }

    #[test]
    fn body_without_mask_omits_optional_fields() {
        let body = KontextBody {
            prompt: "x".into(),
            image_url: "https://a/y.png".into(),
            mask_url: None,
            guidance_scale: None,
            num_inference_steps: None,
            seed: None,
        };
        let s = serde_json::to_string(&body).unwrap();
        assert!(s.contains("\"prompt\":\"x\""));
        assert!(s.contains("\"image_url\":\"https://a/y.png\""));
        assert!(!s.contains("mask_url"), "leaked mask_url: {s}");
        assert!(!s.contains("guidance_scale"), "leaked guidance_scale: {s}");
        assert!(!s.contains("num_inference_steps"), "leaked steps: {s}");
        assert!(!s.contains("seed"), "leaked seed: {s}");
    }

    #[test]
    fn response_decodes_kontext_wire_shape() {
        // Confirmed against fal.run on 2026-05-18.
        let body = r#"{
            "images": [{
                "url": "https://v3b.fal.media/files/b/x.jpg",
                "content_type": "image/jpeg",
                "file_name": null,
                "file_size": null,
                "width": 1024,
                "height": 1024
            }],
            "timings": {},
            "seed": 47804198,
            "has_nsfw_concepts": [false],
            "prompt": "edit"
        }"#;
        let parsed: KontextResponse = serde_json::from_str(body).unwrap();
        assert_eq!(parsed.images.len(), 1);
        assert_eq!(parsed.images[0].width, Some(1024));
        assert_eq!(
            parsed.images[0].content_type.as_deref(),
            Some("image/jpeg")
        );
    }

    #[test]
    fn empty_instruction_rejected() {
        let client = FalClient::with_key("id:secret", fresh_cache());
        let adapter = FalKontextMaxAdapter::new(client);
        let req = InstructionEditRequest::new("https://x/y.png", "   ");
        assert!(matches!(
            adapter.instruction_edit(&req, RunMode::DryRun).unwrap_err(),
            BackendError::InvalidRequest(_)
        ));
    }

    #[test]
    fn empty_source_url_rejected() {
        let client = FalClient::with_key("id:secret", fresh_cache());
        let adapter = FalKontextMaxAdapter::new(client);
        let req = InstructionEditRequest::new("", "fix the watermark");
        assert!(matches!(
            adapter.instruction_edit(&req, RunMode::DryRun).unwrap_err(),
            BackendError::InvalidRequest(_)
        ));
    }

    #[test]
    fn guidance_below_minimum_rejected() {
        let client = FalClient::with_key("id:secret", fresh_cache());
        let adapter = FalKontextMaxAdapter::new(client);
        let mut req = sample_req();
        req.guidance_scale = Some(0.5);
        assert!(matches!(
            adapter.instruction_edit(&req, RunMode::DryRun).unwrap_err(),
            BackendError::InvalidRequest(_)
        ));
    }

    #[test]
    fn steps_above_maximum_rejected() {
        let client = FalClient::with_key("id:secret", fresh_cache());
        let adapter = FalKontextMaxAdapter::new(client);
        let mut req = sample_req();
        req.num_inference_steps = Some(99);
        assert!(matches!(
            adapter.instruction_edit(&req, RunMode::DryRun).unwrap_err(),
            BackendError::InvalidRequest(_)
        ));
    }

    #[test]
    fn dry_run_no_write_emits_request_shape() {
        let cache_dir = fresh_cache();
        let client = FalClient::with_key("id:secret", &cache_dir);
        let adapter = FalKontextMaxAdapter::new(client);
        let out = adapter
            .instruction_edit(&sample_req(), RunMode::DryRun)
            .unwrap();
        assert_eq!(out.mode, "dry-run");
        assert_eq!(out.response.provider, PROVIDER);
        assert_eq!(out.response.image_bytes, 0);
        assert!(!out.response.image_path.exists(), "dry-run wrote an asset");
    }

    #[test]
    fn cost_estimate_is_conservative_per_call() {
        let client = FalClient::with_key("id:secret", fresh_cache());
        let adapter = FalKontextMaxAdapter::new(client);
        let est = adapter.estimate_cost(&sample_req());
        assert_eq!(est.provider, PROVIDER);
        assert!((est.cost_usd - PRICE_PER_CALL_USD).abs() < f32::EPSILON);
    }

}
