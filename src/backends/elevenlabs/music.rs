//! ElevenLabs Music adapter — `RefConditionedMusicGen` cluster, the
//! commercially-safe default.
//!
//! ## Why this exists
//!
//! MusicGen (Fal `fal-ai/musicgen`) has pending litigation around the
//! origin of its training data — commercial deliverables built on its
//! output are not legally safe. ElevenLabs Music licenses its training
//! catalog through Merlin + Kobalt (the two largest music-industry
//! rights aggregators), so output is commercial-safe by construction.
//!
//! ## API shape (probed against `api.elevenlabs.io/openapi.json`)
//!
//! - Endpoint: `POST https://api.elevenlabs.io/v1/music`
//! - Headers: `xi-api-key: <key>`, `Content-Type: application/json`
//! - Body: `{ prompt, music_length_ms (3000..=600000), model_id, seed?, force_instrumental? }`
//! - Response: raw audio bytes (`audio/*` — typically MP3)
//!
//! ## Key permissions
//!
//! Live calls require a key with the `music_generation` permission.
//! TTS-only keys (the `text_to_speech` scope) return HTTP 401
//! `missing_permissions`. Dry-run + cache hits work without a key.
//!
//! ## Cost
//!
//! ElevenLabs Music is roughly $0.30 per generated minute on the
//! standard plans — we estimate at $0.005/sec to match. Real billing
//! varies by tier; the manifest's `cost_estimate_usd` reflects the
//! conservative wire estimate.

use crate::backends::cache::{utc_now_iso8601, AssetCache, Manifest};
use crate::backends::util::pick_audio_ext_from_mime;
use crate::backends::elevenlabs::{truncate_body, ElevenLabsClient, API_BASE, PROVIDER as VENDOR};
use crate::backends::music::{
    MusicResult, RefConditionedMusicGenBackend, RefConditionedMusicRequest, CLUSTER_REF_COND,
};
use crate::backends::{
    check_budget, mode_label, BackendCallOutcome, BackendError, CostEstimate, RunMode,
};
use serde::{Deserialize, Serialize};

/// Provider id stored in manifests + cache keys. Distinct from the
/// bare `elevenlabs` vendor id so the cache namespaces music output
/// separately from TTS output.
pub const PROVIDER: &str = "elevenlabs-music";

/// Default model id. ElevenLabs Music currently exposes a single
/// production model (`music_v1`).
pub const DEFAULT_MODEL: &str = "music_v1";

/// Conservative per-second cost estimate (USD). Real billing varies
/// by tier; this matches MusicGen's per-second rate so the agent's
/// budget gates carry over unchanged.
pub const PRICE_PER_SEC_USD: f32 = 0.005;

/// Minimum supported duration (ms) — enforced by the wire API.
pub const MIN_MS: u32 = 3000;

/// Maximum supported duration (ms) — enforced by the wire API.
pub const MAX_MS: u32 = 600_000;

/// ElevenLabs Music adapter. Cloned cheaply from a shared
/// `ElevenLabsClient`.
#[derive(Debug, Clone)]
pub struct ElevenLabsMusicAdapter {
    client: ElevenLabsClient,
}

impl ElevenLabsMusicAdapter {
    /// Build from a pre-constructed client.
    pub fn new(client: ElevenLabsClient) -> Self {
        Self { client }
    }
}

impl RefConditionedMusicGenBackend for ElevenLabsMusicAdapter {
    fn name(&self) -> &'static str {
        PROVIDER
    }

    fn estimate_cost(&self, request: &RefConditionedMusicRequest) -> CostEstimate {
        let cost = request.duration_secs.max(0.0) * PRICE_PER_SEC_USD;
        CostEstimate {
            provider: PROVIDER.into(),
            cost_usd: cost,
            explanation: format!(
                "{:.1}s × ${PRICE_PER_SEC_USD:.4}/sec (Merlin+Kobalt-licensed)",
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
        let cache = self.client.cache();
        let model = request
            .model_variant
            .clone()
            .unwrap_or_else(|| DEFAULT_MODEL.into());

        if let Some(manifest) = cache.hit(PROVIDER, &request_hash)? {
            let response: MusicResult = serde_json::from_value(manifest.response.clone())
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
            let response = MusicResult {
                provider: PROVIDER.into(),
                audio_path: cache.asset_path(PROVIDER, &request_hash, "mp3"),
                audio_bytes: 0,
                mime: "audio/mpeg".into(),
                model_variant: model,
                prompt_sent: build_prompt(request),
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

        let body = build_body(request, &model);
        let url = format!("{API_BASE}/music");
        let body_json = serde_json::to_string(&body)
            .map_err(|e| BackendError::InvalidRequest(format!("serialize body: {e}")))?;

        let resp = ureq::post(&url)
            .set("xi-api-key", self.client.api_key())
            .set("Accept", "audio/*")
            .set("Content-Type", "application/json")
            .send_string(&body_json);

        let response = match resp {
            Ok(r) => r,
            Err(ureq::Error::Status(status, response)) => {
                let body = response.into_string().unwrap_or_default();
                return Err(BackendError::HttpStatus {
                    status,
                    body: truncate_body(&body),
                });
            }
            Err(e) => return Err(BackendError::Transport(e.to_string())),
        };

        let mime = response
            .header("content-type")
            .unwrap_or("audio/mpeg")
            .to_string();
        let ext = pick_audio_ext_from_mime(&mime);

        let mut buf: Vec<u8> = Vec::with_capacity(256 * 1024);
        use std::io::Read;
        response
            .into_reader()
            .read_to_end(&mut buf)
            .map_err(|e| BackendError::Transport(format!("read body: {e}")))?;
        if buf.is_empty() {
            return Err(BackendError::Decode("empty audio response".into()));
        }

        let audio_path = cache.write_asset(PROVIDER, &request_hash, ext, &buf)?;
        let result = MusicResult {
            provider: PROVIDER.into(),
            audio_path: audio_path.clone(),
            audio_bytes: buf.len() as u64,
            mime,
            model_variant: model,
            prompt_sent: body.prompt.clone(),
            duration_secs: request.duration_secs,
        };

        let manifest = Manifest {
            version: 1,
            provider: PROVIDER.into(),
            cluster: CLUSTER_REF_COND.into(),
            request_hash: request_hash.clone(),
            request: serde_json::to_value(request).map_err(|e| {
                BackendError::Cache(format!("serialize request for cache: {e}"))
            })?,
            response: serde_json::to_value(&result).map_err(|e| {
                BackendError::Cache(format!("serialize response for cache: {e}"))
            })?,
            cost_estimate_usd: estimate.cost_usd,
            asset_path: Some(audio_path.display().to_string()),
            created_at: utc_now_iso8601(),
        };
        cache.store(&manifest)?;

        // Reference the vendor id so dead-code lints stay happy when only
        // the music submodule is touched.
        let _ = VENDOR;

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

/// Compose the prompt EL Music receives. BPM is encoded as free text
/// (EL Music respects "110 bpm" in the prompt) since the wire body has
/// no structured BPM field.
fn build_prompt(request: &RefConditionedMusicRequest) -> String {
    match request.bpm {
        Some(bpm) if !request.prompt.to_ascii_lowercase().contains("bpm") => {
            format!("{} ({:.0} bpm)", request.prompt, bpm)
        }
        _ => request.prompt.clone(),
    }
}

/// Clamp the requested duration into the wire-API legal range and
/// convert to milliseconds. The wire body uses an integer ms field.
fn clamp_ms(duration_secs: f32) -> u32 {
    let ms = (duration_secs * 1000.0).round() as i64;
    ms.clamp(MIN_MS as i64, MAX_MS as i64) as u32
}

fn build_body(request: &RefConditionedMusicRequest, model: &str) -> MusicBody {
    MusicBody {
        prompt: build_prompt(request),
        model_id: model.to_string(),
        music_length_ms: clamp_ms(request.duration_secs),
        seed: request.seed.map(|s| (s as u64 % (i32::MAX as u64)) as u32),
        force_instrumental: true,
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct MusicBody {
    prompt: String,
    model_id: String,
    music_length_ms: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    seed: Option<u32>,
    force_instrumental: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fresh_cache() -> std::path::PathBuf {
        let tmp = std::env::temp_dir().join(format!(
            "wavelet-eleven-music-{}",
            AssetCache::request_hash("seed", "seed", &"x").unwrap()
        ));
        let _ = std::fs::remove_dir_all(&tmp);
        tmp
    }

    #[test]
    fn provider_id_is_namespaced() {
        assert_eq!(PROVIDER, "elevenlabs-music");
    }

    #[test]
    fn estimate_scales_with_duration() {
        let client = ElevenLabsClient::with_key("test", fresh_cache());
        let adapter = ElevenLabsMusicAdapter::new(client);
        let short = adapter.estimate_cost(&RefConditionedMusicRequest::new("x", 5.0));
        let long = adapter.estimate_cost(&RefConditionedMusicRequest::new("x", 30.0));
        assert!(long.cost_usd > short.cost_usd);
        assert!((long.cost_usd - 0.15).abs() < 0.01);
        assert!(long.explanation.contains("Merlin"));
    }

    #[test]
    fn empty_prompt_rejected() {
        let client = ElevenLabsClient::with_key("test", fresh_cache());
        let adapter = ElevenLabsMusicAdapter::new(client);
        let req = RefConditionedMusicRequest::new("   ", 5.0);
        assert!(matches!(
            adapter.generate(&req, RunMode::DryRun).unwrap_err(),
            BackendError::InvalidRequest(_)
        ));
    }

    #[test]
    fn zero_duration_rejected() {
        let client = ElevenLabsClient::with_key("test", fresh_cache());
        let adapter = ElevenLabsMusicAdapter::new(client);
        let req = RefConditionedMusicRequest::new("ambient", 0.0);
        assert!(matches!(
            adapter.generate(&req, RunMode::DryRun).unwrap_err(),
            BackendError::InvalidRequest(_)
        ));
    }

    #[test]
    fn dry_run_emits_request_shape_no_network() {
        let client = ElevenLabsClient::with_key("test", fresh_cache());
        let adapter = ElevenLabsMusicAdapter::new(client.clone());
        let mut req = RefConditionedMusicRequest::new("cinematic strings", 8.0);
        req.bpm = Some(90.0);
        let out = adapter.generate(&req, RunMode::DryRun).unwrap();
        assert_eq!(out.mode, "dry-run");
        assert_eq!(out.response.provider, PROVIDER);
        assert_eq!(out.response.audio_bytes, 0);
        assert_eq!(out.response.model_variant, DEFAULT_MODEL);
        assert!(out.response.prompt_sent.contains("90 bpm"));
        assert!(client
            .cache()
            .hit(PROVIDER, &out.request_hash)
            .unwrap()
            .is_none());
    }

    #[test]
    fn build_prompt_adds_bpm_when_missing() {
        let mut req = RefConditionedMusicRequest::new("ambient strings", 5.0);
        req.bpm = Some(110.0);
        assert_eq!(build_prompt(&req), "ambient strings (110 bpm)".to_string());
    }

    #[test]
    fn build_prompt_preserves_existing_bpm() {
        let mut req = RefConditionedMusicRequest::new("driving 130 BPM techno", 5.0);
        req.bpm = Some(120.0);
        assert_eq!(build_prompt(&req), "driving 130 BPM techno");
    }

    #[test]
    fn clamp_ms_honors_wire_bounds() {
        assert_eq!(clamp_ms(1.0), MIN_MS);
        assert_eq!(clamp_ms(10.0), 10_000);
        assert_eq!(clamp_ms(900.0), MAX_MS);
    }

    #[test]
    fn body_carries_required_fields() {
        let mut req = RefConditionedMusicRequest::new("ambient", 10.0);
        req.bpm = Some(95.0);
        req.seed = Some(42);
        let body = build_body(&req, DEFAULT_MODEL);
        let json = serde_json::to_value(&body).unwrap();
        assert_eq!(json["model_id"], DEFAULT_MODEL);
        assert_eq!(json["music_length_ms"].as_u64().unwrap(), 10_000);
        assert!(json["prompt"].as_str().unwrap().contains("95 bpm"));
        assert_eq!(json["seed"].as_u64().unwrap(), 42);
        assert_eq!(json["force_instrumental"].as_bool().unwrap(), true);
    }

    #[test]
    fn body_omits_seed_when_unset() {
        let req = RefConditionedMusicRequest::new("ambient", 10.0);
        let body = build_body(&req, DEFAULT_MODEL);
        let json = serde_json::to_value(&body).unwrap();
        assert!(json.get("seed").is_none());
    }

    #[test]
    fn over_budget_request_is_rejected() {
        let client = ElevenLabsClient::with_key("test", fresh_cache());
        let adapter = ElevenLabsMusicAdapter::new(client);
        let req = RefConditionedMusicRequest::new("ambient", 60.0);
        let err = adapter
            .generate(&req, RunMode::Live { max_cost_usd: 0.01 })
            .unwrap_err();
        match err {
            BackendError::OverBudget { estimate, budget } => {
                assert!(estimate > 0.20);
                assert!((budget - 0.01).abs() < 1e-6);
            }
            other => panic!("expected OverBudget, got {other:?}"),
        }
    }
}
