//! Face Detect backend types.

#![allow(missing_docs)]

use crate::backends::{BackendCallOutcome, BackendError, CostEstimate, RunMode};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use crate::handlers::util::image_arg_to_url;

/// Face-detection request — locate every face in one image and return
/// its bounding box plus the model's confidence. Powers the
/// face-crop refine paste-back pipeline (HelloRob template — the
/// "make human faces stop looking plastic" step).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FaceDetectRequest {
    /// Image URL — `https://…` or a `data:` URL. Local-path callers
    /// route through `image_arg_to_url` first.
    pub image_url: String,
    /// Minimum confidence threshold in `[0.0, 1.0]`. Detections below
    /// this are dropped. The adapter forwards this to provider-side
    /// filtering when possible and re-checks locally so the contract
    /// is uniform.
    pub min_confidence: f32,
}

impl FaceDetectRequest {
    /// Build a request with a sensible default threshold (`0.5`).
    pub fn new(image_url: impl Into<String>) -> Self {
        Self {
            image_url: image_url.into(),
            min_confidence: 0.5,
        }
    }

    /// Builder-style — override the minimum confidence.
    pub fn with_min_confidence(mut self, conf: f32) -> Self {
        self.min_confidence = conf;
        self
    }
}

/// One face detection — bbox in original-image pixel coordinates plus
/// the provider's confidence score.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct FaceDetection {
    /// `[x, y, w, h]` in pixels, top-left origin. Always clamped into
    /// image bounds by the adapter.
    pub bbox: [u32; 4],
    /// Confidence in `[0.0, 1.0]`. Higher is more confident.
    pub confidence: f32,
}

/// Face-detection call result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FaceDetectResult {
    /// Provider id (`roboflow-face-detection-mik1i`).
    pub provider: String,
    /// Source image dimensions in pixels — adapters echo these back so
    /// downstream crop math doesn't have to re-open the original to
    /// check bounds.
    pub image_width: u32,
    /// Source image height.
    pub image_height: u32,
    /// Detections sorted by descending confidence. May be empty when
    /// no faces clear `min_confidence`.
    pub detections: Vec<FaceDetection>,
}

/// Cluster trait shared by every face-detection adapter.
pub trait FaceDetectBackend {
    /// Provider name.
    fn name(&self) -> &'static str;
    /// Cost estimate.
    fn estimate_cost(&self, request: &FaceDetectRequest) -> CostEstimate;
    /// Run face detection. Returns bboxes + confidences.
    fn detect_faces(
        &self,
        request: &FaceDetectRequest,
        mode: RunMode,
    ) -> Result<BackendCallOutcome<FaceDetectResult>, BackendError>;
}

