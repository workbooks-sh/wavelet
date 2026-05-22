//! ScreenplayOp — extracted from the wavelet CLI clap definitions.

use std::path::PathBuf;
use clap::{Parser, Subcommand, ValueEnum, Args};



#[derive(Subcommand)]
pub enum ScreenplayOp {
    /// Parse a `.fountain` screenplay. Emits one `.clip.html` per scene
    /// under `<workdir>/refs/screenplay-scene/`. Use `--legacy-json` to
    /// emit the old single-blob `screenplay.json` instead.
    Parse {
        /// Path to the Fountain source file.
        path: PathBuf,
        /// Workdir (where `refs/screenplay-scene/` lives). Defaults to
        /// the directory containing `path`.
        #[arg(long)]
        workdir: Option<PathBuf>,
        /// Emit the legacy single-blob `screenplay.json` instead of
        /// per-scene clip-refs. Stdout when `--out` isn't passed.
        #[arg(long)]
        legacy_json: bool,
        /// Pretty-print the emitted JSON (only used with `--legacy-json`).
        #[arg(long)]
        pretty: bool,
        /// Output path for `--legacy-json` mode. Ignored otherwise.
        #[arg(short, long)]
        out: Option<PathBuf>,
    },
    /// Reassemble per-scene clip-refs back into a single `.fountain`.
    Reassemble {
        /// Workdir containing `refs/screenplay-scene/`.
        workdir: PathBuf,
        /// Output path. Stdout when omitted.
        #[arg(short, long)]
        out: Option<PathBuf>,
    },
}
