//! MusicOp — extracted from the wavelet CLI clap definitions.

use std::path::PathBuf;
use clap::{Parser, Subcommand, ValueEnum, Args};



#[derive(Subcommand)]
pub enum MusicOp {
    /// Generate music. Either supply `--prompt` + `--duration` directly,
    /// or supply `--velocity <profile.json> --style "<text>"` to derive
    /// a velocity-aware prompt automatically.
    Gen {
        /// Free-text prompt. Required unless `--velocity` is given.
        #[arg(long)]
        prompt: Option<String>,
        /// Path to a velocity profile JSON. If set, the prompt is
        /// derived from the profile + `--style` and duration matches
        /// the profile.
        #[arg(long)]
        velocity: Option<PathBuf>,
        /// Style descriptor used when deriving a prompt from
        /// `--velocity` (e.g. "cinematic ambient strings").
        #[arg(long, default_value = "cinematic")]
        style: String,
        /// Duration in seconds. Required unless `--velocity` is given.
        #[arg(long)]
        duration: Option<f32>,
        /// Target BPM (overrides the velocity profile's mean BPM when
        /// both are set).
        #[arg(long)]
        bpm: Option<f32>,
        /// Backend. `elevenlabs` (Merlin+Kobalt-licensed,
        /// commercial-safe), `google-lyria-3-pro` / `lyria-pro`, or
        /// `udio` (partnership tier). When unset, resolves from the
        /// cascade (music slot → tool default `lyria-pro`).
        #[arg(long)]
        backend: Option<String>,
        /// Model variant override (e.g. `stereo-large`, `melody-large`).
        #[arg(long)]
        variant: Option<String>,
        /// Random seed for reproducibility.
        #[arg(long)]
        seed: Option<u64>,
        /// Emit the request spec without hitting the API.
        #[arg(long)]
        dry_run: bool,
        /// Maximum USD spend permitted.
        #[arg(long, default_value_t = 0.0)]
        max_cost: f32,
        /// Cache root.
        #[arg(long, default_value = ".wavelet-cache")]
        cache: PathBuf,
        /// Optional destination path. When set, the cached audio is
        /// copied here.
        #[arg(short, long)]
        out: Option<PathBuf>,
        /// Pretty-print the emitted JSON.
        #[arg(long)]
        pretty: bool,
    },
}
