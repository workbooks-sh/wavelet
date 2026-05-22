//! Vision Verify backend types.

#![allow(missing_docs)]

use crate::backends::{BackendCallOutcome, BackendError, CostEstimate, RunMode};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use crate::handlers::util::image_arg_to_url;

/// One pre-render verification call — a still + a list of yes/no
/// criteria the VLM should grade. Used to catch identity drift,
/// bystanders, baked-in watermarks before a paid render/mux step.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VisionVerifyRequest {
    /// Image URL or `data:` URL. The adapter does not fetch/upload —
    /// callers route local paths through `image_arg_to_url` first.
    pub image_url: String,
    /// Natural-language criteria, each phrased as a positive claim
    /// (e.g. `"the subject is a green Porsche 911 GT3"`,
    /// `"no bystanders visible"`, `"no baked-in text or watermarks"`).
    pub criteria: Vec<String>,
}

impl VisionVerifyRequest {
    /// Build from an image URL and a list of criteria.
    pub fn new(image_url: impl Into<String>, criteria: Vec<String>) -> Self {
        Self {
            image_url: image_url.into(),
            criteria,
        }
    }
}

/// Per-criterion verdict.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FindingStatus {
    /// Criterion clearly met.
    Pass,
    /// Criterion partially met or model uncertain. Worth a human look.
    Warn,
    /// Criterion clearly violated. Block the render.
    Fail,
}

/// One graded criterion.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Finding {
    /// The exact criterion text the model graded.
    pub criterion: String,
    /// Verdict.
    pub status: FindingStatus,
    /// Short rationale the model returned.
    pub reason: String,
}

/// Verification call result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VisionVerifyResult {
    /// Provider identifier (`fal-vision-verify`).
    pub provider: String,
    /// One `Finding` per criterion, in input order. If the parser
    /// couldn't recover a verdict for a criterion, that finding is
    /// emitted with `FindingStatus::Warn` and the raw model line as
    /// `reason` — never silently dropped.
    pub findings: Vec<Finding>,
    /// `false` if any finding is `Fail`. `Warn` does not flip this —
    /// callers decide whether to gate on warnings.
    pub overall_pass: bool,
}

/// Cluster trait shared by every vision-verify adapter.
pub trait VisionVerifyBackend {
    /// Provider name.
    fn name(&self) -> &'static str;
    /// Cost estimate.
    fn estimate_cost(&self, request: &VisionVerifyRequest) -> CostEstimate;
    /// Run the verification. Returns one `Finding` per input criterion.
    fn verify(
        &self,
        request: &VisionVerifyRequest,
        mode: RunMode,
    ) -> Result<BackendCallOutcome<VisionVerifyResult>, BackendError>;
}

