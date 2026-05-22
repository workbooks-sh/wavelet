//! Replicate Seedance 1 Pro adapter — `Img2VidGen` + `Txt2VidGen`.
//!
//! Wraps `bytedance/seedance-1-pro` (verified live 2026-05-19).
//! Subject-reference video: the `image` input field locks the subject
//! across the shot, fixing the "car looks frozen" / "actor freezes
//! between cuts" problem the plain Wan i2v adapters have.
//!
//! Pricing (Replicate published): ~$0.15 per second of output. Adapters
//! charge a flat conservative estimate per request — duration ranges
//! 3-10 seconds.

use serde::{Deserialize, Serialize};

use crate::backends::cache::{utc_now_iso8601, AssetCache, Manifest};
use crate::backends::video::{
    Img2VidGenBackend, Img2VidRequest, Txt2VidGenBackend, Txt2VidRequest, VideoResult,
    CLUSTER_IMG2VID, CLUSTER_TXT2VID,
};
use crate::backends::{
    check_budget, mode_label, BackendCallOutcome, BackendError, CostEstimate, RunMode,
};

use super::client::ReplicateClient;

/// Replicate model path.
pub const MODEL_SEEDANCE_1_PRO: &str = "bytedance/seedance-1-pro";

/// Pinned model version hash. Bump when Replicate publishes a new
/// version we explicitly verify. The hash is part of the cache key, so
/// pinning shields cached outputs from silent model updates.
pub const MODEL_SEEDANCE_1_PRO_VERSION: &str =
    "a5fd550893da3b6f67997812759065652454ddaca10e96b83b59cbae1814cb36";

/// Per-second cost (USD). Replicate's published Seedance Pro rate.
pub const PRICE_PER_SECOND_USD: f32 = 0.15;

/// Provider identifier — used in cache keys.
pub const PROVIDER: &str = "replicate-seedance-1-pro";

/// Replicate Seedance 1 Pro adapter.
#[derive(Debug, Clone)]
pub struct ReplicateSeedanceProAdapter {
    client: ReplicateClient,
}

impl ReplicateSeedanceProAdapter {
    /// Build from a pre-constructed client.
    pub fn new(client: ReplicateClient) -> Self {
        Self { client }
    }
}

impl Txt2VidGenBackend for ReplicateSeedanceProAdapter {
    fn name(&self) -> &'static str {
        PROVIDER
    }

    fn estimate_cost(&self, request: &Txt2VidRequest) -> CostEstimate {
        let cost = request.duration_secs * PRICE_PER_SECOND_USD;
        CostEstimate {
            provider: PROVIDER.into(),
            cost_usd: cost,
            explanation: format!(
                "{:.1}s × ${PRICE_PER_SECOND_USD:.2}/s = ${cost:.4}",
                request.duration_secs
            ),
        }
    }

    fn generate(
        &self,
        request: &Txt2VidRequest,
        mode: RunMode,
    ) -> Result<BackendCallOutcome<VideoResult>, BackendError> {
        if request.prompt.trim().is_empty() {
            return Err(BackendError::InvalidRequest("prompt is empty".into()));
        }
        let estimate = <Self as Txt2VidGenBackend>::estimate_cost(self, request);
        check_budget(&estimate, mode)?;
        let request_hash = AssetCache::request_hash(PROVIDER, CLUSTER_TXT2VID, request)?;
        let cache = self.client.cache();

        if let Some(manifest) = cache.hit(PROVIDER, &request_hash)? {
            return cached(manifest, &request_hash, mode);
        }
        if !mode.is_live() {
            return dry_run(cache, &request_hash, request.prompt.clone(), request.duration_secs, request.seed, estimate.cost_usd, mode);
        }

        let input = SeedanceInput {
            prompt: Some(request.prompt.clone()),
            image: None,
            duration: Some(request.duration_secs.round() as i32),
            aspect_ratio: Some(request.aspect_ratio.clone()),
            resolution: Some("1080p".into()),
            fps: None,
            camera_fixed: None,
            last_frame_image: None,
            seed: request.seed,
        };
        live_call(&self.client, &input, request, CLUSTER_TXT2VID, &request_hash, &estimate, mode, request.prompt.clone(), request.seed)
    }
}

impl Img2VidGenBackend for ReplicateSeedanceProAdapter {
    fn name(&self) -> &'static str {
        PROVIDER
    }

    fn estimate_cost(&self, request: &Img2VidRequest) -> CostEstimate {
        let cost = request.duration_secs * PRICE_PER_SECOND_USD;
        CostEstimate {
            provider: PROVIDER.into(),
            cost_usd: cost,
            explanation: format!(
                "{:.1}s × ${PRICE_PER_SECOND_USD:.2}/s = ${cost:.4}",
                request.duration_secs
            ),
        }
    }

    fn generate(
        &self,
        request: &Img2VidRequest,
        mode: RunMode,
    ) -> Result<BackendCallOutcome<VideoResult>, BackendError> {
        if request.prompt.trim().is_empty() {
            return Err(BackendError::InvalidRequest("prompt is empty".into()));
        }
        if request.image.trim().is_empty() {
            return Err(BackendError::InvalidRequest(
                "image is empty (Seedance needs a subject ref)".into(),
            ));
        }
        let estimate = <Self as Img2VidGenBackend>::estimate_cost(self, request);
        check_budget(&estimate, mode)?;
        let request_hash = AssetCache::request_hash(PROVIDER, CLUSTER_IMG2VID, request)?;
        let cache = self.client.cache();

        if let Some(manifest) = cache.hit(PROVIDER, &request_hash)? {
            return cached(manifest, &request_hash, mode);
        }
        if !mode.is_live() {
            return dry_run(cache, &request_hash, request.prompt.clone(), request.duration_secs, request.seed, estimate.cost_usd, mode);
        }

        let input = SeedanceInput {
            prompt: Some(request.prompt.clone()),
            image: Some(request.image.clone()),
            duration: Some(request.duration_secs.round() as i32),
            aspect_ratio: None,
            resolution: Some("1080p".into()),
            fps: None,
            camera_fixed: None,
            last_frame_image: request.last_frame_url.clone(),
            seed: request.seed,
        };
        live_call(&self.client, &input, request, CLUSTER_IMG2VID, &request_hash, &estimate, mode, request.prompt.clone(), request.seed)
    }
}

fn cached(
    manifest: Manifest,
    request_hash: &str,
    mode: RunMode,
) -> Result<BackendCallOutcome<VideoResult>, BackendError> {
    let response: VideoResult = serde_json::from_value(manifest.response.clone())
        .map_err(|e| BackendError::Cache(format!("decode cached response: {e}")))?;
    Ok(BackendCallOutcome {
        response,
        provider: PROVIDER.into(),
        request_hash: request_hash.into(),
        cached: true,
        cost_estimate_usd: 0.0,
        mode: mode_label(mode),
    })
}

fn dry_run(
    cache: &AssetCache,
    request_hash: &str,
    prompt_sent: String,
    duration_secs: f32,
    seed_used: Option<u64>,
    cost_estimate_usd: f32,
    mode: RunMode,
) -> Result<BackendCallOutcome<VideoResult>, BackendError> {
    let response = VideoResult {
        provider: PROVIDER.into(),
        video_path: cache.asset_path(PROVIDER, request_hash, "mp4"),
        video_bytes: 0,
        duration_secs,
        width: 0,
        height: 0,
        mime: "video/mp4".into(),
        prompt_sent,
        seed_used,
    };
    Ok(BackendCallOutcome {
        response,
        provider: PROVIDER.into(),
        request_hash: request_hash.into(),
        cached: false,
        cost_estimate_usd,
        mode: mode_label(mode),
    })
}

#[allow(clippy::too_many_arguments)]
fn live_call<Req: Serialize>(
    client: &ReplicateClient,
    input: &SeedanceInput,
    request: &Req,
    cluster: &'static str,
    request_hash: &str,
    estimate: &CostEstimate,
    mode: RunMode,
    prompt_sent: String,
    seed_used: Option<u64>,
) -> Result<BackendCallOutcome<VideoResult>, BackendError> {
    let pred = client.run_prediction::<_, String>(MODEL_SEEDANCE_1_PRO_VERSION, input)?;
    match pred.status.as_deref() {
        Some("succeeded") => {}
        Some("failed") => {
            return Err(BackendError::Transport(format!(
                "Seedance prediction {} failed: {}",
                pred.id,
                pred.error.unwrap_or_else(|| "no error message".into())
            )));
        }
        Some(other) => {
            return Err(BackendError::Transport(format!(
                "Seedance prediction {} ended with status `{other}`",
                pred.id
            )));
        }
        None => {
            return Err(BackendError::Decode(
                "Seedance prediction returned without a status".into(),
            ));
        }
    }
    let url = pred
        .output
        .ok_or_else(|| BackendError::Decode("Seedance succeeded but output is null".into()))?;

    let bytes = client.fetch_asset(&url)?;
    let cache = client.cache();
    let video_path = cache.write_asset(PROVIDER, request_hash, "mp4", &bytes)?;
    let video_bytes = bytes.len() as u64;

    let result = VideoResult {
        provider: PROVIDER.into(),
        video_path: video_path.clone(),
        video_bytes,
        duration_secs: input.duration.unwrap_or(0) as f32,
        width: 0,
        height: 0,
        mime: "video/mp4".into(),
        prompt_sent,
        seed_used,
    };
    let manifest = Manifest {
        version: 1,
        provider: PROVIDER.into(),
        cluster: cluster.into(),
        request_hash: request_hash.into(),
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
        request_hash: request_hash.into(),
        cached: false,
        cost_estimate_usd: estimate.cost_usd,
        mode: mode_label(mode),
    })
}

#[derive(Debug, Serialize, Deserialize)]
struct SeedanceInput {
    #[serde(skip_serializing_if = "Option::is_none")]
    prompt: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    image: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    duration: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    aspect_ratio: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    resolution: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    fps: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    camera_fixed: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    last_frame_image: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    seed: Option<u64>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fresh_cache() -> std::path::PathBuf {
        let tmp = std::env::temp_dir().join(format!(
            "wavelet-replicate-seedance-{}",
            AssetCache::request_hash("seed", "seed", &"x").unwrap()
        ));
        let _ = std::fs::remove_dir_all(&tmp);
        tmp
    }

    fn stub() -> ReplicateSeedanceProAdapter {
        ReplicateSeedanceProAdapter::new(ReplicateClient::with_token("test-token", fresh_cache()))
    }

    #[test]
    fn cost_scales_with_duration() {
        let adapter = stub();
        let req = Txt2VidRequest {
            prompt: "x".into(),
            negative_prompt: None,
            apply_default_negatives: true,
            duration_secs: 5.0,
            aspect_ratio: "16:9".into(),
            seed: None,
        };
        let est = <ReplicateSeedanceProAdapter as Txt2VidGenBackend>::estimate_cost(&adapter, &req);
        assert!((est.cost_usd - 5.0 * PRICE_PER_SECOND_USD).abs() < 1e-4);
    }

    #[test]
    fn dry_run_returns_request_shape() {
        let adapter = stub();
        let req = Img2VidRequest {
            prompt: "the car drives".into(),
            image: "https://example.com/car.png".into(),
            last_frame_url: None,
            negative_prompt: None,
            apply_default_negatives: true,
            duration_secs: 5.0,
            aspect_ratio: "16:9".into(),
            seed: Some(7),
        };
        let outcome =
            <ReplicateSeedanceProAdapter as Img2VidGenBackend>::generate(&adapter, &req, RunMode::DryRun)
                .unwrap();
        assert_eq!(outcome.provider, PROVIDER);
        assert!(!outcome.cached);
        assert!(outcome.cost_estimate_usd > 0.0);
    }

    #[test]
    fn empty_image_in_i2v_rejected() {
        let adapter = stub();
        let req = Img2VidRequest {
            prompt: "x".into(),
            image: "  ".into(),
            last_frame_url: None,
            negative_prompt: None,
            apply_default_negatives: true,
            duration_secs: 4.0,
            aspect_ratio: "16:9".into(),
            seed: None,
        };
        let err =
            <ReplicateSeedanceProAdapter as Img2VidGenBackend>::generate(&adapter, &req, RunMode::DryRun)
                .unwrap_err();
        assert!(matches!(err, BackendError::InvalidRequest(_)));
    }

    #[test]
    fn input_serializes_with_image_for_i2v() {
        let input = SeedanceInput {
            prompt: Some("x".into()),
            image: Some("https://example.com/x.png".into()),
            duration: Some(5),
            aspect_ratio: None,
            resolution: Some("1080p".into()),
            fps: None,
            camera_fixed: None,
            last_frame_image: None,
            seed: Some(42),
        };
        let v = serde_json::to_value(&input).unwrap();
        assert_eq!(v["prompt"], "x");
        assert_eq!(v["image"], "https://example.com/x.png");
        assert_eq!(v["duration"], 5);
        assert_eq!(v["resolution"], "1080p");
        assert_eq!(v["seed"], 42);
        assert!(v.get("aspect_ratio").is_none());
    }
}
