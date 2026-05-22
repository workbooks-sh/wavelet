//! Segment By Text backend types.

#![allow(missing_docs)]

use crate::backends::{BackendCallOutcome, BackendError, CostEstimate, RunMode};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use crate::handlers::util::image_arg_to_url;
use super::bg_remove::ImageResult;

/// Text-prompted segmentation request — like bg-remove, but the prompt
/// names what to keep ("the car", "the person on the left").
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SegmentByTextRequest {
    /// Source image URL or local path.
    pub image: String,
    /// Text prompt naming the subject to keep.
    pub prompt: String,
}

impl SegmentByTextRequest {
    /// Build a minimum-viable request.
    pub fn new(image: impl Into<String>, prompt: impl Into<String>) -> Self {
        Self {
            image: image.into(),
            prompt: prompt.into(),
        }
    }
}

/// Cluster trait shared by every text-prompted segmentation adapter.
pub trait SegmentByTextBackend {
    /// Provider name.
    fn name(&self) -> &'static str;
    /// Cost estimate.
    fn estimate_cost(&self, request: &SegmentByTextRequest) -> CostEstimate;
    /// Segment by text. Returns a cached RGBA PNG with alpha=mask.
    fn segment(
        &self,
        request: &SegmentByTextRequest,
        mode: RunMode,
    ) -> Result<BackendCallOutcome<ImageResult>, BackendError>;
}

