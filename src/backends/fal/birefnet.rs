//! Fal birefnet adapter — `BgRemove` cluster.
//!
//! Wraps `fal-ai/birefnet` — high-quality background removal that
//! preserves fine detail (hair, leaves, mesh). Accepts `image_url`,
//! returns `{image: {url, content_type, width, height}, mask_image}`.
//!
//! Cost: ~$0.005 per image.

use crate::backends::cache::{utc_now_iso8601, AssetCache, Manifest};
use crate::backends::fal::FalClient;
use crate::backends::image::{
    BgRemoveBackend, BgRemoveRequest, ImageResult, CLUSTER_BG_REMOVE,
};
use crate::backends::{
    check_budget, mode_label, BackendCallOutcome, BackendError, CostEstimate, RunMode,
};
use serde::{Deserialize, Serialize};

/// Provider id.
pub const PROVIDER: &str = "fal-birefnet";

/// Fal model path.
pub const MODEL_PATH: &str = "fal-ai/birefnet";

/// Per-call cost estimate (USD).
pub const PRICE_PER_CALL_USD: f32 = 0.01;

/// Fal birefnet adapter.
#[derive(Debug, Clone)]
pub struct FalBirefnetAdapter {
    client: FalClient,
}

impl FalBirefnetAdapter {
    /// Build from a pre-constructed client.
    pub fn new(client: FalClient) -> Self {
        Self { client }
    }
}

impl BgRemoveBackend for FalBirefnetAdapter {
    fn name(&self) -> &'static str {
        PROVIDER
    }

    fn estimate_cost(&self, _: &BgRemoveRequest) -> CostEstimate {
        CostEstimate {
            provider: PROVIDER.into(),
            cost_usd: PRICE_PER_CALL_USD,
            explanation: format!("${PRICE_PER_CALL_USD:.4}/call (conservative)"),
        }
    }

    fn remove_bg(
        &self,
        request: &BgRemoveRequest,
        mode: RunMode,
    ) -> Result<BackendCallOutcome<ImageResult>, BackendError> {
        if request.image.trim().is_empty() {
            return Err(BackendError::InvalidRequest("image url is empty".into()));
        }

        let estimate = self.estimate_cost(request);
        check_budget(&estimate, mode)?;

        let request_hash = AssetCache::request_hash(PROVIDER, CLUSTER_BG_REMOVE, request)?;
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

        let body = BirefnetBody {
            image_url: request.image.clone(),
        };
        let parsed: BirefnetResponse = self.client.post_sync(MODEL_PATH, &body)?;
        let image_url = parsed.image.url.clone();
        let image_bytes_raw = self.client.fetch_asset(&image_url)?;
        let image_path = cache.write_asset(PROVIDER, &request_hash, "png", &image_bytes_raw)?;
        let image_bytes = image_bytes_raw.len() as u64;

        let result = ImageResult {
            provider: PROVIDER.into(),
            image_path: image_path.clone(),
            image_bytes,
            width: parsed.image.width.unwrap_or(0),
            height: parsed.image.height.unwrap_or(0),
            mime: parsed.image.content_type.unwrap_or_else(|| "image/png".into()),
        };

        let manifest = Manifest {
            version: 1,
            provider: PROVIDER.into(),
            cluster: CLUSTER_BG_REMOVE.into(),
            request_hash: request_hash.clone(),
            request: serde_json::to_value(request).map_err(|e| {
                BackendError::Cache(format!("serialize request for cache: {e}"))
            })?,
            response: serde_json::to_value(&result).map_err(|e| {
                BackendError::Cache(format!("serialize response for cache: {e}"))
            })?,
            cost_estimate_usd: estimate.cost_usd,
            asset_path: Some(image_path.display().to_string()),
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
struct BirefnetBody {
    image_url: String,
}

#[derive(Debug, Deserialize)]
struct BirefnetResponse {
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
            "wavelet-fal-birefnet-{}",
            AssetCache::request_hash("seed", "seed", &"x").unwrap()
        ));
        let _ = std::fs::remove_dir_all(&tmp);
        tmp
    }

    #[test]
    fn empty_url_rejected() {
        let client = FalClient::with_key("id:secret", fresh_cache());
        let adapter = FalBirefnetAdapter::new(client);
        assert!(matches!(
            adapter
                .remove_bg(&BgRemoveRequest::new(""), RunMode::DryRun)
                .unwrap_err(),
            BackendError::InvalidRequest(_)
        ));
    }

    #[test]
    fn dry_run_emits_request_shape() {
        let client = FalClient::with_key("id:secret", fresh_cache());
        let adapter = FalBirefnetAdapter::new(client.clone());
        let req = BgRemoveRequest::new("https://example.com/car.jpg");
        let out = adapter.remove_bg(&req, RunMode::DryRun).unwrap();
        assert_eq!(out.mode, "dry-run");
        assert_eq!(out.response.provider, PROVIDER);
        assert!(out.response.image_path.to_string_lossy().ends_with(".png"));
    }

    #[test]
    fn response_decodes_minimal_payload() {
        let body = r#"{
            "image": {
                "url": "https://x/y.png",
                "content_type": "image/png",
                "width": 1920,
                "height": 1080
            }
        }"#;
        let parsed: BirefnetResponse = serde_json::from_str(body).unwrap();
        assert_eq!(parsed.image.url, "https://x/y.png");
        assert_eq!(parsed.image.width, Some(1920));
    }
}
