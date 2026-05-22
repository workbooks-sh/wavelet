//! DialogueOp — extracted from the wavelet CLI clap definitions.

use std::path::PathBuf;
use clap::{Parser, Subcommand, ValueEnum, Args};



#[derive(Subcommand)]
pub enum DialogueOp {
    /// Word-level caption alignment — run a VO audio file through an
    /// ASR backend (Fal Whisper with `chunk_level: "word"`) or the
    /// synthetic equal-pacing fallback. Emits a `WordTimestamp[]`
    /// JSON the `wavelet captions overlay` verb consumes.
    Captions {
        /// Path or URL of the VO audio file. Local files are encoded
        /// as a `data:` URI; HTTPS URLs and existing `data:` URIs pass
        /// through.
        #[arg(long)]
        audio: String,
        /// The spoken line — used by the synthetic fallback (word
        /// split) and kept in the manifest for the ASR backend. Pass
        /// `""` to let the ASR transcribe-only path drive the output.
        #[arg(long, default_value = "")]
        text: String,
        /// Backend: `fal-whisper-words` (default) or `synthetic`.
        #[arg(long, default_value = "fal-whisper-words")]
        backend: String,
        /// Total VO duration in milliseconds — required for the
        /// synthetic fallback; ignored by `fal-whisper-words`.
        #[arg(long, default_value_t = 0)]
        duration_ms: u32,
        /// Optional style tag stored in the JSON so the overlay verb
        /// can default to the same style. One of
        /// `hormozi|capcut|minimal`.
        #[arg(long)]
        style: Option<String>,
        /// Emit the request spec without hitting the API.
        #[arg(long)]
        dry_run: bool,
        /// Maximum USD spend permitted for this call.
        #[arg(long, default_value_t = 0.0)]
        max_cost: f32,
        /// Cache root.
        #[arg(long, default_value = ".wavelet-cache")]
        cache: PathBuf,
        /// Optional output JSON path. Without `-o` the JSON prints to
        /// stdout.
        #[arg(short, long)]
        out: Option<PathBuf>,
        /// Pretty-print the emitted JSON.
        #[arg(long)]
        pretty: bool,
    },
    /// Text-to-speech across the `VoiceIdTts` cluster (ElevenLabs
    /// primary). Caches the result by request hash; identical requests
    /// return the cached MP3 with no re-billing.
    Tts {
        /// Text to synthesize.
        text: String,
        /// Voice id (provider-specific). Defaults to ElevenLabs
        /// "Rachel" — pass `--voice <id>` to override.
        #[arg(long, default_value = "21m00Tcm4TlvDq8ikWAM")]
        voice: String,
        /// Backend identifier. When unset, resolves from the cascade
        /// (tts slot → tool default `elevenlabs`).
        #[arg(long)]
        backend: Option<String>,
        /// Model id override (provider-specific).
        #[arg(long)]
        model: Option<String>,
        /// Voice stability (0.0–1.0).
        #[arg(long)]
        stability: Option<f32>,
        /// Voice similarity boost (0.0–1.0).
        #[arg(long)]
        similarity: Option<f32>,
        /// Voice style exaggeration (0.0–1.0).
        #[arg(long)]
        style: Option<f32>,
        /// Emit the request spec without hitting the API.
        #[arg(long)]
        dry_run: bool,
        /// Maximum USD spend permitted for this call.
        #[arg(long, default_value_t = 0.0)]
        max_cost: f32,
        /// Cache root.
        #[arg(long, default_value = ".wavelet-cache")]
        cache: PathBuf,
        /// Optional destination path. When set, the cached MP3 is
        /// copied here as a convenience.
        #[arg(short, long)]
        out: Option<PathBuf>,
        /// Pretty-print the emitted JSON.
        #[arg(long)]
        pretty: bool,
    },
}
