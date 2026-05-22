//! VelocityOp — extracted from the wavelet CLI clap definitions.

use std::path::PathBuf;
use clap::{Parser, Subcommand, ValueEnum, Args};



#[derive(Subcommand)]
pub enum VelocityOp {
    /// Propose a velocity profile from a Fountain screenplay (heuristic,
    /// no LLM). Outputs the JSON profile to stdout or `-o <path>`.
    Propose {
        /// Path to the Fountain source file.
        screenplay: PathBuf,
        /// Optional output path. Without it, JSON goes to stdout.
        #[arg(short, long)]
        out: Option<PathBuf>,
        /// Pretty-print the emitted JSON.
        #[arg(long)]
        pretty: bool,
    },
    /// Validate a velocity profile against the detected BPM of a music
    /// track. Onsets within ±window-radius of each anchor are
    /// converted to BPM and compared.
    Validate {
        /// Path to the JSON velocity profile.
        profile: PathBuf,
        /// Path to the music file.
        #[arg(long, value_name = "AUDIO")]
        against: PathBuf,
        /// BPM delta allowed per anchor before it's flagged. Default 5.
        #[arg(long, default_value_t = 5.0)]
        tolerance: f32,
        /// Window radius in seconds around each anchor for onset
        /// counting. Default 2.0.
        #[arg(long, default_value_t = 2.0)]
        window: f32,
        /// Pretty-print the emitted JSON.
        #[arg(long)]
        pretty: bool,
        /// Frame rate for the sibling `cuts.edl` emitted alongside the
        /// validation report. Default 30.
        #[arg(long, default_value_t = 30)]
        fps: u32,
        /// Suppress the sibling `cuts.edl` emission. Default is to
        /// write `<against>.cuts.edl` next to the audio file.
        #[arg(long)]
        no_emit_edl: bool,
    },
    /// Detect musical onsets in an audio file and emit them as cut
    /// markers in an EDL (Final Cut Pro 7 / Resolve compatible).
    OnsetsToEdl {
        /// Path to the music file.
        #[arg(long, value_name = "AUDIO")]
        music: PathBuf,
        /// Frame rate for the timecode column. Default 30.
        #[arg(long, default_value_t = 30)]
        fps: u32,
        /// Output format. Only `edl` is implemented today; `fcpxml`
        /// and `premiere-marker-csv` are reserved for follow-ups.
        #[arg(long, default_value = "edl")]
        format: String,
        /// Output path. Without it, the EDL goes to stdout.
        #[arg(short, long)]
        out: Option<PathBuf>,
    },
    /// Render the velocity profile as standalone SVG.
    RenderCurve {
        /// Path to the JSON velocity profile.
        profile: PathBuf,
        /// Optional validation report (output of `velocity validate`) to
        /// overlay detected BPM points + verdict.
        #[arg(long)]
        overlay: Option<PathBuf>,
        /// Optional output SVG path. Without it, SVG goes to stdout.
        #[arg(short, long)]
        out: Option<PathBuf>,
    },
}
