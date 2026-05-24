//! ClipOp — extracted from the wavelet CLI clap definitions.

use std::path::PathBuf;
use clap::{Parser, Subcommand, ValueEnum, Args};



#[derive(Subcommand)]
pub enum ClipOp {
    /// List every clip-ref under `<workdir>/refs/**/*.clip.html`.
    Ls {
        /// Workdir to walk. Defaults to the current directory.
        #[arg(long)]
        workdir: Option<PathBuf>,
        /// Filter by `kind` (kebab-case, e.g. `shot`, `still`).
        #[arg(long)]
        kind: Option<String>,
        /// Filter by `scene` slug (substring match).
        #[arg(long)]
        scene: Option<String>,
        /// Filter by `tags` (one tag substring, case-insensitive).
        #[arg(long)]
        tag: Option<String>,
        /// Print as an ASCII tree grouped by root ancestor of each
        /// edit chain.
        #[arg(long)]
        lineage: bool,
    },
    /// Show one clip-ref by short id or path.
    Show {
        /// Clip-id prefix (≥4 chars) or path to a `.clip.html` file.
        target: String,
        /// Workdir to search. Defaults to the current directory.
        #[arg(long)]
        workdir: Option<PathBuf>,
    },
    /// Print an ASCII tree of the full lineage (ancestors + descendants)
    /// for one clip.
    Lineage {
        /// Clip-id prefix (≥4 chars) for the target clip.
        clip_id: String,
        /// Workdir to walk. Defaults to the current directory.
        #[arg(long)]
        workdir: Option<PathBuf>,
    },
    /// One-shot backfill — walk a legacy workdir's cache + screenplay
    /// outputs and synthesize clip-refs for everything that should have
    /// one. Idempotent.
    Import {
        /// Workdir to import. Defaults to the current directory.
        #[arg(long)]
        workdir: Option<PathBuf>,
        /// Cache directory under the workdir. Defaults to `.wavelet-cache`.
        #[arg(long, default_value = ".wavelet-cache")]
        cache: PathBuf,
        /// Plan only — print actions, don't write.
        #[arg(long)]
        dry_run: bool,
    },
    /// Detect and trim leading / trailing freeze frames from a clip.
    /// Veo and similar AI video generators routinely emit ~0.5–1.5s of
    /// frozen frames at the start (and sometimes end) of clips before
    /// the action begins. This subcommand identifies those static
    /// regions via ffmpeg `freezedetect`, reports the trim range as
    /// JSON, and optionally writes the trimmed result with `--out`.
    /// Lossless stream-copy — no re-encode.
    TrimStatic {
        /// Path to the input MP4 clip.
        input: PathBuf,
        /// When set, write the trimmed clip to this path. When omitted,
        /// only the JSON report is emitted to stdout.
        #[arg(long)]
        out: Option<PathBuf>,
        /// freezedetect noise threshold in dB (negative). Lower is
        /// stricter. Default -60 matches ffmpeg's default; -50 catches
        /// near-freezes where the model emits one pixel of jitter per
        /// frame but is visually static.
        #[arg(long, default_value_t = -60.0)]
        noise_db: f32,
        /// Minimum freeze duration in seconds. Below this, the detector
        /// ignores the freeze. Default 0.4s — short enough to catch
        /// leading freezes on 4-5s clips, long enough to not chop
        /// natural pauses inside motion.
        #[arg(long, default_value_t = 0.4)]
        min_freeze_secs: f32,
        /// Pretty-print the emitted JSON.
        #[arg(long)]
        pretty: bool,
    },
}
