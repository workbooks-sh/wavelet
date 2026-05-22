//! Low Denoise backend types.

#![allow(missing_docs)]

use crate::backends::{BackendCallOutcome, BackendError, CostEstimate, RunMode};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use crate::handlers::util::image_arg_to_url;
use super::bg_remove::ImageResult;

/// Low-denoise img2img request — runs an existing image back through a
/// diffusion model at low `strength` (typically 0.15-0.25) so the
/// identity is preserved but skin micro-detail, hair edges, and eye
/// catchlights are restored. The inner step of the face-refine
/// paste-back pipeline.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LowDenoiseImg2ImgRequest {
    /// Source image URL or `data:` URL.
    pub image_url: String,
    /// Generation prompt — describe what the input contains, not what
    /// you want it to change into. For face-refine this is typically
    /// `"portrait of a person, detailed skin texture, natural lighting"`.
    pub prompt: String,
    /// Denoise fraction in `[0.0, 1.0]`. The whole point of this trait
    /// is that the value here is *low* — `0.2` keeps identity intact,
    /// `0.5+` starts re-rolling the face entirely.
    pub strength: f32,
    /// Optional inference-step count. Provider clamps to its supported
    /// range; `None` lets the provider choose.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub num_inference_steps: Option<u32>,
    /// Random seed for reproducibility.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seed: Option<u64>,
}

impl LowDenoiseImg2ImgRequest {
    /// Build a minimum-viable request.
    pub fn new(
        image_url: impl Into<String>,
        prompt: impl Into<String>,
        strength: f32,
    ) -> Self {
        Self {
            image_url: image_url.into(),
            prompt: prompt.into(),
            strength,
            num_inference_steps: None,
            seed: None,
        }
    }
}

/// Cluster trait shared by every low-denoise img2img adapter.
pub trait LowDenoiseImg2ImgBackend {
    /// Provider name.
    fn name(&self) -> &'static str;
    /// Cost estimate.
    fn estimate_cost(&self, request: &LowDenoiseImg2ImgRequest) -> CostEstimate;
    /// Run the refine pass. Returns the refined image on disk.
    fn refine(
        &self,
        request: &LowDenoiseImg2ImgRequest,
        mode: RunMode,
    ) -> Result<BackendCallOutcome<ImageResult>, BackendError>;
}

