//! Fal EVF-SAM adapter — `SegmentByText` cluster.
//!
//! Wraps `fal-ai/evf-sam` — Early Vision Fusion SAM. Takes `image_url`
//! + `prompt` ("the car") and returns a grayscale mask. The adapter
//! then runs `compose::apply_mask` locally to produce a final RGBA
//! PNG where the subject named in the prompt is opaque and everything
//! else is transparent.
//!
//! This is the fix for "bg-remove kept the bystander" — by naming the
//! subject explicitly, the mask drops people / watermarks / other
//! cars even when they share the foreground.

use crate::backends::cache::{utc_now_iso8601, AssetCache, Manifest};
use crate::backends::fal::FalClient;
use crate::backends::image::{
    compose, ImageResult, SegmentByTextBackend, SegmentByTextRequest, CLUSTER_SEGMENT,
};
use crate::backends::util::{base64_decode, sniff_image_ext};
use crate::backends::{
    check_budget, mode_label, BackendCallOutcome, BackendError, CostEstimate, RunMode,
};
use serde::{Deserialize, Serialize};

/// Provider id.
pub const PROVIDER: &str = "fal-evf-sam";

/// Fal model path.
pub const MODEL_PATH: &str = "fal-ai/evf-sam";

/// Per-call cost estimate.
pub const PRICE_PER_CALL_USD: f32 = 0.01;

/// Fal EVF-SAM adapter.
#[derive(Debug, Clone)]
pub struct FalEvfSamAdapter {
    client: FalClient,
}

impl FalEvfSamAdapter {
    /// Build from a pre-constructed client.
    pub fn new(client: FalClient) -> Self {
        Self { client }
    }
}

impl SegmentByTextBackend for FalEvfSamAdapter {
    fn name(&self) -> &'static str {
        PROVIDER
    }

    fn estimate_cost(&self, _: &SegmentByTextRequest) -> CostEstimate {
        CostEstimate {
            provider: PROVIDER.into(),
            cost_usd: PRICE_PER_CALL_USD,
            explanation: format!("${PRICE_PER_CALL_USD:.4}/call (conservative)"),
        }
    }

    fn segment(
        &self,
        request: &SegmentByTextRequest,
        mode: RunMode,
    ) -> Result<BackendCallOutcome<ImageResult>, BackendError> {
        if request.image.trim().is_empty() {
            return Err(BackendError::InvalidRequest("image is empty".into()));
        }
        if request.prompt.trim().is_empty() {
            return Err(BackendError::InvalidRequest("prompt is empty".into()));
        }

        let estimate = self.estimate_cost(request);
        check_budget(&estimate, mode)?;

        let request_hash = AssetCache::request_hash(PROVIDER, CLUSTER_SEGMENT, request)?;
        let cache = self.client.cache();

        if let Some(manifest) = cache.hit(PROVIDER, &request_hash)? {
            let response: ImageResult = serde_json::from_value(manifest.response.clone())
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
            let response = ImageResult {
                provider: PROVIDER.into(),
                image_path: cache.asset_path(PROVIDER, &request_hash, "png"),
                image_bytes: 0,
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

        let body = EvfSamBody {
            image_url: request.image.clone(),
            prompt: request.prompt.clone(),
        };
        let parsed: EvfSamResponse = self.client.post_sync(MODEL_PATH, &body)?;
        let mask_url = parsed.image.url.clone();
        let mask_bytes = self.client.fetch_asset(&mask_url)?;
        let mask_path = cache.write_asset(PROVIDER, &request_hash, "mask.png", &mask_bytes)?;

        // Fetch the source if it's a URL; if a local path, use directly.
        // For URLs, sniff the content from the bytes (the URL extension
        // is unreliable — some CDNs serve JPEG behind .png URLs) and
        // write with the correct file extension so `apply_mask`'s
        // `image::open` can decode it.
        let source_path = if request.image.starts_with("http") {
            let src_bytes = self.client.fetch_asset(&request.image)?;
            let ext = sniff_image_ext(&src_bytes);
            let name = format!("src.{ext}");
            cache.write_asset(PROVIDER, &request_hash, &name, &src_bytes)?
        } else if request.image.starts_with("data:") {
            // data:image/png;base64,<bytes> — decode + write to cache.
            let comma = request.image.find(',').ok_or_else(|| {
                BackendError::InvalidRequest("data: URL missing comma separator".into())
            })?;
            let header = &request.image[..comma];
            let payload = &request.image[comma + 1..];
            let decoded = base64_decode(payload)
                .map_err(|e| BackendError::InvalidRequest(format!("data: URL base64: {e}")))?;
            let ext = if header.contains("jpeg") || header.contains("jpg") {
                "jpg"
            } else if header.contains("webp") {
                "webp"
            } else {
                "png"
            };
            cache.write_asset(PROVIDER, &request_hash, &format!("src.{ext}"), &decoded)?
        } else {
            std::path::PathBuf::from(&request.image)
        };

        // Apply the mask to the source → final RGBA PNG.
        let out_path = cache.asset_path(PROVIDER, &request_hash, "png");
        let (w, h) = compose::apply_mask(&source_path, &mask_path, &out_path)?;
        let image_bytes = std::fs::metadata(&out_path).map(|m| m.len()).unwrap_or(0);

        let result = ImageResult {
            provider: PROVIDER.into(),
            image_path: out_path.clone(),
            image_bytes,
            width: w,
            height: h,
            mime: "image/png".into(),
        };

        let manifest = Manifest {
            version: 1,
            provider: PROVIDER.into(),
            cluster: CLUSTER_SEGMENT.into(),
            request_hash: request_hash.clone(),
            request: serde_json::to_value(request).map_err(|e| {
                BackendError::Cache(format!("serialize request for cache: {e}"))
            })?,
            response: serde_json::to_value(&result).map_err(|e| {
                BackendError::Cache(format!("serialize response for cache: {e}"))
            })?,
            cost_estimate_usd: estimate.cost_usd,
            asset_path: Some(out_path.display().to_string()),
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

#[derive(Debug, Serialize)]
struct EvfSamBody {
    image_url: String,
    prompt: String,
}

#[derive(Debug, Deserialize)]
struct EvfSamResponse {
    image: FalImageFile,
}

#[derive(Debug, Deserialize)]
struct FalImageFile {
    url: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fresh_cache() -> std::path::PathBuf {
        let tmp = std::env::temp_dir().join(format!(
            "wavelet-fal-evfsam-{}",
            AssetCache::request_hash("seed", "seed", &"x").unwrap()
        ));
        let _ = std::fs::remove_dir_all(&tmp);
        tmp
    }

    #[test]
    fn empty_request_rejected() {
        let client = FalClient::with_key("id:secret", fresh_cache());
        let adapter = FalEvfSamAdapter::new(client);
        assert!(matches!(
            adapter
                .segment(&SegmentByTextRequest::new("", "the car"), RunMode::DryRun)
                .unwrap_err(),
            BackendError::InvalidRequest(_)
        ));
        assert!(matches!(
            adapter
                .segment(&SegmentByTextRequest::new("x.png", ""), RunMode::DryRun)
                .unwrap_err(),
            BackendError::InvalidRequest(_)
        ));
    }

    #[test]
    fn dry_run_emits_request_shape() {
        let client = FalClient::with_key("id:secret", fresh_cache());
        let adapter = FalEvfSamAdapter::new(client.clone());
        let req = SegmentByTextRequest::new("https://example.com/car.jpg", "the car");
        let out = adapter.segment(&req, RunMode::DryRun).unwrap();
        assert_eq!(out.response.provider, PROVIDER);
        assert!(out.response.image_path.to_string_lossy().ends_with(".png"));
    }
}
