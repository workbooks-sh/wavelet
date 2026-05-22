//! Ref Conditioned backend types.

#![allow(missing_docs)]

use crate::backends::{BackendCallOutcome, BackendError, CostEstimate, RunMode};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use crate::handlers::util::image_arg_to_url;
use super::txt2img::default_image_size;
use super::bg_remove::ImageResult;

/// Reference-conditioned image-gen request — generate a scene-aware
/// still conditioned on 1-10 reference images of the same subject.
/// Replaces the cutout-composite step: the model places the product
/// into the lighting, angle, and perspective of the prompted scene
/// rather than pasting an isolated cutout.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RefConditionedImgRequest {
    /// Scene prompt — describes the environment, lighting, framing.
    pub prompt: String,
    /// 1-10 public HTTPS URLs of reference images. The provider needs
    /// fetchable URLs; local-path UX is wired separately (wb-m9qe).
    pub image_urls: Vec<String>,
    /// Image-size hint (`landscape_16_9`, `square_hd`, `portrait_4_3`).
    #[serde(default = "default_image_size")]
    pub image_size: String,
    /// Random seed for reproducibility.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seed: Option<u64>,
}

impl RefConditionedImgRequest {
    /// Build a minimum-viable request from a prompt + reference URLs.
    pub fn new(prompt: impl Into<String>, image_urls: Vec<String>) -> Self {
        Self {
            prompt: prompt.into(),
            image_urls,
            image_size: default_image_size(),
            seed: None,
        }
    }
}

/// Cluster trait shared by every reference-conditioned img-gen adapter.
pub trait RefConditionedImgGenBackend {
    /// Provider name.
    fn name(&self) -> &'static str;
    /// Cost estimate.
    fn estimate_cost(&self, request: &RefConditionedImgRequest) -> CostEstimate;
    /// Generate the still. Returns a cached image (PNG/JPG) on disk.
    fn generate(
        &self,
        request: &RefConditionedImgRequest,
        mode: RunMode,
    ) -> Result<BackendCallOutcome<ImageResult>, BackendError>;
}

