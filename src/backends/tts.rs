//! Text-to-speech cluster — voice_id + text + voice_settings → audio.
//!
//! Providers in this cluster share a prompting shape: a target voice
//! id (a stable handle identifying the synthesized voice) plus a text
//! payload and optional voice-tuning settings (stability, similarity,
//! style). The trait surfaces that shape; per-provider adapters
//! translate it to their wire format.
//!
//! Members: **ElevenLabs** (primary), with Cartesia / Play.ht as future
//! fallbacks sharing the same shape.

use crate::backends::{BackendCallOutcome, BackendError, CostEstimate, RunMode};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Cluster identifier — used in cache keys + manifests.
pub const CLUSTER: &str = "tts";

/// One TTS request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TtsRequest {
    /// Text to synthesize.
    pub text: String,
    /// Provider-specific stable voice handle. For ElevenLabs this is
    /// the `voice_id` GUID (e.g. `21m00Tcm4TlvDq8ikWAM` = "Rachel").
    pub voice_id: String,
    /// Optional model identifier (e.g. `eleven_multilingual_v2`). If
    /// `None` the adapter picks a sensible default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Voice stability — how consistent the voice stays between
    /// generations. 0.0 = expressive but variable, 1.0 = monotone.
    /// Provider-specific scale; usually `[0.0, 1.0]`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stability: Option<f32>,
    /// Similarity boost — how closely the output adheres to the
    /// reference voice. Usually `[0.0, 1.0]`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub similarity_boost: Option<f32>,
    /// Style exaggeration. Higher = more stylized; can degrade
    /// consistency. Usually `[0.0, 1.0]`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub style: Option<f32>,
}

impl TtsRequest {
    /// Build a minimum-viable TTS request.
    pub fn new(text: impl Into<String>, voice_id: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            voice_id: voice_id.into(),
            model: None,
            stability: None,
            similarity_boost: None,
            style: None,
        }
    }
}

/// Result of a TTS synthesis.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TtsResult {
    /// Provider identifier (`elevenlabs`).
    pub provider: String,
    /// Voice id the request was synthesized against.
    pub voice_id: String,
    /// Model id actually used.
    pub model: String,
    /// Path to the cached audio file on disk (provider-specific
    /// container — typically MP3).
    pub audio_path: PathBuf,
    /// Audio file size in bytes.
    pub audio_bytes: u64,
    /// Cheap byte-rate-based duration estimate in seconds (no decode).
    /// Use the audio mixer's symphonia decode for an exact value.
    pub duration_secs_est: f32,
    /// Mime type of the audio container (`audio/mpeg`).
    pub mime: String,
}

/// Cluster trait shared by every TTS adapter.
pub trait VoiceIdTtsBackend {
    /// Provider name (`"elevenlabs"`).
    fn name(&self) -> &'static str;

    /// Estimate the cost of a request. TTS providers typically bill per
    /// character; the estimate uses the request's text length.
    fn estimate_cost(&self, request: &TtsRequest) -> CostEstimate;

    /// Synthesize audio. The cached audio file path is returned in the
    /// `TtsResult.audio_path` field.
    fn synthesize(
        &self,
        request: &TtsRequest,
        mode: RunMode,
    ) -> Result<BackendCallOutcome<TtsResult>, BackendError>;
}

// Re-export the centralized budget gate.
pub(crate) use crate::backends::check_budget;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_round_trips() {
        let req = TtsRequest {
            text: "Hello, world.".into(),
            voice_id: "21m00Tcm4TlvDq8ikWAM".into(),
            model: Some("eleven_multilingual_v2".into()),
            stability: Some(0.5),
            similarity_boost: Some(0.75),
            style: Some(0.0),
        };
        let json = serde_json::to_string(&req).unwrap();
        let back: TtsRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(back.text, "Hello, world.");
        assert_eq!(back.stability, Some(0.5));
    }

    // Centralized budget gate is covered by `backends::mod.rs` tests.
}
