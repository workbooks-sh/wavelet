//! Fal Kokoro TTS adapter — `VoiceIdTts` cluster.
//!
//! Wraps `fal-ai/kokoro` — fast, open-weight TTS hosted on Fal.
//! Accepts `{prompt|text}` and returns `{audio.url}` (WAV).
//!
//! Cost: pay-per-call. Each gen yields one WAV under 100KB for short
//! lines. Conservative estimate: $0.005/call.

use crate::backends::cache::{utc_now_iso8601, AssetCache, Manifest};
use crate::backends::fal::FalClient;
use crate::backends::tts::{TtsRequest, TtsResult, VoiceIdTtsBackend, CLUSTER};
use crate::backends::{
    check_budget, mode_label, BackendCallOutcome, BackendError, CostEstimate, RunMode,
};
use serde::{Deserialize, Serialize};

/// Provider id.
pub const PROVIDER: &str = "fal-kokoro";

/// Fal model path.
pub const MODEL_PATH: &str = "fal-ai/kokoro";

/// Default voice id when caller doesn't override. Kokoro uses string
/// voice ids like `af_bella`, `af_nicole`, `am_adam`, `am_michael`, etc.
pub const DEFAULT_VOICE: &str = "af_nicole";

/// Per-call cost estimate (USD). Kokoro is one of the cheapest TTS
/// options on Fal — conservative ceiling.
pub const PRICE_PER_CALL_USD: f32 = 0.01;

/// Fal Kokoro adapter.
#[derive(Debug, Clone)]
pub struct FalKokoroAdapter {
    client: FalClient,
}

impl FalKokoroAdapter {
    /// Build from a pre-constructed client.
    pub fn new(client: FalClient) -> Self {
        Self { client }
    }
}

impl VoiceIdTtsBackend for FalKokoroAdapter {
    fn name(&self) -> &'static str {
        PROVIDER
    }

    fn estimate_cost(&self, _request: &TtsRequest) -> CostEstimate {
        CostEstimate {
            provider: PROVIDER.into(),
            cost_usd: PRICE_PER_CALL_USD,
            explanation: format!("${PRICE_PER_CALL_USD:.4}/call (conservative)"),
        }
    }

    fn synthesize(
        &self,
        request: &TtsRequest,
        mode: RunMode,
    ) -> Result<BackendCallOutcome<TtsResult>, BackendError> {
        if request.text.trim().is_empty() {
            return Err(BackendError::InvalidRequest("text is empty".into()));
        }

        let estimate = self.estimate_cost(request);
        check_budget(&estimate, mode)?;

        let request_hash = AssetCache::request_hash(PROVIDER, CLUSTER, request)?;
        let cache = self.client.cache();

        if let Some(manifest) = cache.hit(PROVIDER, &request_hash)? {
            let response: TtsResult = serde_json::from_value(manifest.response.clone())
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
            let voice = if request.voice_id.is_empty() {
                DEFAULT_VOICE.to_string()
            } else {
                request.voice_id.clone()
            };
            let response = TtsResult {
                provider: PROVIDER.into(),
                voice_id: voice,
                model: request.model.clone().unwrap_or_else(|| "kokoro".into()),
                audio_path: cache.asset_path(PROVIDER, &request_hash, "wav"),
                audio_bytes: 0,
                duration_secs_est: 0.0,
                mime: "audio/wav".into(),
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

        // Live call. Kokoro accepts `prompt` or `text` — we send both
        // for compatibility with both Fal API revisions.
        let voice = if request.voice_id.is_empty() {
            DEFAULT_VOICE.to_string()
        } else {
            request.voice_id.clone()
        };
        let body = KokoroBody {
            prompt: request.text.clone(),
            text: request.text.clone(),
            voice: voice.clone(),
            speed: request.style.unwrap_or(1.0),
        };
        let parsed: KokoroResponse = self.client.post_sync(MODEL_PATH, &body)?;
        let audio_url = parsed.audio.url.clone();
        let audio_bytes_raw = self.client.fetch_asset(&audio_url)?;
        let audio_path = cache.write_asset(PROVIDER, &request_hash, "wav", &audio_bytes_raw)?;
        let audio_bytes = audio_bytes_raw.len() as u64;
        // Approximate: WAV is 16-bit stereo 24kHz = ~96 kB/s. Kokoro
        // emits 22kHz mono WAV around 44 kB/s.
        let duration_secs_est = audio_bytes as f32 / 44_000.0;

        let result = TtsResult {
            provider: PROVIDER.into(),
            voice_id: voice,
            model: request.model.clone().unwrap_or_else(|| "kokoro".into()),
            audio_path: audio_path.clone(),
            audio_bytes,
            duration_secs_est,
            mime: parsed
                .audio
                .content_type
                .unwrap_or_else(|| "audio/wav".into()),
        };

        let manifest = Manifest {
            version: 1,
            provider: PROVIDER.into(),
            cluster: CLUSTER.into(),
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
struct KokoroBody {
    /// Some Fal Kokoro revisions take `prompt`; send both for safety.
    prompt: String,
    text: String,
    voice: String,
    #[serde(skip_serializing_if = "is_unit_speed")]
    speed: f32,
}

fn is_unit_speed(s: &f32) -> bool {
    (*s - 1.0).abs() < 1e-6
}

#[derive(Debug, Deserialize)]
struct KokoroResponse {
    #[serde(alias = "audio_url")]
    audio: FalAudioFile,
}

#[derive(Debug, Deserialize)]
struct FalAudioFile {
    url: String,
    #[serde(default)]
    content_type: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fresh_cache() -> std::path::PathBuf {
        let tmp = std::env::temp_dir().join(format!(
            "wavelet-fal-kokoro-{}",
            AssetCache::request_hash("seed", "seed", &"x").unwrap()
        ));
        let _ = std::fs::remove_dir_all(&tmp);
        tmp
    }

    #[test]
    fn empty_text_rejected() {
        let client = FalClient::with_key("id:secret", fresh_cache());
        let adapter = FalKokoroAdapter::new(client);
        let req = TtsRequest::new("   ", "");
        assert!(matches!(
            adapter.synthesize(&req, RunMode::DryRun).unwrap_err(),
            BackendError::InvalidRequest(_)
        ));
    }

    #[test]
    fn cost_is_flat_per_call() {
        let client = FalClient::with_key("id:secret", fresh_cache());
        let adapter = FalKokoroAdapter::new(client);
        let a = adapter.estimate_cost(&TtsRequest::new("short", DEFAULT_VOICE));
        let b = adapter.estimate_cost(&TtsRequest::new("x".repeat(1000), DEFAULT_VOICE));
        assert!((a.cost_usd - b.cost_usd).abs() < 1e-6);
    }

    #[test]
    fn dry_run_emits_request_shape() {
        let client = FalClient::with_key("id:secret", fresh_cache());
        let adapter = FalKokoroAdapter::new(client.clone());
        let req = TtsRequest::new("Hello world.", DEFAULT_VOICE);
        let out = adapter.synthesize(&req, RunMode::DryRun).unwrap();
        assert_eq!(out.mode, "dry-run");
        assert_eq!(out.response.provider, PROVIDER);
        assert_eq!(out.response.voice_id, DEFAULT_VOICE);
        assert!(out.response.audio_path.to_string_lossy().ends_with(".wav"));
    }

    #[test]
    fn response_decodes_alias_field() {
        let body = r#"{
            "audio_url": {
                "url": "https://example.com/a.wav",
                "content_type": "audio/wav"
            }
        }"#;
        let parsed: KokoroResponse = serde_json::from_str(body).unwrap();
        assert_eq!(parsed.audio.url, "https://example.com/a.wav");
    }
}
