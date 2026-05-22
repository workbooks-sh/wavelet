//! Bg Remove backend types.

#![allow(missing_docs)]

use crate::backends::{BackendCallOutcome, BackendError, CostEstimate, RunMode};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use crate::handlers::util::image_arg_to_url;
use super::vision_verify::Finding;

/// One background-removal request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BgRemoveRequest {
    /// Source image URL or local path. URL-accepting providers
    /// (birefnet) pass directly; local-path providers upload first.
    pub image: String,
    /// Optional output format hint. Most providers return PNG (alpha).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_format: Option<String>,
}

impl BgRemoveRequest {
    /// Build a minimum-viable request.
    pub fn new(image: impl Into<String>) -> Self {
        Self {
            image: image.into(),
            output_format: None,
        }
    }
}

/// Result of an image-processing call.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageResult {
    /// Provider identifier (`fal-birefnet`).
    pub provider: String,
    /// Cached image path on disk.
    pub image_path: PathBuf,
    /// File size in bytes.
    pub image_bytes: u64,
    /// Pixel dimensions.
    pub width: u32,
    /// Pixel height.
    pub height: u32,
    /// Mime type.
    pub mime: String,
}

/// Cluster trait shared by every bg-removal adapter.
pub trait BgRemoveBackend {
    /// Provider name (`"fal-birefnet"`).
    fn name(&self) -> &'static str;

    /// Estimate the cost.
    fn estimate_cost(&self, request: &BgRemoveRequest) -> CostEstimate;

    /// Remove the background. Returns the cached PNG path.
    fn remove_bg(
        &self,
        request: &BgRemoveRequest,
        mode: RunMode,
    ) -> Result<BackendCallOutcome<ImageResult>, BackendError>;
}

