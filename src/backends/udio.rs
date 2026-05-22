//! Udio music adapter — `RefConditionedMusicGen` cluster alternative.
//!
//! ## Status: stub
//!
//! Udio's public HTTP API is gated behind a partnership program — the
//! signed-up endpoint shape is not yet documented in a stable form we
//! can wire. Dry-run is supported (so pipelines can pick `--backend
//! udio` to inspect the request spec); live calls return
//! `BackendError::Transport("udio: API access not yet wired …")` until
//! the partner endpoint is published.
//!
//! See follow-up issue `wb-udio-wire` for activation.
//!
//! ## Licensing
//!
//! Per Udio's terms of service, generated output is licensed for
//! commercial use under their partnership program. This matches the
//! commercial-safety bar set by ElevenLabs Music (Merlin+Kobalt) and
//! is the reason Udio stays in the cluster — even before the wire
//! integration lands.

use crate::backends::cache::AssetCache;
use crate::backends::music::{
    MusicResult, RefConditionedMusicGenBackend, RefConditionedMusicRequest, CLUSTER_REF_COND,
};
use crate::backends::{
    check_budget, mode_label, BackendCallOutcome, BackendError, CostEstimate, RunMode,
};
use std::path::PathBuf;

/// Provider id stored in manifests + cache keys.
pub const PROVIDER: &str = "udio";

/// Default model variant id used in manifests for forward compat.
pub const DEFAULT_VARIANT: &str = "udio-v1";

/// Conservative per-second cost estimate (USD). Held in parity with EL
/// Music so the agent's budget gate behaves identically when swapping
/// between commercially-safe backends.
pub const PRICE_PER_SEC_USD: f32 = 0.005;

/// Udio adapter. Live calls are not yet wired — dry-run + budget gates
/// are functional so the agent can plan a request without a real key.
#[derive(Debug, Clone, Default)]
pub struct UdioMusicAdapter {
    cache_root: PathBuf,
}

impl UdioMusicAdapter {
    /// Build with an explicit cache root. The adapter touches the
    /// cache only via [`AssetCache`] in dry-run; live calls bail
    /// before any cache write.
    pub fn new(cache_root: impl Into<PathBuf>) -> Self {
        Self {
            cache_root: cache_root.into(),
        }
    }
}

impl RefConditionedMusicGenBackend for UdioMusicAdapter {
    fn name(&self) -> &'static str {
        PROVIDER
    }

    fn estimate_cost(&self, request: &RefConditionedMusicRequest) -> CostEstimate {
        let cost = request.duration_secs.max(0.0) * PRICE_PER_SEC_USD;
        CostEstimate {
            provider: PROVIDER.into(),
            cost_usd: cost,
            explanation: format!(
                "{:.1}s × ${PRICE_PER_SEC_USD:.4}/sec (Udio partnership tier)",
                request.duration_secs
            ),
        }
    }

    fn generate(
        &self,
        request: &RefConditionedMusicRequest,
        mode: RunMode,
    ) -> Result<BackendCallOutcome<MusicResult>, BackendError> {
        if request.prompt.trim().is_empty() {
            return Err(BackendError::InvalidRequest("prompt is empty".into()));
        }
        if request.duration_secs <= 0.0 {
            return Err(BackendError::InvalidRequest(
                "duration_secs must be > 0".into(),
            ));
        }

        let estimate = self.estimate_cost(request);
        check_budget(&estimate, mode)?;

        let request_hash = AssetCache::request_hash(PROVIDER, CLUSTER_REF_COND, request)?;

        if !mode.is_live() {
            let cache = AssetCache::new(&self.cache_root);
            let response = MusicResult {
                provider: PROVIDER.into(),
                audio_path: cache.asset_path(PROVIDER, &request_hash, "mp3"),
                audio_bytes: 0,
                mime: "audio/mpeg".into(),
                model_variant: DEFAULT_VARIANT.into(),
                prompt_sent: request.prompt.clone(),
                duration_secs: request.duration_secs,
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

        Err(BackendError::Transport(
            "udio: API access not yet wired — partnership program required. \
             Use --backend elevenlabs for the commercially-safe default."
                .into(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fresh_cache() -> PathBuf {
        std::env::temp_dir().join(format!(
            "wavelet-udio-{}",
            AssetCache::request_hash("seed", "seed", &"x").unwrap()
        ))
    }

    #[test]
    fn estimate_scales_with_duration() {
        let a = UdioMusicAdapter::new(fresh_cache());
        let short = a.estimate_cost(&RefConditionedMusicRequest::new("x", 5.0));
        let long = a.estimate_cost(&RefConditionedMusicRequest::new("x", 30.0));
        assert!(long.cost_usd > short.cost_usd);
        assert!(long.explanation.contains("Udio"));
    }

    #[test]
    fn empty_prompt_rejected() {
        let a = UdioMusicAdapter::new(fresh_cache());
        let req = RefConditionedMusicRequest::new("   ", 5.0);
        assert!(matches!(
            a.generate(&req, RunMode::DryRun).unwrap_err(),
            BackendError::InvalidRequest(_)
        ));
    }

    #[test]
    fn dry_run_emits_request_shape() {
        let a = UdioMusicAdapter::new(fresh_cache());
        let req = RefConditionedMusicRequest::new("orchestral hero cue", 30.0);
        let out = a.generate(&req, RunMode::DryRun).unwrap();
        assert_eq!(out.mode, "dry-run");
        assert_eq!(out.response.provider, PROVIDER);
        assert_eq!(out.response.model_variant, DEFAULT_VARIANT);
        assert_eq!(out.response.audio_bytes, 0);
    }

    #[test]
    fn live_call_is_explicit_stub() {
        let a = UdioMusicAdapter::new(fresh_cache());
        let req = RefConditionedMusicRequest::new("ambient", 10.0);
        let err = a
            .generate(&req, RunMode::Live { max_cost_usd: 1.0 })
            .unwrap_err();
        match err {
            BackendError::Transport(msg) => assert!(msg.contains("not yet wired")),
            other => panic!("expected Transport(not yet wired), got {other:?}"),
        }
    }
}
