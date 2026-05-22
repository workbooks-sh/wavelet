//! Instruction Edit backend types.

#![allow(missing_docs)]

use crate::backends::{BackendCallOutcome, BackendError, CostEstimate, RunMode};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use crate::handlers::util::image_arg_to_url;
use super::bg_remove::ImageResult;
use super::vision_verify::{Finding, FindingStatus};

/// Instruction-edit request — surgical edit of an existing image by
/// natural-language instruction. The whole point is to preserve every
/// pixel the model isn't asked to touch. Mask is optional and currently
/// ignored by `fal-ai/flux-pro/kontext/max` (instruction-only); kept on
/// the request shape so a future masked variant can use it without
/// breaking callers.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstructionEditRequest {
    /// Source image — `https://…` or a `data:` URL. Callers route local
    /// paths through `image_arg_to_url` first.
    pub source_image_url: String,
    /// Plain-language edit description, e.g.
    /// `"replace the red can with a blue can, leave everything else unchanged"`.
    pub instruction: String,
    /// Optional region-constraint mask (white inside the editable area,
    /// black outside). Passed through to providers that accept it;
    /// Kontext Max currently ignores it. Callers that need region
    /// precision should bake the region into `instruction` text via
    /// [`region_to_instruction_hint`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mask_url: Option<String>,
    /// Optional guidance scale (>= 1.0 for Kontext Max).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guidance_scale: Option<f32>,
    /// Optional inference-step count (<= 50 for Kontext Max).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub num_inference_steps: Option<u32>,
    /// Random seed for reproducibility.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seed: Option<u64>,
}

impl InstructionEditRequest {
    /// Build a minimum-viable request.
    pub fn new(source_image_url: impl Into<String>, instruction: impl Into<String>) -> Self {
        Self {
            source_image_url: source_image_url.into(),
            instruction: instruction.into(),
            mask_url: None,
            guidance_scale: None,
            num_inference_steps: None,
            seed: None,
        }
    }
}

/// Cluster trait shared by every instruction-edit adapter.
pub trait InstructionEditBackend {
    /// Provider name.
    fn name(&self) -> &'static str;
    /// Cost estimate.
    fn estimate_cost(&self, request: &InstructionEditRequest) -> CostEstimate;
    /// Run the surgical edit. Returns the cached image on disk.
    fn instruction_edit(
        &self,
        request: &InstructionEditRequest,
        mode: RunMode,
    ) -> Result<BackendCallOutcome<ImageResult>, BackendError>;
}

/// Convert a `Finding` whose status is `Fail` into a Kontext-shaped edit
/// instruction. Returns `None` for `Pass` / `Warn` findings — callers
/// gate on `Fail` only.
///
/// The conversion picks the wording from the `reason` field when it
/// names specific offending tokens (a misspelling like `"PORZCHE
/// instead of PORSCHE"`, a banned element like `"license plate
/// visible"`), and otherwise falls back to repairing the criterion
/// directly. Every output ends with the literal trailing clause
/// `", leave everything else unchanged"` — that's the empirically
/// load-bearing phrase that keeps Kontext from re-rolling the frame.
pub fn finding_to_kontext_instruction(finding: &Finding) -> Option<String> {
    if finding.status != FindingStatus::Fail {
        return None;
    }
    let reason = finding.reason.trim();
    let criterion = finding.criterion.trim();
    let body = if let Some(edit) = reason_to_replacement(reason) {
        edit
    } else if let Some(removal) = reason_to_removal(reason, criterion) {
        removal
    } else if !reason.is_empty() {
        format!("fix this: {reason}. Ensure the result satisfies: {criterion}")
    } else {
        format!("edit the image so that: {criterion}")
    };
    Some(format!("{body}, leave everything else unchanged"))
}

/// Heuristic: `"X instead of Y"` → `"replace 'X' with 'Y'"`. Returns
/// `None` when the reason doesn't contain the pivot phrase.
fn reason_to_replacement(reason: &str) -> Option<String> {
    let lowered = reason.to_lowercase();
    let pivot = " instead of ";
    let idx = lowered.find(pivot)?;
    let wrong = reason[..idx].trim().trim_matches('\'').trim_matches('"');
    let right_start = idx + pivot.len();
    let right_raw = reason[right_start..]
        .trim_end_matches(|c: char| c == '.' || c == '!' || c == ';')
        .trim();
    let right = right_raw.trim_matches('\'').trim_matches('"');
    if wrong.is_empty() || right.is_empty() {
        return None;
    }
    Some(format!("replace '{wrong}' with '{right}'"))
}

/// Heuristic: reasons like `"license plate visible"` /
/// `"baked-in watermark"` / `"bystanders present"` → removal
/// instruction. The removal path only fires when the criterion is
/// itself phrased negatively (`"no X"`) — otherwise positive criteria
/// like `"subject is a green car"` would get garbled into "remove the
/// color" when the reason contains an incidental "appears".
fn reason_to_removal(reason: &str, criterion: &str) -> Option<String> {
    let lowered_reason = reason.to_lowercase();
    let lowered_criterion = criterion.to_lowercase();
    const REMOVAL_CUES: &[&str] = &[
        "visible",
        "present",
        "shows",
        "showing",
        "contains",
        "baked-in",
        "baked in",
    ];
    let reason_has_cue = REMOVAL_CUES.iter().any(|c| lowered_reason.contains(c));
    let criterion_is_negative = lowered_criterion.starts_with("no ");
    if !reason_has_cue && !criterion_is_negative {
        return None;
    }
    // Prefer the criterion's noun phrase when negative; otherwise lift
    // the head of the reason (everything before the cue word).
    let noun = if let Some(rest) = lowered_criterion.strip_prefix("no ") {
        rest.trim().to_string()
    } else {
        let cue_idx = REMOVAL_CUES
            .iter()
            .filter_map(|c| lowered_reason.find(c))
            .min()
            .unwrap_or(lowered_reason.len());
        let head = lowered_reason[..cue_idx].trim();
        if head.is_empty() {
            return None;
        }
        head.to_string()
    };
    if noun.is_empty() {
        return None;
    }
    Some(format!("remove the {noun}"))
}

/// Translate a pixel-space bbox into a coarse instruction hint
/// ("in the upper-right region", "in the bottom-center quadrant").
/// Used by the CLI `--region` flag to add a location qualifier to an
/// instruction. `image_w` and `image_h` are the source dimensions.
///
/// The output is in spatial-third terminology (upper / middle / lower
/// crossed with left / center / right) — coarser than pixels, more
/// stable in Kontext's prompt vocabulary.
pub fn region_to_instruction_hint(
    bbox: [u32; 4],
    image_w: u32,
    image_h: u32,
) -> String {
    let [x, y, w, h] = bbox;
    let cx = x.saturating_add(w / 2);
    let cy = y.saturating_add(h / 2);
    let third_w = image_w.max(1) / 3;
    let third_h = image_h.max(1) / 3;
    let vert = if cy < third_h {
        "upper"
    } else if cy < third_h * 2 {
        "middle"
    } else {
        "lower"
    };
    let horiz = if cx < third_w {
        "left"
    } else if cx < third_w * 2 {
        "center"
    } else {
        "right"
    };
    if vert == "middle" && horiz == "center" {
        "in the center of the image".to_string()
    } else {
        format!("in the {vert}-{horiz} region of the image")
    }
}

