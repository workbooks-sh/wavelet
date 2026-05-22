//! Fal Flux Schnell adapter — `Txt2Img` cluster.
//!
//! Wraps `fal-ai/flux/schnell` — fast, cheap text-to-image. Used for
//! environment plates: the backdrop the isolated subject gets composited
//! over in Path B.
//!
//! Cost: ~$0.003/image. Effectively free per shot — adds <1% to a
//! per-shot budget.

use crate::backends::cache::{utc_now_iso8601, AssetCache, Manifest};
use crate::backends::fal::FalClient;
use crate::backends::image::{
    ImageResult, Txt2ImgBackend, Txt2ImgRequest, CLUSTER_TXT2IMG,
};
use crate::backends::{
    check_budget, mode_label, BackendCallOutcome, BackendError, CostEstimate, RunMode,
};
use serde::{Deserialize, Serialize};

/// Provider id.
pub const PROVIDER: &str = "fal-flux-schnell";

/// Fal model path.
pub const MODEL_PATH: &str = "fal-ai/flux/schnell";

/// Per-image cost estimate (USD).
pub const PRICE_PER_CALL_USD: f32 = 0.005;

/// Fal Flux Schnell adapter.
#[derive(Debug, Clone)]
pub struct FalFluxSchnellAdapter {
    client: FalClient,
}

impl FalFluxSchnellAdapter {
    /// Build from a pre-constructed client.
    pub fn new(client: FalClient) -> Self {
        Self { client }
    }
}

impl Txt2ImgBackend for FalFluxSchnellAdapter {
    fn name(&self) -> &'static str {
        PROVIDER
    }

    fn estimate_cost(&self, _: &Txt2ImgRequest) -> CostEstimate {
        CostEstimate {
            provider: PROVIDER.into(),
            cost_usd: PRICE_PER_CALL_USD,
            explanation: format!("${PRICE_PER_CALL_USD:.4}/image (conservative)"),
        }
    }

    fn generate(
        &self,
        request: &Txt2ImgRequest,
        mode: RunMode,
    ) -> Result<BackendCallOutcome<ImageResult>, BackendError> {
        if request.prompt.trim().is_empty() {
            return Err(BackendError::InvalidRequest("prompt is empty".into()));
        }

        let estimate = self.estimate_cost(request);
        check_budget(&estimate, mode)?;

        let request_hash = AssetCache::request_hash(PROVIDER, CLUSTER_TXT2IMG, request)?;
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
                image_path: cache.asset_path(PROVIDER, &request_hash, "jpg"),
                image_bytes: 0,
                width: 0,
                height: 0,
                mime: "image/jpeg".into(),
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

        let body = FluxBody {
            prompt: request.prompt.clone(),
            image_size: request.image_size.clone(),
            seed: request.seed,
        };
        let parsed: FluxResponse = self.client.post_sync(MODEL_PATH, &body)?;
        let img = parsed
            .images
            .into_iter()
            .next()
            .ok_or_else(|| BackendError::Decode("no image returned".into()))?;
        let image_bytes_raw = self.client.fetch_asset(&img.url)?;
        let image_path = cache.write_asset(PROVIDER, &request_hash, "jpg", &image_bytes_raw)?;
        let image_bytes = image_bytes_raw.len() as u64;

        let result = ImageResult {
            provider: PROVIDER.into(),
            image_path: image_path.clone(),
            image_bytes,
            width: img.width.unwrap_or(0),
            height: img.height.unwrap_or(0),
            mime: img.content_type.unwrap_or_else(|| "image/jpeg".into()),
        };

        let manifest = Manifest {
            version: 1,
            provider: PROVIDER.into(),
            cluster: CLUSTER_TXT2IMG.into(),
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
struct FluxBody {
    prompt: String,
    image_size: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    seed: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct FluxResponse {
    images: Vec<FluxImage>,
}

#[derive(Debug, Deserialize)]
struct FluxImage {
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
            "wavelet-fal-flux-{}",
            AssetCache::request_hash("seed", "seed", &"x").unwrap()
        ));
        let _ = std::fs::remove_dir_all(&tmp);
        tmp
    }

    #[test]
    fn empty_prompt_rejected() {
        let client = FalClient::with_key("id:secret", fresh_cache());
        let adapter = FalFluxSchnellAdapter::new(client);
        assert!(matches!(
            adapter
                .generate(&Txt2ImgRequest::new(""), RunMode::DryRun)
                .unwrap_err(),
            BackendError::InvalidRequest(_)
        ));
    }

    #[test]
    fn dry_run_emits_request_shape() {
        let client = FalClient::with_key("id:secret", fresh_cache());
        let adapter = FalFluxSchnellAdapter::new(client);
        let out = adapter
            .generate(&Txt2ImgRequest::new("empty hangar"), RunMode::DryRun)
            .unwrap();
        assert_eq!(out.response.provider, PROVIDER);
        assert!(out.response.image_path.to_string_lossy().ends_with(".jpg"));
    }
}
