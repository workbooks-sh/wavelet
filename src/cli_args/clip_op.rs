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
}
