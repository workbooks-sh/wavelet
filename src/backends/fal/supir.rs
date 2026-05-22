//! Fal SUPIR adapter — `UpscaleBackend` cluster (image side).
//!
//! Wraps `fal-ai/supir` — high-quality single-still upscaler that
//! restores micro-detail rather than just resampling. Used as the
//! final-pass polish on scene-stills (e.g. Seedream output) before
//! they feed into i2v, and as the still-side counterpart to Topaz's
//! video upscale.
//!
//! Per the ComfyUI workflow audit, a final upscale pass is the single
//! largest perceived-quality lever in the production pipeline (6/15
//! surveyed workflows include one). SUPIR is the image-shaped half of
//! that lever.
//!
//! Cost: ~$0.10 per call (conservative).

use crate::backends::cache::{utc_now_iso8601, AssetCache, Manifest};
use crate::backends::fal::FalClient;
use crate::backends::image::{
    UpscaleBackend, UpscaleRequest, UpscaleResponse, CLUSTER_UPSCALE_IMAGE,
};
use crate::backends::{
    check_budget, mode_label, BackendCallOutcome, BackendError, CostEstimate, RunMode,
};
use serde::{Deserialize, Serialize};

/// Provider id.
pub const PROVIDER: &str = "fal-supir";

/// Fal model path.
pub const MODEL_PATH: &str = "fal-ai/supir";

/// Per-call cost estimate (USD). SUPIR's billed-per-image floor is
/// substantially higher than birefnet — kept conservative to avoid
/// surprise spend on batched callers.
pub const PRICE_PER_CALL_USD: f32 = 0.10;

/// Image extensions SUPIR accepts. Anything else falls through to a
/// clear `InvalidRequest` rather than wasting an API roundtrip.
pub const SUPPORTED_EXTS: &[&str] = &["png", "jpg", "jpeg", "webp"];

/// Fal SUPIR adapter.
#[derive(Debug, Clone)]
pub struct FalSupirAdapter {
    client: FalClient,
}

impl FalSupirAdapter {
    /// Build from a pre-constructed client.
    pub fn new(client: FalClient) -> Self {
        Self { client }
    }
}

impl UpscaleBackend for FalSupirAdapter {
    fn name(&self) -> &'static str {
        PROVIDER
    }

    fn estimate_cost(&self, _: &UpscaleRequest) -> CostEstimate {
        CostEstimate {
            provider: PROVIDER.into(),
            cost_usd: PRICE_PER_CALL_USD,
            explanation: format!("${PRICE_PER_CALL_USD:.4}/image (conservative)"),
        }
    }

    fn upscale(
        &self,
        request: &UpscaleRequest,
        mode: RunMode,
    ) -> Result<BackendCallOutcome<UpscaleResponse>, BackendError> {
        validate_request(request)?;

        let estimate = self.estimate_cost(request);
        check_budget(&estimate, mode)?;

        let request_hash = AssetCache::request_hash(PROVIDER, CLUSTER_UPSCALE_IMAGE, request)?;
        let cache = self.client.cache();

        if let Some(manifest) = cache.hit(PROVIDER, &request_hash)? {
            let response: UpscaleResponse = serde_json::from_value(manifest.response.clone())
                .map_err(|e| BackendError::Cache(format!("decode cached response: {e}")))?;
            return Ok(BackendCallOutcome {
                response,
                provider: PROVIDER.into(),
                request_hash,
                cached: true,
                cost_estimate_usd: 0.0,
                mode: mode_label(mode),
            });
        }

        if !mode.is_live() {
            let path = cache.asset_path(PROVIDER, &request_hash, "png");
            let response = UpscaleResponse {
                provider: PROVIDER.into(),
                url: path.display().to_string(),
                output_path: path,
                output_bytes: 0,
                width: 0,
                height: 0,
                mime: "image/png".into(),
            };
            return Ok(BackendCallOutcome {
                response,
                provider: PROVIDER.into(),
                request_hash,
                cached: false,
                cost_estimate_usd: estimate.cost_usd,
                mode: mode_label(mode),
            });
        }

        let body = SupirBody {
            image_url: request.source_url.clone(),
            scale: effective_scale(request),
        };
        let parsed: SupirResponse = self.client.post_sync(MODEL_PATH, &body)?;
        let image_url = parsed.image.url.clone();
        let image_bytes_raw = self.client.fetch_asset(&image_url)?;
        let output_path = cache.write_asset(PROVIDER, &request_hash, "png", &image_bytes_raw)?;
        let output_bytes = image_bytes_raw.len() as u64;

        let result = UpscaleResponse {
            provider: PROVIDER.into(),
            url: output_path.display().to_string(),
            output_path: output_path.clone(),
            output_bytes,
            width: parsed.image.width.unwrap_or(0),
            height: parsed.image.height.unwrap_or(0),
            mime: parsed.image.content_type.unwrap_or_else(|| "image/png".into()),
        };

        let manifest = Manifest {
            version: 1,
            provider: PROVIDER.into(),
            cluster: CLUSTER_UPSCALE_IMAGE.into(),
            request_hash: request_hash.clone(),
            request: serde_json::to_value(request).map_err(|e| {
                BackendError::Cache(format!("serialize request for cache: {e}"))
            })?,
            response: serde_json::to_value(&result).map_err(|e| {
                BackendError::Cache(format!("serialize response for cache: {e}"))
            })?,
            cost_estimate_usd: estimate.cost_usd,
            asset_path: Some(output_path.display().to_string()),
            created_at: utc_now_iso8601(),
        };
        cache.store(&manifest)?;

        Ok(BackendCallOutcome {
            response: result,
            provider: PROVIDER.into(),
            request_hash,
            cached: false,
            cost_estimate_usd: estimate.cost_usd,
            mode: mode_label(mode),
        })
    }
}

/// SUPIR caps at 4× — anything beyond that goes through Topaz or a
/// multi-pass run. Clamping here keeps the API call from getting
/// 422'd at the boundary.
const MAX_SCALE: f32 = 4.0;

fn validate_request(request: &UpscaleRequest) -> Result<(), BackendError> {
    if request.source_url.trim().is_empty() {
        return Err(BackendError::InvalidRequest("source_url is empty".into()));
    }
    if !looks_like_image(&request.source_url) {
        return Err(BackendError::InvalidRequest(format!(
            "supir is image-only; got url with non-image extension: {}",
            request.source_url
        )));
    }
    if request.target_scale <= 1.0 && request.target_resolution.is_none() {
        return Err(BackendError::InvalidRequest(
            "target_scale must be > 1.0 (or set target_resolution)".into(),
        ));
    }
    if request.target_scale > MAX_SCALE {
        return Err(BackendError::InvalidRequest(format!(
            "target_scale {} exceeds supir cap of {MAX_SCALE}",
            request.target_scale
        )));
    }
    Ok(())
}

fn looks_like_image(url: &str) -> bool {
    let lower = url.split('?').next().unwrap_or(url).to_lowercase();
    SUPPORTED_EXTS.iter().any(|ext| lower.ends_with(&format!(".{ext}")))
}

fn effective_scale(request: &UpscaleRequest) -> f32 {
    if let Some((w, _h)) = request.target_resolution {
        let derived = (w as f32 / 1024.0).clamp(1.0, MAX_SCALE);
        return derived;
    }
    request.target_scale.clamp(1.0, MAX_SCALE)
}

#[derive(Debug, Serialize)]
struct SupirBody {
    image_url: String,
    scale: f32,
}

#[derive(Debug, Deserialize)]
struct SupirResponse {
    image: FalImageFile,
}

#[derive(Debug, Deserialize)]
struct FalImageFile {
    url: String,
    #[serde(default)]
    content_type: Option<String>,
    #[serde(default)]
    width: Option<u32>,
    #[serde(default)]
    height: Option<u32>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fresh_cache() -> std::path::PathBuf {
        let tmp = std::env::temp_dir().join(format!(
            "wavelet-fal-supir-{}",
            AssetCache::request_hash("seed", "seed", &"x").unwrap()
        ));
        let _ = std::fs::remove_dir_all(&tmp);
        tmp
    }

    #[test]
    fn empty_url_rejected() {
        let client = FalClient::with_key("id:secret", fresh_cache());
        let adapter = FalSupirAdapter::new(client);
        assert!(matches!(
            adapter
                .upscale(&UpscaleRequest::new(""), RunMode::DryRun)
                .unwrap_err(),
            BackendError::InvalidRequest(_)
        ));
    }

    #[test]
    fn video_url_rejected() {
        let client = FalClient::with_key("id:secret", fresh_cache());
        let adapter = FalSupirAdapter::new(client);
        let req = UpscaleRequest::new("https://x/clip.mp4");
        let err = adapter.upscale(&req, RunMode::DryRun).unwrap_err();
        assert!(matches!(err, BackendError::InvalidRequest(_)));
    }

    #[test]
    fn scale_above_cap_rejected() {
        let client = FalClient::with_key("id:secret", fresh_cache());
        let adapter = FalSupirAdapter::new(client);
        let req = UpscaleRequest::new("https://x/y.png").with_scale(8.0);
        let err = adapter.upscale(&req, RunMode::DryRun).unwrap_err();
        assert!(matches!(err, BackendError::InvalidRequest(_)));
    }

    #[test]
    fn scale_at_or_below_one_rejected() {
        let client = FalClient::with_key("id:secret", fresh_cache());
        let adapter = FalSupirAdapter::new(client);
        let req = UpscaleRequest::new("https://x/y.png").with_scale(1.0);
        let err = adapter.upscale(&req, RunMode::DryRun).unwrap_err();
        assert!(matches!(err, BackendError::InvalidRequest(_)));
    }

    #[test]
    fn dry_run_emits_request_shape() {
        let client = FalClient::with_key("id:secret", fresh_cache());
        let adapter = FalSupirAdapter::new(client.clone());
        let req = UpscaleRequest::new("https://example.com/scene.png").with_scale(2.0);
        let out = adapter.upscale(&req, RunMode::DryRun).unwrap();
        assert_eq!(out.mode, "dry-run");
        assert_eq!(out.response.provider, PROVIDER);
        assert!(out.response.output_path.to_string_lossy().ends_with(".png"));
    }

    #[test]
    fn response_decodes_minimal_payload() {
        let body = r#"{
            "image": {
                "url": "https://x/y.png",
                "content_type": "image/png",
                "width": 2048,
                "height": 1536
            }
        }"#;
        let parsed: SupirResponse = serde_json::from_str(body).unwrap();
        assert_eq!(parsed.image.url, "https://x/y.png");
        assert_eq!(parsed.image.width, Some(2048));
    }

    #[test]
    fn effective_scale_picks_resolution_when_set() {
        let req = UpscaleRequest::new("https://x/y.png")
            .with_scale(2.0)
            .with_resolution(3840, 2160);
        let scale = effective_scale(&req);
        assert!(scale > 1.0 && scale <= 4.0);
    }

    #[test]
    fn looks_like_image_recognizes_extensions() {
        assert!(looks_like_image("https://x/y.png"));
        assert!(looks_like_image("https://x/y.JPG"));
        assert!(looks_like_image("https://x/y.webp?token=abc"));
        assert!(!looks_like_image("https://x/y.mp4"));
        assert!(!looks_like_image("https://x/y.mov"));
        assert!(!looks_like_image("https://x/y"));
    }
}
