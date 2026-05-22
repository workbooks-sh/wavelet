//! Replicate Wan 2.7 R2V adapter — `MultiRefVideoGen` cluster.
//!
//! Wraps `wan-video/wan-2.7-r2v` (probed live 2026-05-19). Accepts
//! 1–N reference images and (optionally) reference videos; the model
//! conditions on all of them simultaneously. This is the closest
//! Replicate equivalent to Wan VACE — subject preservation plus
//! multi-control (depth maps, canny edges, pose stacks all live as
//! peers in the `reference_images` array; the model decides how to
//! weight them).
//!
//! Per wb-pi6a: 6/15 production ComfyUI commercial workflows stack
//! depth + canny into Wan-VACE-class i2v models. This adapter lets
//! the agent pre-extract depth/canny with separate tools and feed all
//! refs in one R2V call.
//!
//! Pricing: ~$0.20 / 5s video (Replicate published rate for the R2V
//! tier — billing is per-output-second, conservative single-call
//! floor used by the cost estimator).

use serde::{Deserialize, Serialize};

use crate::backends::cache::{utc_now_iso8601, AssetCache, Manifest};
use crate::backends::video::{
    MultiRefVideoGenBackend, MultiRefVideoRequest, VideoResult, CLUSTER_MULTI_REF_VIDEO,
};
use crate::backends::{
    check_budget, mode_label, BackendCallOutcome, BackendError, CostEstimate, RunMode,
};

use super::client::ReplicateClient;

/// Replicate model path.
pub const MODEL_WAN_2_7_R2V: &str = "wan-video/wan-2.7-r2v";

/// Pinned model version (probed 2026-05-19).
pub const MODEL_WAN_2_7_R2V_VERSION: &str =
    "00f2bfc4cadaa306f7d52b705c06aee00a53457f6a5aa5bd72a67e4e08627a41";

/// Per-second cost — conservative bench against Replicate's published
/// R2V pricing.
pub const PRICE_PER_SECOND_USD: f32 = 0.04;

/// Max reference images per request — Wan 2.7 R2V soft-caps around
/// 6 refs before subject identity drifts. Adapters reject above this.
pub const MAX_REF_IMAGES: usize = 6;

/// Provider id.
pub const PROVIDER: &str = "replicate-wan-2.7-r2v";

/// Replicate Wan 2.7 R2V adapter.
#[derive(Debug, Clone)]
pub struct ReplicateWanR2vAdapter {
    client: ReplicateClient,
}

impl ReplicateWanR2vAdapter {
    /// Build from a pre-constructed client.
    pub fn new(client: ReplicateClient) -> Self {
        Self { client }
    }
}

impl MultiRefVideoGenBackend for ReplicateWanR2vAdapter {
    fn name(&self) -> &'static str {
        PROVIDER
    }

    fn estimate_cost(&self, request: &MultiRefVideoRequest) -> CostEstimate {
        let cost = request.duration_secs * PRICE_PER_SECOND_USD;
        CostEstimate {
            provider: PROVIDER.into(),
            cost_usd: cost,
            explanation: format!(
                "{:.1}s × ${PRICE_PER_SECOND_USD:.2}/s = ${cost:.4} (Wan 2.7 R2V, Replicate)",
                request.duration_secs
            ),
        }
    }

    fn generate(
        &self,
        request: &MultiRefVideoRequest,
        mode: RunMode,
    ) -> Result<BackendCallOutcome<VideoResult>, BackendError> {
        if request.prompt.trim().is_empty() {
            return Err(BackendError::InvalidRequest("prompt is empty".into()));
        }
        if request.reference_images.is_empty() && request.reference_videos.is_empty() {
            return Err(BackendError::InvalidRequest(
                "need at least one entry in reference_images or reference_videos".into(),
            ));
        }
        if request.reference_images.len() > MAX_REF_IMAGES {
            return Err(BackendError::InvalidRequest(format!(
                "reference_images has {} entries, max {MAX_REF_IMAGES}",
                request.reference_images.len()
            )));
        }
        for url in request
            .reference_images
            .iter()
            .chain(request.reference_videos.iter())
        {
            if url.trim().is_empty() {
                return Err(BackendError::InvalidRequest(
                    "reference list contains an empty entry".into(),
                ));
            }
        }
        let estimate = self.estimate_cost(request);
        check_budget(&estimate, mode)?;

        let request_hash =
            AssetCache::request_hash(PROVIDER, CLUSTER_MULTI_REF_VIDEO, request)?;
        let cache = self.client.cache();

        if let Some(manifest) = cache.hit(PROVIDER, &request_hash)? {
            let response: VideoResult = serde_json::from_value(manifest.response.clone())
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
            let response = VideoResult {
                provider: PROVIDER.into(),
                video_path: cache.asset_path(PROVIDER, &request_hash, "mp4"),
                video_bytes: 0,
                duration_secs: request.duration_secs,
                width: 0,
                height: 0,
                mime: "video/mp4".into(),
                prompt_sent: request.prompt.clone(),
                seed_used: request.seed,
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

        let input = WanR2vInput {
            prompt: request.prompt.clone(),
            reference_images: request.reference_images.clone(),
            reference_videos: request.reference_videos.clone(),
            negative_prompt: request.negative_prompt.clone(),
            duration: Some(request.duration_secs.round() as i32),
            aspect_ratio: Some(request.aspect_ratio.clone()),
            resolution: Some("720p".into()),
            seed: request.seed.map(|s| s as i64),
        };
        let pred = self
            .client
            .run_prediction::<_, String>(MODEL_WAN_2_7_R2V_VERSION, &input)?;
        match pred.status.as_deref() {
            Some("succeeded") => {}
            Some("failed") => {
                return Err(BackendError::Transport(format!(
                    "Wan R2V prediction {} failed: {}",
                    pred.id,
                    pred.error.unwrap_or_else(|| "no error message".into())
                )));
            }
            other => {
                return Err(BackendError::Transport(format!(
                    "Wan R2V prediction {} ended with status {other:?}",
                    pred.id
                )));
            }
        }
        let url = pred
            .output
            .ok_or_else(|| BackendError::Decode("Wan R2V output is null".into()))?;
        let bytes = self.client.fetch_asset(&url)?;
        let video_path = cache.write_asset(PROVIDER, &request_hash, "mp4", &bytes)?;
        let video_bytes = bytes.len() as u64;

        let result = VideoResult {
            provider: PROVIDER.into(),
            video_path: video_path.clone(),
            video_bytes,
            duration_secs: input.duration.unwrap_or(0) as f32,
            width: 0,
            height: 0,
            mime: "video/mp4".into(),
            prompt_sent: input.prompt.clone(),
            seed_used: input.seed.map(|s| s as u64),
        };
        let manifest = Manifest {
            version: 1,
            provider: PROVIDER.into(),
            cluster: CLUSTER_MULTI_REF_VIDEO.into(),
            request_hash: request_hash.clone(),
            request: serde_json::to_value(request)
                .map_err(|e| BackendError::Cache(format!("serialize request for cache: {e}")))?,
            response: serde_json::to_value(&result)
                .map_err(|e| BackendError::Cache(format!("serialize response for cache: {e}")))?,
            cost_estimate_usd: estimate.cost_usd,
            asset_path: Some(video_path.display().to_string()),
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

#[derive(Debug, Serialize, Deserialize)]
struct WanR2vInput {
    prompt: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    reference_images: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    reference_videos: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    negative_prompt: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    duration: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    aspect_ratio: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    resolution: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    seed: Option<i64>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fresh_cache() -> std::path::PathBuf {
        let tmp = std::env::temp_dir().join(format!(
            "wavelet-replicate-wan-r2v-{}",
            AssetCache::request_hash("seed", "seed", &"x").unwrap()
        ));
        let _ = std::fs::remove_dir_all(&tmp);
        tmp
    }

    fn stub() -> ReplicateWanR2vAdapter {
        ReplicateWanR2vAdapter::new(ReplicateClient::with_token("test-token", fresh_cache()))
    }

    #[test]
    fn cost_scales_with_duration() {
        let adapter = stub();
        let req = MultiRefVideoRequest::new("x", vec!["https://x/r1.png".into()]);
        let est = adapter.estimate_cost(&req);
        assert!((est.cost_usd - 5.0 * PRICE_PER_SECOND_USD).abs() < 1e-4);
    }

    #[test]
    fn empty_prompt_rejected() {
        let adapter = stub();
        let req = MultiRefVideoRequest::new("  ", vec!["https://x/r.png".into()]);
        let err = adapter.generate(&req, RunMode::DryRun).unwrap_err();
        assert!(matches!(err, BackendError::InvalidRequest(_)));
    }

    #[test]
    fn empty_ref_list_rejected() {
        let adapter = stub();
        let req = MultiRefVideoRequest::new("x", vec![]);
        let err = adapter.generate(&req, RunMode::DryRun).unwrap_err();
        assert!(matches!(err, BackendError::InvalidRequest(_)));
    }

    #[test]
    fn too_many_refs_rejected() {
        let adapter = stub();
        let req = MultiRefVideoRequest::new(
            "x",
            (0..(MAX_REF_IMAGES + 1))
                .map(|i| format!("https://x/r{i}.png"))
                .collect(),
        );
        let err = adapter.generate(&req, RunMode::DryRun).unwrap_err();
        assert!(matches!(err, BackendError::InvalidRequest(_)));
    }

    #[test]
    fn dry_run_returns_request_shape() {
        let adapter = stub();
        let req = MultiRefVideoRequest::new(
            "the car drives, lit by golden hour",
            vec![
                "https://x/subject.png".into(),
                "https://x/depth.png".into(),
                "https://x/canny.png".into(),
            ],
        );
        let outcome = adapter.generate(&req, RunMode::DryRun).unwrap();
        assert_eq!(outcome.provider, PROVIDER);
        assert!(!outcome.cached);
        assert!(outcome.cost_estimate_usd > 0.0);
    }

    #[test]
    fn input_serializes_with_refs() {
        let input = WanR2vInput {
            prompt: "x".into(),
            reference_images: vec!["https://x/r1.png".into(), "https://x/r2.png".into()],
            reference_videos: vec![],
            negative_prompt: None,
            duration: Some(5),
            aspect_ratio: Some("16:9".into()),
            resolution: Some("720p".into()),
            seed: Some(42),
        };
        let v = serde_json::to_value(&input).unwrap();
        assert_eq!(v["reference_images"].as_array().unwrap().len(), 2);
        assert!(v.get("reference_videos").is_none());
        assert_eq!(v["duration"], 5);
        assert_eq!(v["seed"], 42);
    }
}
