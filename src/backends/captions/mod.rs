//! Captions cluster — `(audio, optional reference text)` → per-word
//! timestamps.
//!
//! Captions are *table-stakes* for AI commercial spots: word-level
//! kinetic captions (CapCut / Hormozi style) with ~0.25-0.4s dwell per
//! word and a single emphasis word per beat. See
//! `docs/research/text-in-ai-video.md` §"What the practitioners actually
//! do".
//!
//! ## Prompting shape
//!
//! Every provider in this cluster shares the same inputs and outputs:
//!
//! - **Input**: `audio_url` (HTTPS URL or `data:audio/...;base64,…` URI)
//!   plus an optional `text` reference (the spoken line, used by
//!   force-alignment backends).
//! - **Output**: a flat list of `WordTimestamp { word, start_ms, end_ms }`.
//!
//! Both Whisper-style ASR backends and naive equal-pacing fallbacks
//! satisfy the trait — they only differ in fidelity. The trait does NOT
//! emit caption HTML directly; HTML generation lives in
//! [`crate::backends::captions::overlay`] so the alignment source and
//! the styling step stay decoupled.
//!
//! ## Members
//!
//! - **`fal-whisper-words`** — Fal-hosted Whisper with
//!   `chunk_level: "word"`. Native per-word timestamps, ~$0.006/min.
//!   Lives at [`crate::backends::fal::whisper_words`].
//! - **`synthetic-equal-pacing`** — distributes the VO duration evenly
//!   across the word count. Crude but works offline / dry-run; used as
//!   a fallback when ASR is unavailable. Lives in [`synthetic`].

use crate::backends::{BackendCallOutcome, BackendError, CostEstimate, RunMode};
use serde::{Deserialize, Serialize};

pub mod overlay;
pub mod synthetic;

pub use overlay::{render_overlay_html, OverlayConfig, OverlayStyle};
pub use synthetic::SyntheticEqualPacingAdapter;

/// Cluster identifier — used in cache keys + manifests.
pub const CLUSTER: &str = "captions";

/// One captions request — audio + optional reference text. Force-align
/// backends use the reference text to constrain the recognized words;
/// pure ASR backends ignore it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CaptionsRequest {
    /// Audio source. Accepts:
    /// - `https://…` public URL
    /// - `data:audio/wav;base64,…` data URI
    /// - local filesystem path (resolved + re-encoded as a data URI by
    ///   the adapter at call time)
    pub audio_url: String,
    /// The VO text — the line the audio is *supposed* to contain.
    /// Force-alignment backends use this to lock outputs to the script;
    /// ASR-only backends use it to validate the recognized transcript.
    /// Empty string means "no reference; trust the ASR transcript".
    #[serde(default)]
    pub text: String,
    /// Estimated total audio duration in milliseconds. Optional —
    /// synthetic fallback NEEDS it (to compute per-word dwell);
    /// real-ASR backends ignore it. `0` = unknown.
    #[serde(default)]
    pub duration_ms: u32,
}

impl CaptionsRequest {
    /// Build a minimum-viable captions request.
    pub fn new(audio_url: impl Into<String>, text: impl Into<String>) -> Self {
        Self {
            audio_url: audio_url.into(),
            text: text.into(),
            duration_ms: 0,
        }
    }

    /// Set the audio duration hint (required for the synthetic fallback).
    pub fn with_duration_ms(mut self, ms: u32) -> Self {
        self.duration_ms = ms;
        self
    }
}

/// One word's timing — millisecond-resolution start + end, both
/// relative to the audio's zero.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WordTimestamp {
    /// The literal word as it should appear in the caption. Punctuation
    /// is retained — the overlay generator decides whether to strip it.
    pub word: String,
    /// Word start, milliseconds since audio start.
    pub start_ms: u32,
    /// Word end, milliseconds since audio start.
    pub end_ms: u32,
}

/// Result of a captions alignment call.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CaptionsResult {
    /// Provider identifier (`fal-whisper-words`, `synthetic-equal-pacing`).
    pub provider: String,
    /// Per-word timings, in audio-start-relative milliseconds.
    pub words: Vec<WordTimestamp>,
    /// Total span detected (last word end, ms). `0` when `words` is empty.
    pub total_ms: u32,
}

impl CaptionsResult {
    /// True iff the timestamps are monotonically non-decreasing in
    /// `start_ms` (a basic sanity check used by tests).
    pub fn is_monotonic(&self) -> bool {
        self.words
            .windows(2)
            .all(|w| w[0].start_ms <= w[1].start_ms)
    }
}

/// Cluster trait shared by every captions adapter.
pub trait CaptionsBackend {
    /// Provider name (`"fal-whisper-words"`, `"synthetic-equal-pacing"`).
    fn name(&self) -> &'static str;

    /// Estimate the cost of an alignment call.
    fn estimate_cost(&self, request: &CaptionsRequest) -> CostEstimate;

    /// Run the alignment.
    fn captions(
        &self,
        request: &CaptionsRequest,
        mode: RunMode,
    ) -> Result<BackendCallOutcome<CaptionsResult>, BackendError>;
}

/// Tokenise a VO line into the same word list a recogniser would
/// emit. Whitespace-split, punctuation preserved with its trailing
/// word. Used by the synthetic backend and by the overlay generator
/// when grouping words for CapCut style.
pub fn tokenize_words(text: &str) -> Vec<String> {
    text.split_whitespace().map(|s| s.to_string()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokenize_splits_on_whitespace() {
        assert_eq!(
            tokenize_words("Hello, brave new world!"),
            vec!["Hello,", "brave", "new", "world!"]
        );
    }

    #[test]
    fn tokenize_empty_yields_empty() {
        assert!(tokenize_words("   ").is_empty());
    }

    #[test]
    fn monotonic_check_catches_inversion() {
        let r = CaptionsResult {
            provider: "test".into(),
            total_ms: 0,
            words: vec![
                WordTimestamp { word: "a".into(), start_ms: 0, end_ms: 100 },
                WordTimestamp { word: "b".into(), start_ms: 50, end_ms: 200 },
                WordTimestamp { word: "c".into(), start_ms: 40, end_ms: 300 },
            ],
        };
        assert!(!r.is_monotonic());
    }

    #[test]
    fn request_round_trips() {
        let req = CaptionsRequest::new("https://x/a.wav", "hi")
            .with_duration_ms(1234);
        let json = serde_json::to_string(&req).unwrap();
        let back: CaptionsRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(back.audio_url, "https://x/a.wav");
        assert_eq!(back.text, "hi");
        assert_eq!(back.duration_ms, 1234);
    }
}
