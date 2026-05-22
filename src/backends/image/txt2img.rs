//! Txt2Img backend types.

#![allow(missing_docs)]

use crate::backends::{BackendCallOutcome, BackendError, CostEstimate, RunMode};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use crate::handlers::util::image_arg_to_url;
use super::bg_remove::ImageResult;

/// Text-to-image request — generate a still from a prompt. Used for
/// environment plates in Path B (the backdrop the isolated subject
/// gets composited over).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Txt2ImgRequest {
    /// Generation prompt.
    pub prompt: String,
    /// Image-size hint (`landscape_16_9`, `square_hd`, `portrait_4_3`).
    #[serde(default = "default_image_size")]
    pub image_size: String,
    /// Random seed for reproducibility.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seed: Option<u64>,
}

pub(crate) fn default_image_size() -> String {
    "landscape_16_9".into()
}

impl Txt2ImgRequest {
    /// Build a minimum-viable request.
    pub fn new(prompt: impl Into<String>) -> Self {
        Self {
            prompt: prompt.into(),
            image_size: default_image_size(),
            seed: None,
        }
    }
}

/// Cluster trait shared by every txt2img adapter.
pub trait Txt2ImgBackend {
    /// Provider name.
    fn name(&self) -> &'static str;
    /// Cost estimate.
    fn estimate_cost(&self, request: &Txt2ImgRequest) -> CostEstimate;
    /// Generate the still.
    fn generate(
        &self,
        request: &Txt2ImgRequest,
        mode: RunMode,
    ) -> Result<BackendCallOutcome<ImageResult>, BackendError>;
}

