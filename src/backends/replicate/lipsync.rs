//! Replicate Sync Labs lip-sync 2 Pro adapter — `LipSync` cluster.
//!
//! Wraps `sync/lipsync-2-pro` (verified live 2026-05-19). Hedra
//! Character-3 is the phoneme-accurate winner per May-2026 SOTA
//! research, but Hedra isn't on Fal (probed: every `fal-ai/hedra*` and
//! `fal-ai/character-3` path 404s), and we'd rather not add a third
//! HTTP transport for one adapter. sync.so's lipsync-2-pro is the
//! "studio-grade" tier in Replicate's lipsync collection — the closest
//! shipping equivalent.
//!
//! Pricing: sync.so charges $0.20 / minute of output. For a typical
//! 8-15s commercial shot this is $0.027–0.05.

use serde::{Deserialize, Serialize};

use crate::backends::cache::{utc_now_iso8601, AssetCache, Manifest};
use crate::backends::video::{
    LipSyncBackend, LipSyncRequest, VideoResult, CLUSTER_LIPSYNC,
};
use crate::backends::{
    check_budget, mode_label, BackendCallOutcome, BackendError, CostEstimate, RunMode,
};

use super::client::ReplicateClient;

/// Replicate model path.
pub const MODEL_SYNC_LIPSYNC_2_PRO: &str = "sync/lipsync-2-pro";

/// Pinned model version (probed live 2026-05-19).
pub const MODEL_SYNC_LIPSYNC_2_PRO_VERSION: &str =
    "11f76931a8a9dbaea7958865fced66b2ee03ec0fda2928dbc7cb432c7bb48c6c";

/// Per-minute price. sync.so's published lipsync-2-pro rate.
pub const PRICE_PER_MINUTE_USD: f32 = 0.20;

/// Provider id — cache key.
pub const PROVIDER: &str = "replicate-sync-lipsync-2-pro";

/// Replicate sync.so lipsync-2-pro adapter.
#[derive(Debug, Clone)]
pub struct ReplicateSyncLipSyncAdapter {
    client: ReplicateClient,
}

impl ReplicateSyncLipSyncAdapter {
    /// Build from a pre-constructed client.
    pub fn new(client: ReplicateClient) -> Self {
        Self { client }
    }
}

impl LipSyncBackend for ReplicateSyncLipSyncAdapter {
    fn name(&self) -> &'static str {
        PROVIDER
    }

    fn estimate_cost(&self, _: &LipSyncRequest) -> CostEstimate {
        // Without probing the audio file we don't know the duration —
        // bill the conservative single-minute floor. Callers that need
        // a tighter estimate can pre-probe with ffprobe and override
        // their --max-cost.
        CostEstimate {
            provider: PROVIDER.into(),
            cost_usd: PRICE_PER_MINUTE_USD,
            explanation: format!(
                "${PRICE_PER_MINUTE_USD:.2}/minute (conservative single-minute floor)"
            ),
        }
    }

    fn sync(
        &self,
        request: &LipSyncRequest,
        mode: RunMode,
    ) -> Result<BackendCallOutcome<VideoResult>, BackendError> {
        if request.video.trim().is_empty() {
            return Err(BackendError::InvalidRequest("video is empty".into()));
        }
        if request.audio.trim().is_empty() {
            return Err(BackendError::InvalidRequest("audio is empty".into()));
        }
        let estimate = self.estimate_cost(request);
        check_budget(&estimate, mode)?;

        let request_hash = AssetCache::request_hash(PROVIDER, CLUSTER_LIPSYNC, request)?;
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
                duration_secs: 0.0,
                width: 0,
                height: 0,
                mime: "video/mp4".into(),
                prompt_sent: format!("video={} audio={}", request.video, request.audio),
                seed_used: None,
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

        let input = LipSyncInput {
            video: request.video.clone(),
            audio: request.audio.clone(),
            sync_mode: request.sync_mode.clone(),
            temperature: request.temperature,
            active_speaker: request.active_speaker,
        };
        let pred = self
            .client
            .run_prediction::<_, String>(MODEL_SYNC_LIPSYNC_2_PRO_VERSION, &input)?;
        match pred.status.as_deref() {
            Some("succeeded") => {}
            Some("failed") => {
                return Err(BackendError::Transport(format!(
                    "sync.so lipsync prediction {} failed: {}",
                    pred.id,
                    pred.error.unwrap_or_else(|| "no error message".into())
                )));
            }
            other => {
                return Err(BackendError::Transport(format!(
                    "sync.so lipsync prediction {} ended with status {other:?}",
                    pred.id
                )));
            }
        }
        let url = pred
            .output
            .ok_or_else(|| BackendError::Decode("sync.so lipsync output is null".into()))?;

        let bytes = self.client.fetch_asset(&url)?;
        let video_path = cache.write_asset(PROVIDER, &request_hash, "mp4", &bytes)?;
        let video_bytes = bytes.len() as u64;

        let result = VideoResult {
            provider: PROVIDER.into(),
            video_path: video_path.clone(),
            video_bytes,
            duration_secs: 0.0,
            width: 0,
            height: 0,
            mime: "video/mp4".into(),
            prompt_sent: format!("video={} audio={}", request.video, request.audio),
            seed_used: None,
        };
        let manifest = Manifest {
            version: 1,
            provider: PROVIDER.into(),
            cluster: CLUSTER_LIPSYNC.into(),
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
struct LipSyncInput {
    video: String,
    audio: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    sync_mode: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    active_speaker: Option<bool>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fresh_cache() -> std::path::PathBuf {
        let tmp = std::env::temp_dir().join(format!(
            "wavelet-replicate-lipsync-{}",
            AssetCache::request_hash("seed", "seed", &"x").unwrap()
        ));
        let _ = std::fs::remove_dir_all(&tmp);
        tmp
    }

    fn stub() -> ReplicateSyncLipSyncAdapter {
        ReplicateSyncLipSyncAdapter::new(ReplicateClient::with_token("test-token", fresh_cache()))
    }

    #[test]
    fn cost_estimate_uses_floor() {
        let adapter = stub();
        let req = LipSyncRequest::new("https://x/v.mp4", "https://x/a.mp3");
        let est = adapter.estimate_cost(&req);
        assert!((est.cost_usd - PRICE_PER_MINUTE_USD).abs() < 1e-4);
    }

    #[test]
    fn empty_video_rejected() {
        let adapter = stub();
        let req = LipSyncRequest::new("", "https://x/a.mp3");
        let err = adapter.sync(&req, RunMode::DryRun).unwrap_err();
        assert!(matches!(err, BackendError::InvalidRequest(_)));
    }

    #[test]
    fn empty_audio_rejected() {
        let adapter = stub();
        let req = LipSyncRequest::new("https://x/v.mp4", "");
        let err = adapter.sync(&req, RunMode::DryRun).unwrap_err();
        assert!(matches!(err, BackendError::InvalidRequest(_)));
    }

    #[test]
    fn dry_run_returns_request_shape() {
        let adapter = stub();
        let req = LipSyncRequest::new("https://x/v.mp4", "https://x/a.mp3");
        let outcome = adapter.sync(&req, RunMode::DryRun).unwrap();
        assert_eq!(outcome.provider, PROVIDER);
        assert!(!outcome.cached);
        assert!(outcome.cost_estimate_usd > 0.0);
    }

    #[test]
    fn input_serializes_with_optional_fields_skipped() {
        let input = LipSyncInput {
            video: "https://x/v.mp4".into(),
            audio: "https://x/a.mp3".into(),
            sync_mode: None,
            temperature: None,
            active_speaker: None,
        };
        let json = serde_json::to_string(&input).unwrap();
        assert!(!json.contains("sync_mode"));
        assert!(!json.contains("temperature"));
        assert!(!json.contains("active_speaker"));
        assert!(json.contains("\"video\""));
        assert!(json.contains("\"audio\""));
    }
}
