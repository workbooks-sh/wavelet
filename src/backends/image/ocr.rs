//! Ocr backend types.

#![allow(missing_docs)]

use crate::backends::{BackendCallOutcome, BackendError, CostEstimate, RunMode};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use crate::handlers::util::image_arg_to_url;

/// OCR request — extract recognized text from one image. Used as a
/// pre-overlay guard: if the generated still already has baked-in text
/// (signage, license plates, watermarks), the typography pass should
/// route HTML overlays away from that region.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OcrRequest {
    /// Image URL — `https://…` or a `data:` URL. The adapter does not
    /// fetch/upload — callers route local paths through `image_arg_to_url`.
    pub image_url: String,
}

impl OcrRequest {
    /// Build a minimum-viable request.
    pub fn new(image_url: impl Into<String>) -> Self {
        Self {
            image_url: image_url.into(),
        }
    }
}

/// One recognized text block.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OcrDetection {
    /// Recognized text content.
    pub text: String,
    /// `[x, y, w, h]` in image pixel coords, when the backend returns
    /// bboxes. `None` for text-only providers (Roboflow doctr).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bbox: Option<[u32; 4]>,
    /// Provider-reported confidence in `[0.0, 1.0]`. `None` for
    /// providers that return a single combined string.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confidence: Option<f32>,
}

/// OCR call result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OcrResult {
    /// Provider identifier (e.g. `roboflow-doctr`).
    pub provider: String,
    /// One entry per recognized text block, in reading order.
    pub detections: Vec<OcrDetection>,
    /// Every detection's `text` joined with `\n` — convenience for
    /// callers that only want a flat blob.
    pub combined_text: String,
}

/// Cluster trait shared by every OCR adapter.
pub trait OcrBackend {
    /// Provider name.
    fn name(&self) -> &'static str;
    /// Cost estimate.
    fn estimate_cost(&self, request: &OcrRequest) -> CostEstimate;
    /// Recognize text in the image.
    fn recognize(
        &self,
        request: &OcrRequest,
        mode: RunMode,
    ) -> Result<BackendCallOutcome<OcrResult>, BackendError>;
}

