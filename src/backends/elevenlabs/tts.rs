//! ElevenLabs Text-to-Speech adapter.
//!
//! Wraps `POST /v1/text-to-speech/{voice_id}`:
//! - Body: `{ text, model_id, voice_settings: { stability, similarity_boost, style, use_speaker_boost } }`
//! - Headers: `xi-api-key`, `Content-Type: application/json`, `Accept: audio/mpeg`
//! - Response: binary MP3 (audio/mpeg)
//!
//! Cost model: ElevenLabs bills per character. The multilingual v2
//! tier is approximately $0.30 per 1000 characters — we use that as
//! the estimate. Tier-specific overrides could come from a flag later;
//! for now the estimate is conservative-but-cheap.

use crate::backends::cache::{utc_now_iso8601, Manifest};
use crate::backends::elevenlabs::{truncate_body, ElevenLabsClient, API_BASE, PROVIDER};
use crate::backends::tts::{
    check_budget, TtsRequest, TtsResult, VoiceIdTtsBackend, CLUSTER,
};
use crate::backends::{
    cache::AssetCache, mode_label, BackendCallOutcome, BackendError, CostEstimate, RunMode,
};
use serde::Serialize;

/// Default model id when the caller doesn't override.
pub const DEFAULT_MODEL: &str = "eleven_multilingual_v2";

/// Estimated USD price per character for the default model. Used to
/// produce the `CostEstimate.cost_usd` the CLI gate checks.
pub const PRICE_PER_CHAR_USD: f32 = 0.0003;

/// Estimated byte rate (bytes per second) for the returned MP3 stream.
/// ElevenLabs returns ~24 kbps MP3 by default → ~3000 bytes/sec.
const ESTIMATED_BYTES_PER_SEC: f32 = 3000.0;

/// ElevenLabs TTS adapter. Cloned cheaply from a shared
/// `ElevenLabsClient`.
#[derive(Debug, Clone)]
pub struct ElevenLabsTtsAdapter {
    client: ElevenLabsClient,
}

impl ElevenLabsTtsAdapter {
    /// Build from a pre-constructed client.
    pub fn new(client: ElevenLabsClient) -> Self {
        Self { client }
    }
}

impl VoiceIdTtsBackend for ElevenLabsTtsAdapter {
    fn name(&self) -> &'static str {
        PROVIDER
    }

    fn estimate_cost(&self, request: &TtsRequest) -> CostEstimate {
        let chars = request.text.chars().count();
        let cost_usd = chars as f32 * PRICE_PER_CHAR_USD;
        CostEstimate {
            provider: PROVIDER.into(),
            cost_usd,
            explanation: format!(
                "{} chars × ${PRICE_PER_CHAR_USD:.4}/char (approximate)",
                chars
            ),
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
        if request.voice_id.trim().is_empty() {
            return Err(BackendError::InvalidRequest("voice_id is empty".into()));
        }

        let estimate = self.estimate_cost(request);
        check_budget(&estimate, mode)?;

        let request_hash = AssetCache::request_hash(PROVIDER, CLUSTER, request)?;
        let cache = self.client.cache();

        // Cache hit short-circuits the network.
        if let Some(manifest) = cache.hit(PROVIDER, &request_hash)? {
            let response: TtsResult =
                serde_json::from_value(manifest.response.clone()).map_err(|e| {
                    BackendError::Cache(format!("decode cached response: {e}"))
                })?;
            return Ok(BackendCallOutcome {
                response,
                provider: PROVIDER.into(),
                request_hash,
                cached: true,
                cost_estimate_usd: 0.0,
                mode: mode_label(mode),
            });
        }

        // Dry-run: synthesize an empty response describing the request.
        if !mode.is_live() {
            let model = request.model.clone().unwrap_or_else(|| DEFAULT_MODEL.into());
            let response = TtsResult {
                provider: PROVIDER.into(),
                voice_id: request.voice_id.clone(),
                model,
                audio_path: cache.asset_path(PROVIDER, &request_hash, "mp3"),
                audio_bytes: 0,
                duration_secs_est: 0.0,
                mime: "audio/mpeg".into(),
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

        // Live call.
        let model = request.model.clone().unwrap_or_else(|| DEFAULT_MODEL.into());
        let url = build_url(&request.voice_id);
        let body = build_body(request, &model);
        let resp = ureq::post(&url)
            .set("xi-api-key", self.client.api_key())
            .set("Accept", "audio/mpeg")
            .set("Content-Type", "application/json")
            .send_string(&serde_json::to_string(&body).map_err(|e| {
                BackendError::InvalidRequest(format!("serialize body: {e}"))
            })?);

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

        let mut buf: Vec<u8> = Vec::with_capacity(64 * 1024);
        use std::io::Read;
        response
            .into_reader()
            .read_to_end(&mut buf)
            .map_err(|e| BackendError::Transport(format!("read body: {e}")))?;

        if buf.is_empty() {
            return Err(BackendError::Decode("empty audio response".into()));
        }

        let audio_path = cache.write_asset(PROVIDER, &request_hash, "mp3", &buf)?;
        let audio_bytes = buf.len() as u64;
        let duration_secs_est = audio_bytes as f32 / ESTIMATED_BYTES_PER_SEC;

        let result = TtsResult {
            provider: PROVIDER.into(),
            voice_id: request.voice_id.clone(),
            model: model.clone(),
            audio_path: audio_path.clone(),
            audio_bytes,
            duration_secs_est,
            mime: "audio/mpeg".into(),
        };

        // Persist the manifest so subsequent identical requests hit cache.
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

fn build_url(voice_id: &str) -> String {
    format!("{API_BASE}/text-to-speech/{voice_id}")
}

#[derive(Debug, Serialize)]
struct TtsBody<'a> {
    text: &'a str,
    model_id: &'a str,
    voice_settings: VoiceSettings,
}

#[derive(Debug, Serialize)]
struct VoiceSettings {
    stability: f32,
    similarity_boost: f32,
    style: f32,
    use_speaker_boost: bool,
}

fn build_body<'a>(request: &'a TtsRequest, model: &'a str) -> TtsBody<'a> {
    TtsBody {
        text: &request.text,
        model_id: model,
        voice_settings: VoiceSettings {
            stability: request.stability.unwrap_or(0.5),
            similarity_boost: request.similarity_boost.unwrap_or(0.75),
            style: request.style.unwrap_or(0.0),
            use_speaker_boost: true,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backends::elevenlabs::voices;

    fn fresh_cache() -> std::path::PathBuf {
        let tmp = std::env::temp_dir().join(format!(
            "wavelet-eleven-tts-{}",
            AssetCache::request_hash("seed", "seed", &"x").unwrap()
        ));
        let _ = std::fs::remove_dir_all(&tmp);
        tmp
    }

    #[test]
    fn url_includes_voice_id() {
        let url = build_url(voices::RACHEL);
        assert!(url.ends_with(voices::RACHEL));
        assert!(url.starts_with("https://api.elevenlabs.io/v1/text-to-speech/"));
    }

    #[test]
    fn body_carries_defaults_when_unspecified() {
        let req = TtsRequest::new("hi", voices::RACHEL);
        let body = build_body(&req, DEFAULT_MODEL);
        let json = serde_json::to_value(&body).unwrap();
        assert_eq!(json["text"], "hi");
        assert_eq!(json["model_id"], DEFAULT_MODEL);
        assert_eq!(json["voice_settings"]["stability"].as_f64().unwrap(), 0.5);
        assert_eq!(
            json["voice_settings"]["similarity_boost"].as_f64().unwrap(),
            0.75
        );
        assert_eq!(json["voice_settings"]["style"].as_f64().unwrap(), 0.0);
    }

    #[test]
    fn body_overrides_with_caller_settings() {
        let mut req = TtsRequest::new("hi", voices::RACHEL);
        req.stability = Some(0.2);
        req.similarity_boost = Some(0.9);
        req.style = Some(0.4);
        let body = build_body(&req, "custom-model");
        let json = serde_json::to_value(&body).unwrap();
        assert_eq!(json["model_id"], "custom-model");
        assert!((json["voice_settings"]["stability"].as_f64().unwrap() - 0.2).abs() < 1e-6);
        assert!(
            (json["voice_settings"]["similarity_boost"].as_f64().unwrap() - 0.9).abs() < 1e-6
        );
        assert!((json["voice_settings"]["style"].as_f64().unwrap() - 0.4).abs() < 1e-6);
    }

    #[test]
    fn cost_estimate_scales_with_text_length() {
        let client = ElevenLabsClient::with_key("test", fresh_cache());
        let adapter = ElevenLabsTtsAdapter::new(client);
        let short = adapter.estimate_cost(&TtsRequest::new("hi", voices::RACHEL));
        let long = adapter.estimate_cost(&TtsRequest::new("x".repeat(1000), voices::RACHEL));
        assert!(long.cost_usd > short.cost_usd);
        // 1000 chars × $0.0003 = $0.30
        assert!((long.cost_usd - 0.30).abs() < 0.01);
    }

    #[test]
    fn empty_text_is_invalid_request() {
        let client = ElevenLabsClient::with_key("test", fresh_cache());
        let adapter = ElevenLabsTtsAdapter::new(client);
        let req = TtsRequest::new("   ", voices::RACHEL);
        let err = adapter.synthesize(&req, RunMode::DryRun).unwrap_err();
        assert!(matches!(err, BackendError::InvalidRequest(_)));
    }

    #[test]
    fn empty_voice_id_is_invalid_request() {
        let client = ElevenLabsClient::with_key("test", fresh_cache());
        let adapter = ElevenLabsTtsAdapter::new(client);
        let req = TtsRequest::new("hi", "");
        let err = adapter.synthesize(&req, RunMode::DryRun).unwrap_err();
        assert!(matches!(err, BackendError::InvalidRequest(_)));
    }

    #[test]
    fn dry_run_emits_request_shape_without_network() {
        let client = ElevenLabsClient::with_key("test", fresh_cache());
        let adapter = ElevenLabsTtsAdapter::new(client.clone());
        let req = TtsRequest::new("Hello world.", voices::RACHEL);
        let out = adapter.synthesize(&req, RunMode::DryRun).unwrap();
        assert_eq!(out.mode, "dry-run");
        assert_eq!(out.response.provider, "elevenlabs");
        assert_eq!(out.response.voice_id, voices::RACHEL);
        assert_eq!(out.response.model, DEFAULT_MODEL);
        assert_eq!(out.response.audio_bytes, 0);
        assert!(!out.cached);
        // Cache must remain empty after dry-run.
        assert!(client
            .cache()
            .hit(PROVIDER, &out.request_hash)
            .unwrap()
            .is_none());
    }

    #[test]
    fn over_budget_request_is_rejected() {
        let client = ElevenLabsClient::with_key("test", fresh_cache());
        let adapter = ElevenLabsTtsAdapter::new(client);
        // 1000-char request at $0.30 estimate; budget of $0.01 should reject.
        let req = TtsRequest::new("x".repeat(1000), voices::RACHEL);
        let err = adapter
            .synthesize(&req, RunMode::Live { max_cost_usd: 0.01 })
            .unwrap_err();
        match err {
            BackendError::OverBudget { estimate, budget } => {
                assert!(estimate > 0.20);
                assert!((budget - 0.01).abs() < 1e-6);
            }
            other => panic!("expected OverBudget, got {other:?}"),
        }
    }

    #[test]
    fn dry_run_bypasses_budget_regardless_of_estimate() {
        let client = ElevenLabsClient::with_key("test", fresh_cache());
        let adapter = ElevenLabsTtsAdapter::new(client);
        // 1000-char request has a $0.30 estimate. Dry-run still passes
        // because the budget gate is bypassed when there's no spend.
        let req = TtsRequest::new("x".repeat(1000), voices::RACHEL);
        let out = adapter.synthesize(&req, RunMode::DryRun).unwrap();
        assert_eq!(out.mode, "dry-run");
        assert!((out.cost_estimate_usd - 0.30).abs() < 0.01);
    }
}
