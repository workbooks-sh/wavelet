//! Fal Whisper adapter — `Captions` cluster.
//!
//! Wraps `fal-ai/whisper` with `chunk_level: "word"`. Returns per-word
//! `[start_s, end_s]` timestamps over the entire VO audio. The
//! optional `text` reference on the request is currently informational
//! only — Whisper transcribes the audio independently; if the caller
//! provides VO text we keep it in the manifest for auditing but do not
//! pass it to the model. (Fal's `prompt` parameter accepts up to 224
//! tokens of stylistic priming, not force-alignment input — applying
//! it changes the *generated* transcript, which is the opposite of
//! what we want.)
//!
//! Probed response shape (verified live, 2026-05-18):
//!
//! ```json
//! {
//!   "text": " the full transcript",
//!   "chunks": [
//!     { "timestamp": [0.89, 1.65], "text": " María.", "speaker": null }
//!   ],
//!   "inferred_languages": ["en"]
//! }
//! ```
//!
//! Cost: Fal advertises ~$0.0001/sec for whisper-large-v3-turbo, so a
//! 10-second VO ≈ $0.001. Per-call conservative ceiling: $0.05 (covers
//! 8+ minutes of audio).

use crate::backends::cache::{utc_now_iso8601, AssetCache, Manifest};
use crate::backends::captions::{
    CaptionsBackend, CaptionsRequest, CaptionsResult, WordTimestamp, CLUSTER,
};
use crate::backends::fal::FalClient;
use crate::backends::{
    check_budget, mode_label, BackendCallOutcome, BackendError, CostEstimate, RunMode,
};
use serde::{Deserialize, Serialize};

/// Provider id.
pub const PROVIDER: &str = "fal-whisper-words";

/// Fal model path.
pub const MODEL_PATH: &str = "fal-ai/whisper";

/// Conservative per-call ceiling — Whisper is cheap (~$0.0001/sec of
/// audio) but we cap the estimate so the budget gate stays meaningful.
pub const PRICE_PER_CALL_USD: f32 = 0.05;

/// Fal Whisper (word-chunk) adapter.
#[derive(Debug, Clone)]
pub struct FalWhisperWordsAdapter {
    client: FalClient,
}

impl FalWhisperWordsAdapter {
    /// Construct from a pre-built client.
    pub fn new(client: FalClient) -> Self {
        Self { client }
    }
}

impl CaptionsBackend for FalWhisperWordsAdapter {
    fn name(&self) -> &'static str {
        PROVIDER
    }

    fn estimate_cost(&self, _request: &CaptionsRequest) -> CostEstimate {
        CostEstimate {
            provider: PROVIDER.into(),
            cost_usd: PRICE_PER_CALL_USD,
            explanation: format!("${PRICE_PER_CALL_USD:.4}/call (conservative)"),
        }
    }

    fn captions(
        &self,
        request: &CaptionsRequest,
        mode: RunMode,
    ) -> Result<BackendCallOutcome<CaptionsResult>, BackendError> {
        if request.audio_url.trim().is_empty() {
            return Err(BackendError::InvalidRequest("audio_url is empty".into()));
        }

        let estimate = self.estimate_cost(request);
        check_budget(&estimate, mode)?;

        let request_hash = AssetCache::request_hash(PROVIDER, CLUSTER, request)?;
        let cache = self.client.cache();

        if let Some(manifest) = cache.hit(PROVIDER, &request_hash)? {
            let response: CaptionsResult = serde_json::from_value(manifest.response.clone())
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
            let response = CaptionsResult {
                provider: PROVIDER.into(),
                words: Vec::new(),
                total_ms: 0,
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

        // Resolve local paths into a fal-storage URL — Whisper rejects
        // `data:` audio URIs with 422 Unsupported, so for local files
        // we upload through fal-storage and use the returned public
        // URL. HTTPS URLs pass through unchanged.
        let resolved_audio = resolve_audio_input(&self.client, &request.audio_url)?;
        let body = WhisperBody {
            audio_url: resolved_audio,
            task: "transcribe",
            chunk_level: "word",
        };
        let parsed: WhisperResponse = self.client.post_sync(MODEL_PATH, &body)?;

        let words: Vec<WordTimestamp> = parsed
            .chunks
            .iter()
            .map(|c| WordTimestamp {
                word: c.text.trim().to_string(),
                start_ms: (c.timestamp.0 * 1000.0).round() as u32,
                end_ms: (c.timestamp.1 * 1000.0).round() as u32,
            })
            .filter(|w| !w.word.is_empty())
            .collect();
        let total_ms = words.last().map(|w| w.end_ms).unwrap_or(0);

        let response = CaptionsResult {
            provider: PROVIDER.into(),
            words,
            total_ms,
        };

        let manifest = Manifest {
            version: 1,
            provider: PROVIDER.into(),
            cluster: CLUSTER.into(),
            request_hash: request_hash.clone(),
            request: serde_json::to_value(request).map_err(|e| {
                BackendError::Cache(format!("serialize request for cache: {e}"))
            })?,
            response: serde_json::to_value(&response).map_err(|e| {
                BackendError::Cache(format!("serialize response for cache: {e}"))
            })?,
            cost_estimate_usd: estimate.cost_usd,
            asset_path: None,
            created_at: utc_now_iso8601(),
        };
        cache.store(&manifest)?;

        Ok(BackendCallOutcome {
            response,
            provider: PROVIDER.into(),
            request_hash,
            cached: false,
            cost_estimate_usd: estimate.cost_usd,
            mode: mode_label(mode),
        })
    }
}

/// If `input` is an `http(s)://` URL, return it unchanged. If it's a
/// `data:` URI, return unchanged too — though Whisper itself rejects
/// these with 422, leaving the choice up to the caller (some Fal
/// endpoints DO accept data URIs). Otherwise treat it as a local
/// filesystem path, read the bytes, and upload to fal-storage.
fn resolve_audio_input(client: &FalClient, input: &str) -> Result<String, BackendError> {
    if input.starts_with("http://") || input.starts_with("https://") || input.starts_with("data:")
    {
        return Ok(input.to_string());
    }
    let bytes = std::fs::read(input).map_err(|e| {
        BackendError::InvalidRequest(format!("read local audio '{input}': {e}"))
    })?;
    let mime = sniff_audio_mime(input, &bytes);
    let file_name = std::path::Path::new(input)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("audio.wav");
    client.upload_bytes(&bytes, mime, file_name)
}

fn sniff_audio_mime(path: &str, bytes: &[u8]) -> &'static str {
    if bytes.starts_with(b"RIFF") {
        return "audio/wav";
    }
    if bytes.starts_with(b"ID3") || bytes.starts_with(&[0xFF, 0xFB]) || bytes.starts_with(&[0xFF, 0xF3]) {
        return "audio/mpeg";
    }
    if bytes.starts_with(b"OggS") {
        return "audio/ogg";
    }
    if bytes.len() > 4 && &bytes[4..8] == b"ftyp" {
        return "audio/mp4";
    }
    // Fall back to the extension.
    let lower = path.to_ascii_lowercase();
    if lower.ends_with(".mp3") {
        "audio/mpeg"
    } else if lower.ends_with(".m4a") || lower.ends_with(".mp4") {
        "audio/mp4"
    } else if lower.ends_with(".ogg") {
        "audio/ogg"
    } else {
        "audio/wav"
    }
}

#[derive(Debug, Serialize)]
struct WhisperBody {
    audio_url: String,
    task: &'static str,
    chunk_level: &'static str,
}

#[derive(Debug, Deserialize)]
struct WhisperResponse {
    #[allow(dead_code)]
    #[serde(default)]
    text: String,
    #[serde(default)]
    chunks: Vec<WhisperChunk>,
}

#[derive(Debug, Deserialize)]
struct WhisperChunk {
    timestamp: (f32, f32),
    text: String,
    #[allow(dead_code)]
    #[serde(default)]
    speaker: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fresh_cache() -> std::path::PathBuf {
        let tmp = std::env::temp_dir().join(format!(
            "wavelet-fal-whisper-{}",
            AssetCache::request_hash("seed", "seed", &"x").unwrap()
        ));
        let _ = std::fs::remove_dir_all(&tmp);
        tmp
    }

    #[test]
    fn empty_audio_rejected() {
        let client = FalClient::with_key("id:secret", fresh_cache());
        let adapter = FalWhisperWordsAdapter::new(client);
        let req = CaptionsRequest::new("", "");
        assert!(matches!(
            adapter.captions(&req, RunMode::DryRun).unwrap_err(),
            BackendError::InvalidRequest(_)
        ));
    }

    #[test]
    fn dry_run_emits_empty_words() {
        let client = FalClient::with_key("id:secret", fresh_cache());
        let adapter = FalWhisperWordsAdapter::new(client);
        let req = CaptionsRequest::new("https://x/a.wav", "hello");
        let out = adapter.captions(&req, RunMode::DryRun).unwrap();
        assert_eq!(out.mode, "dry-run");
        assert!(out.response.words.is_empty());
        assert_eq!(out.response.provider, PROVIDER);
    }

    #[test]
    fn whisper_response_decodes_word_chunks() {
        let body = r#"{
            "text": "hello world",
            "chunks": [
                { "timestamp": [0.10, 0.42], "text": " hello", "speaker": null },
                { "timestamp": [0.50, 1.05], "text": " world", "speaker": null }
            ]
        }"#;
        let parsed: WhisperResponse = serde_json::from_str(body).unwrap();
        assert_eq!(parsed.chunks.len(), 2);
        assert!((parsed.chunks[0].timestamp.0 - 0.10).abs() < 1e-3);
        assert_eq!(parsed.chunks[1].text.trim(), "world");
    }

    #[test]
    fn url_passes_through() {
        let client = FalClient::with_key("id:secret", fresh_cache());
        assert_eq!(
            resolve_audio_input(&client, "https://example.com/a.wav").unwrap(),
            "https://example.com/a.wav"
        );
        assert_eq!(
            resolve_audio_input(&client, "data:audio/wav;base64,XYZ").unwrap(),
            "data:audio/wav;base64,XYZ"
        );
    }

    #[test]
    fn mime_sniffer_detects_wav() {
        assert_eq!(sniff_audio_mime("foo", b"RIFF...."), "audio/wav");
        assert_eq!(sniff_audio_mime("foo.mp3", &[]), "audio/mpeg");
        assert_eq!(sniff_audio_mime("foo.unknown", &[]), "audio/wav");
    }
}
