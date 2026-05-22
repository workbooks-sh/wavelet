//! Upscale backend types.

#![allow(missing_docs)]

use crate::backends::{BackendCallOutcome, BackendError, CostEstimate, RunMode};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use crate::handlers::util::image_arg_to_url;
use super::identity_check::IdentityCheckResult;

/// Final-pass upscale request — works for both single stills and video
/// clips. The adapter routes based on which sub-trait it implements
/// (image-only adapters reject video URLs and vice-versa); the CLI's
/// `auto` mode picks the adapter by input extension.
///
/// Exactly one of `target_scale` or `target_resolution` must take effect
/// at the adapter — when both are set, `target_resolution` wins (it's
/// the more specific knob). When neither is meaningful (`target_scale`
/// at 1.0 and `target_resolution` `None`), the adapter falls back to
/// its provider-default scale (typically 2×).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct UpscaleRequest {
    /// Source asset URL — image (`png`/`jpg`/`webp`) or video
    /// (`mp4`/`mov`/`webm`). Adapter validates the input shape.
    pub source_url: String,
    /// Multiplicative scale factor (`2.0`, `4.0`). Ignored when
    /// `target_resolution` is `Some`.
    pub target_scale: f32,
    /// Optional explicit `(width, height)` target. Overrides
    /// `target_scale` when set. Adapters that only accept a scale
    /// factor map this to the nearest supported scale.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_resolution: Option<(u32, u32)>,
}

impl UpscaleRequest {
    /// Build a minimum-viable request — defaults to 2× scale.
    pub fn new(source_url: impl Into<String>) -> Self {
        Self {
            source_url: source_url.into(),
            target_scale: 2.0,
            target_resolution: None,
        }
    }

    /// Builder-style — set the scale factor.
    pub fn with_scale(mut self, scale: f32) -> Self {
        self.target_scale = scale;
        self
    }

    /// Builder-style — pin an explicit `(width, height)` target.
    pub fn with_resolution(mut self, w: u32, h: u32) -> Self {
        self.target_resolution = Some((w, h));
        self
    }
}

/// Result of a final-pass upscale. Mirrors `ImageResult` / `VideoResult`
/// fields the consumer cares about — provider, cached path, dimensions
/// — without forcing the caller to discriminate between still and
/// clip results upfront.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct UpscaleResponse {
    /// Provider identifier (`fal-supir`, `fal-topaz-video-upscale`).
    pub provider: String,
    /// Cached output path on disk.
    pub output_path: PathBuf,
    /// Asset size in bytes.
    pub output_bytes: u64,
    /// Output pixel width.
    pub width: u32,
    /// Output pixel height.
    pub height: u32,
    /// Mime type of the output (`image/png`, `video/mp4`, …).
    pub mime: String,
    /// Convenience alias for `output_path.display().to_string()` — the
    /// public URL is not retained because cache assets live on disk
    /// behind the canonical pathway.
    pub url: String,
}

/// Cluster trait shared by every final-pass upscale adapter. One trait
/// covers both image and video adapters — the `UpscaleRequest` carries
/// a URL and adapters reject inputs of the wrong shape during validation.
pub trait UpscaleBackend {
    /// Provider name.
    fn name(&self) -> &'static str;
    /// Cost estimate for this specific request.
    fn estimate_cost(&self, request: &UpscaleRequest) -> CostEstimate;
    /// Run the upscale. Returns a cached asset.
    fn upscale(
        &self,
        request: &UpscaleRequest,
        mode: RunMode,
    ) -> Result<BackendCallOutcome<UpscaleResponse>, BackendError>;
}

/// Cosine similarity of two equal-length embedding vectors. Returns 0.0
/// when either vector has zero magnitude — keeps callers from having
/// to special-case the degenerate input.
pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    let mut dot = 0.0f32;
    let mut na = 0.0f32;
    let mut nb = 0.0f32;
    for (x, y) in a.iter().zip(b.iter()) {
        dot += x * y;
        na += x * x;
        nb += y * y;
    }
    if na <= f32::EPSILON || nb <= f32::EPSILON {
        return 0.0;
    }
    dot / (na.sqrt() * nb.sqrt())
}

