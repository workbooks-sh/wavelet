//! Replicate Grounded-SAM adapter — `SegmentByText` cluster.
//!
//! Wraps `schananas/grounded_sam` (probed live 2026-05-19). Functional
//! equivalent of SAM 3's text-prompted segmentation — `mask_prompt`
//! drives Grounding DINO detection, then SAM refines the mask.
//!
//! Why not SAM 3 itself: not on Replicate yet (probed: lucataco/sam-3,
//! meta/sam-3, facebook-research/sam-3 all 404). Fal hosts at
//! `fal-ai/sam-3` (probed 403, exists but our account balance is
//! exhausted as of 2026-05-19). When SAM 3 lands on either, the
//! adapter swap is one constant.

use serde::{Deserialize, Serialize};

use crate::backends::cache::{utc_now_iso8601, AssetCache, Manifest};
use crate::backends::image::{
    ImageResult, SegmentByTextBackend, SegmentByTextRequest, CLUSTER_SEGMENT,
};
use crate::backends::{
    check_budget, mode_label, BackendCallOutcome, BackendError, CostEstimate, RunMode,
};

use super::client::ReplicateClient;

/// Replicate model path.
pub const MODEL_GROUNDED_SAM: &str = "schananas/grounded_sam";

/// Pinned version (probed 2026-05-19).
pub const MODEL_GROUNDED_SAM_VERSION: &str =
    "ee871c19efb1941f55f66a3d7d960428c8a5afcb77449547fe8e5a3ab9ebc21c";

/// Per-call cost — Replicate published rate (~$0.01).
pub const PRICE_PER_CALL_USD: f32 = 0.01;

/// Provider id.
pub const PROVIDER: &str = "replicate-grounded-sam";

/// Replicate Grounded-SAM adapter.
#[derive(Debug, Clone)]
pub struct ReplicateGroundedSamAdapter {
    client: ReplicateClient,
}

impl ReplicateGroundedSamAdapter {
    /// Build from a pre-constructed client.
    pub fn new(client: ReplicateClient) -> Self {
        Self { client }
    }
}

impl SegmentByTextBackend for ReplicateGroundedSamAdapter {
    fn name(&self) -> &'static str {
        PROVIDER
    }

    fn estimate_cost(&self, _: &SegmentByTextRequest) -> CostEstimate {
        CostEstimate {
            provider: PROVIDER.into(),
            cost_usd: PRICE_PER_CALL_USD,
            explanation: format!("${PRICE_PER_CALL_USD:.4}/call (Grounded-SAM, Replicate)"),
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

        let request_hash =
            AssetCache::request_hash(PROVIDER, CLUSTER_SEGMENT, request)?;
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

        let input = GroundedSamInput {
            image: request.image.clone(),
            mask_prompt: request.prompt.clone(),
            negative_mask_prompt: None,
            adjustment_factor: None,
        };
        let pred = self
            .client
            .run_prediction::<_, String>(MODEL_GROUNDED_SAM_VERSION, &input)?;
        match pred.status.as_deref() {
            Some("succeeded") => {}
            Some("failed") => {
                return Err(BackendError::Transport(format!(
                    "Grounded-SAM prediction {} failed: {}",
                    pred.id,
                    pred.error.unwrap_or_else(|| "no error message".into())
                )));
            }
            other => {
                return Err(BackendError::Transport(format!(
                    "Grounded-SAM prediction {} ended with status {other:?}",
                    pred.id
                )));
            }
        }
        let url = pred
            .output
            .ok_or_else(|| BackendError::Decode("Grounded-SAM output is null".into()))?;
        let bytes = self.client.fetch_asset(&url)?;
        let image_path = cache.write_asset(PROVIDER, &request_hash, "png", &bytes)?;
        let image_bytes = bytes.len() as u64;

        let result = ImageResult {
            provider: PROVIDER.into(),
            image_path: image_path.clone(),
            image_bytes,
            width: 0,
            height: 0,
            mime: "image/png".into(),
        };
        let manifest = Manifest {
            version: 1,
            provider: PROVIDER.into(),
            cluster: CLUSTER_SEGMENT.into(),
            request_hash: request_hash.clone(),
            request: serde_json::to_value(request)
                .map_err(|e| BackendError::Cache(format!("serialize request for cache: {e}")))?,
            response: serde_json::to_value(&result)
                .map_err(|e| BackendError::Cache(format!("serialize response for cache: {e}")))?,
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
struct GroundedSamInput {
    image: String,
    mask_prompt: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    negative_mask_prompt: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    adjustment_factor: Option<i32>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fresh_cache() -> std::path::PathBuf {
        let tmp = std::env::temp_dir().join(format!(
            "wavelet-replicate-grounded-sam-{}",
            AssetCache::request_hash("seed", "seed", &"x").unwrap()
        ));
        let _ = std::fs::remove_dir_all(&tmp);
        tmp
    }

    fn stub() -> ReplicateGroundedSamAdapter {
        ReplicateGroundedSamAdapter::new(ReplicateClient::with_token("test-token", fresh_cache()))
    }

    #[test]
    fn cost_is_flat() {
        let adapter = stub();
        let req = SegmentByTextRequest::new("https://x/img.png", "the car");
        let est = adapter.estimate_cost(&req);
        assert!((est.cost_usd - PRICE_PER_CALL_USD).abs() < 1e-6);
    }

    #[test]
    fn empty_image_rejected() {
        let adapter = stub();
        let req = SegmentByTextRequest::new("", "the car");
        let err = adapter.segment(&req, RunMode::DryRun).unwrap_err();
        assert!(matches!(err, BackendError::InvalidRequest(_)));
    }

    #[test]
    fn empty_prompt_rejected() {
        let adapter = stub();
        let req = SegmentByTextRequest::new("https://x/img.png", " ");
        let err = adapter.segment(&req, RunMode::DryRun).unwrap_err();
        assert!(matches!(err, BackendError::InvalidRequest(_)));
    }

    #[test]
    fn dry_run_returns_request_shape() {
        let adapter = stub();
        let req = SegmentByTextRequest::new("https://x/img.png", "the car");
        let outcome = adapter.segment(&req, RunMode::DryRun).unwrap();
        assert_eq!(outcome.provider, PROVIDER);
        assert!(!outcome.cached);
        assert!(outcome.cost_estimate_usd > 0.0);
    }

    #[test]
    fn input_serializes_minimal_shape() {
        let input = GroundedSamInput {
            image: "https://x/img.png".into(),
            mask_prompt: "the car".into(),
            negative_mask_prompt: None,
            adjustment_factor: None,
        };
        let json = serde_json::to_string(&input).unwrap();
        assert!(json.contains("\"image\""));
        assert!(json.contains("\"mask_prompt\":\"the car\""));
        assert!(!json.contains("negative_mask_prompt"));
        assert!(!json.contains("adjustment_factor"));
    }
}
