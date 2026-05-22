//! Gemini Flash TTS adapter — `VoiceIdTts` cluster impl on Google AI
//! Studio. Single key (`GOOGLE_API_KEY`) shared with Lyria + Veo +
//! Nano Banana.
//!
//! ## Wire format (verified 2026-05-20)
//!
//! ```text
//! POST https://generativelanguage.googleapis.com/v1beta/models/gemini-3.1-flash-tts-preview:generateContent?key=…
//! {
//!   "contents":[{"parts":[{"text":"Hello"}]}],
//!   "generationConfig":{
//!     "responseModalities":["AUDIO"],
//!     "speechConfig":{
//!       "voiceConfig":{"prebuiltVoiceConfig":{"voiceName":"Kore"}}
//!     }
//!   }
//! }
//! → candidates[0].content.parts[].inlineData.{ mimeType: "audio/l16; rate=24000; channels=1", data }
//! ```
//!
//! Output is raw 16-bit little-endian PCM at 24kHz mono. We wrap it
//! as a WAV (44-byte RIFF header + PCM payload) so the resulting
//! file plays in any standard audio tool.
//!
//! Voice names are Gemini's prebuilt voice catalogue (Kore, Puck,
//! Charon, Aoede, Fenrir, …). Default is `Kore`.

use serde::{Deserialize, Serialize};

use crate::backends::cache::{utc_now_iso8601, AssetCache, Manifest};
use crate::backends::tts::{
    check_budget, TtsRequest, TtsResult, VoiceIdTtsBackend, CLUSTER,
};
use crate::backends::{
    mode_label, BackendCallOutcome, BackendError, CostEstimate, RunMode,
};

use super::client::GoogleAiClient;

/// Model id.
pub const MODEL_GEMINI_TTS: &str = "gemini-3.1-flash-tts-preview";

/// Provider id stored in manifests + cache keys.
pub const PROVIDER: &str = "google-gemini-tts";

/// Default prebuilt voice when the caller passes no `voice_id` or
/// the placeholder ElevenLabs id.
pub const DEFAULT_VOICE: &str = "Kore";

/// Conservative per-character cost estimate (USD). Gemini's TTS
/// pricing isn't public for the preview tier; we mirror ElevenLabs
/// `eleven_multilingual_v2` so cost gates behave consistently and
/// users opting into Gemini TTS don't get over-charged budget gates.
pub const PRICE_PER_CHAR_USD: f32 = 0.0003;

/// PCM sample rate Gemini emits (Hz).
const PCM_RATE_HZ: u32 = 24_000;

/// PCM bits per sample.
const PCM_BITS: u16 = 16;

/// PCM channel count.
const PCM_CHANNELS: u16 = 1;

/// Estimated byte rate for the WAV-wrapped stream (used for the
/// `duration_secs_est` cheap metric).
const WAV_BYTES_PER_SEC: f32 =
    (PCM_RATE_HZ as f32) * (PCM_BITS as f32 / 8.0) * (PCM_CHANNELS as f32);

/// Gemini Flash TTS adapter.
#[derive(Debug, Clone)]
pub struct GeminiTtsAdapter {
    client: GoogleAiClient,
}

impl GeminiTtsAdapter {
    /// Build from a pre-constructed client.
    pub fn new(client: GoogleAiClient) -> Self {
        Self { client }
    }
}

impl VoiceIdTtsBackend for GeminiTtsAdapter {
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
                "{chars} chars × ${PRICE_PER_CHAR_USD:.4}/char (approximate, preview)"
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

        let voice = resolve_voice(&request.voice_id);
        let model = request
            .model
            .clone()
            .unwrap_or_else(|| MODEL_GEMINI_TTS.into());

        if !mode.is_live() {
            let response = TtsResult {
                provider: PROVIDER.into(),
                voice_id: voice.clone(),
                model,
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

        let body = build_body(&request.text, &voice);
        let parsed: GenerateContentResponse =
            self.client.post_sync(&model, "generateContent", &body)?;
        let (pcm, mime) = extract_audio(&parsed)?;
        let (rate, channels, bits) = parse_pcm_mime(&mime);
        let wav = wrap_pcm_as_wav(&pcm, rate, channels, bits);

        let audio_path = cache.write_asset(PROVIDER, &request_hash, "wav", &wav)?;
        let audio_bytes = wav.len() as u64;
        let duration_secs_est = audio_bytes as f32 / WAV_BYTES_PER_SEC;

        let result = TtsResult {
            provider: PROVIDER.into(),
            voice_id: voice,
            model: model.clone(),
            audio_path: audio_path.clone(),
            audio_bytes,
            duration_secs_est,
            mime: "audio/wav".into(),
        };

        let manifest = Manifest {
            version: 1,
            provider: PROVIDER.into(),
            cluster: CLUSTER.into(),
            request_hash: request_hash.clone(),
            request: serde_json::to_value(request)
                .map_err(|e| BackendError::Cache(format!("serialize request: {e}")))?,
            response: serde_json::to_value(&result)
                .map_err(|e| BackendError::Cache(format!("serialize response: {e}")))?,
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

fn resolve_voice(requested: &str) -> String {
    let trimmed = requested.trim();
    // ElevenLabs voice ids are 20-char alphanumeric handles. If we
    // see one of those (or empty), fall back to a Gemini prebuilt.
    if trimmed.is_empty() || trimmed.len() >= 16 && trimmed.chars().all(|c| c.is_ascii_alphanumeric()) {
        DEFAULT_VOICE.into()
    } else {
        trimmed.into()
    }
}

fn build_body(text: &str, voice: &str) -> GenerateContentBody {
    GenerateContentBody {
        contents: vec![Content {
            parts: vec![Part { text: text.into() }],
        }],
        generation_config: GenerationConfig {
            response_modalities: vec!["AUDIO".into()],
            speech_config: SpeechConfig {
                voice_config: VoiceConfig {
                    prebuilt_voice_config: PrebuiltVoiceConfig {
                        voice_name: voice.into(),
                    },
                },
            },
        },
    }
}

fn extract_audio(resp: &GenerateContentResponse) -> Result<(Vec<u8>, String), BackendError> {
    use crate::backends::util::base64_decode;
    let candidate = resp
        .candidates
        .first()
        .ok_or_else(|| BackendError::Decode("gemini-tts: empty candidates".into()))?;
    let mut bytes: Vec<u8> = Vec::new();
    let mut mime: Option<String> = None;
    for part in &candidate.content.parts {
        if let Some(inline) = &part.inline_data {
            if inline.mime_type.starts_with("audio/") {
                let chunk = base64_decode(&inline.data).map_err(|e| {
                    BackendError::Decode(format!("gemini-tts decode audio: {e}"))
                })?;
                bytes.extend_from_slice(&chunk);
                if mime.is_none() {
                    mime = Some(inline.mime_type.clone());
                }
            }
        }
    }
    if bytes.is_empty() {
        return Err(BackendError::Decode(
            "gemini-tts: no audio inlineData parts in response".into(),
        ));
    }
    Ok((bytes, mime.unwrap_or_else(|| "audio/l16; rate=24000; channels=1".into())))
}

/// Parse `audio/l16; rate=24000; channels=1` → `(24000, 1, 16)`.
/// Returns sensible defaults for fields the header omits.
fn parse_pcm_mime(mime: &str) -> (u32, u16, u16) {
    let mut rate = PCM_RATE_HZ;
    let mut channels = PCM_CHANNELS;
    let bits: u16 = if mime.contains("l24") {
        24
    } else if mime.contains("l8") {
        8
    } else {
        PCM_BITS
    };
    for part in mime.split(';') {
        let part = part.trim();
        if let Some(v) = part.strip_prefix("rate=") {
            if let Ok(r) = v.parse() {
                rate = r;
            }
        } else if let Some(v) = part.strip_prefix("channels=") {
            if let Ok(c) = v.parse() {
                channels = c;
            }
        }
    }
    (rate, channels, bits)
}

/// Wrap raw PCM bytes in a minimal RIFF/WAVE header.
fn wrap_pcm_as_wav(pcm: &[u8], rate: u32, channels: u16, bits: u16) -> Vec<u8> {
    let byte_rate = rate * (channels as u32) * (bits as u32 / 8);
    let block_align = channels * (bits / 8);
    let data_len = pcm.len() as u32;
    let riff_len = 36 + data_len;

    let mut out = Vec::with_capacity(44 + pcm.len());
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&riff_len.to_le_bytes());
    out.extend_from_slice(b"WAVE");
    out.extend_from_slice(b"fmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&channels.to_le_bytes());
    out.extend_from_slice(&rate.to_le_bytes());
    out.extend_from_slice(&byte_rate.to_le_bytes());
    out.extend_from_slice(&block_align.to_le_bytes());
    out.extend_from_slice(&bits.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    out.extend_from_slice(pcm);
    out
}

#[derive(Debug, Serialize)]
struct GenerateContentBody {
    contents: Vec<Content>,
    #[serde(rename = "generationConfig")]
    generation_config: GenerationConfig,
}

#[derive(Debug, Serialize)]
struct Content {
    parts: Vec<Part>,
}

#[derive(Debug, Serialize)]
struct Part {
    text: String,
}

#[derive(Debug, Serialize)]
struct GenerationConfig {
    #[serde(rename = "responseModalities")]
    response_modalities: Vec<String>,
    #[serde(rename = "speechConfig")]
    speech_config: SpeechConfig,
}

#[derive(Debug, Serialize)]
struct SpeechConfig {
    #[serde(rename = "voiceConfig")]
    voice_config: VoiceConfig,
}

#[derive(Debug, Serialize)]
struct VoiceConfig {
    #[serde(rename = "prebuiltVoiceConfig")]
    prebuilt_voice_config: PrebuiltVoiceConfig,
}

#[derive(Debug, Serialize)]
struct PrebuiltVoiceConfig {
    #[serde(rename = "voiceName")]
    voice_name: String,
}

#[derive(Debug, Deserialize)]
struct GenerateContentResponse {
    #[serde(default)]
    candidates: Vec<Candidate>,
}

#[derive(Debug, Deserialize)]
struct Candidate {
    content: CandidateContent,
}

#[derive(Debug, Deserialize)]
struct CandidateContent {
    #[serde(default)]
    parts: Vec<ResponsePart>,
}

#[derive(Debug, Deserialize)]
struct ResponsePart {
    #[serde(default, rename = "inlineData", alias = "inline_data")]
    inline_data: Option<InlineData>,
}

#[derive(Debug, Deserialize)]
struct InlineData {
    #[serde(rename = "mimeType", alias = "mime_type")]
    mime_type: String,
    data: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fresh_cache() -> std::path::PathBuf {
        let tmp = std::env::temp_dir().join(format!(
            "wavelet-gemini-tts-{}",
            AssetCache::request_hash("seed", "seed", &"x").unwrap()
        ));
        let _ = std::fs::remove_dir_all(&tmp);
        tmp
    }

    fn stub_client() -> GoogleAiClient {
        GoogleAiClient::with_key("test-key", fresh_cache())
    }

    #[test]
    fn empty_text_rejected() {
        let adapter = GeminiTtsAdapter::new(stub_client());
        let req = TtsRequest::new("   ", "Kore");
        assert!(matches!(
            adapter.synthesize(&req, RunMode::DryRun).unwrap_err(),
            BackendError::InvalidRequest(_)
        ));
    }

    #[test]
    fn dry_run_emits_request_shape() {
        let adapter = GeminiTtsAdapter::new(stub_client());
        let req = TtsRequest::new("Hello world", "Kore");
        let out = adapter.synthesize(&req, RunMode::DryRun).unwrap();
        assert_eq!(out.mode, "dry-run");
        assert_eq!(out.response.provider, PROVIDER);
        assert_eq!(out.response.audio_bytes, 0);
        assert_eq!(out.response.voice_id, "Kore");
    }

    #[test]
    fn elevenlabs_style_voice_falls_back_to_default() {
        // The CLI defaults `--voice` to an ElevenLabs id when the user
        // doesn't override. We auto-swap that for a Gemini prebuilt.
        assert_eq!(resolve_voice("21m00Tcm4TlvDq8ikWAM"), DEFAULT_VOICE);
        assert_eq!(resolve_voice(""), DEFAULT_VOICE);
        assert_eq!(resolve_voice("Kore"), "Kore");
        assert_eq!(resolve_voice("Puck"), "Puck");
    }

    #[test]
    fn body_shape_includes_response_modalities_and_voice() {
        let body = build_body("Hello", "Kore");
        let v = serde_json::to_value(&body).unwrap();
        assert_eq!(v["contents"][0]["parts"][0]["text"], "Hello");
        assert_eq!(v["generationConfig"]["responseModalities"][0], "AUDIO");
        assert_eq!(
            v["generationConfig"]["speechConfig"]["voiceConfig"]["prebuiltVoiceConfig"]
                ["voiceName"],
            "Kore"
        );
    }

    #[test]
    fn parse_pcm_mime_extracts_rate_channels_bits() {
        assert_eq!(
            parse_pcm_mime("audio/l16; rate=24000; channels=1"),
            (24000, 1, 16)
        );
        assert_eq!(parse_pcm_mime("audio/l24; rate=48000"), (48000, 1, 24));
        assert_eq!(parse_pcm_mime("audio/l16"), (PCM_RATE_HZ, PCM_CHANNELS, 16));
    }

    #[test]
    fn wav_wrapper_has_riff_header_and_payload() {
        let pcm = vec![1u8, 2, 3, 4, 5, 6, 7, 8];
        let wav = wrap_pcm_as_wav(&pcm, 24000, 1, 16);
        assert_eq!(&wav[0..4], b"RIFF");
        assert_eq!(&wav[8..12], b"WAVE");
        assert_eq!(&wav[12..16], b"fmt ");
        assert_eq!(&wav[36..40], b"data");
        assert_eq!(&wav[44..], &pcm[..]);
        assert_eq!(wav.len(), 44 + pcm.len());
    }

    #[test]
    fn extract_audio_handles_inline_data() {
        let body = r#"{
            "candidates": [{
                "content": {
                    "parts": [
                        { "inlineData": { "mimeType": "audio/l16; rate=24000; channels=1", "data": "aGVsbG8=" } }
                    ]
                }
            }]
        }"#;
        let parsed: GenerateContentResponse = serde_json::from_str(body).unwrap();
        let (bytes, mime) = extract_audio(&parsed).unwrap();
        assert_eq!(bytes, b"hello");
        assert!(mime.starts_with("audio/l16"));
    }
}
