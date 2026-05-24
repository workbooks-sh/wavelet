//! ScreenplayOp — extracted from the wavelet CLI clap definitions.

use std::path::PathBuf;
use clap::{Parser, Subcommand, ValueEnum, Args};



/// Subcommands for `wavelet screenplay`.
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
    /// Validate that the screenplay's copy density fits the declared
    /// spot duration. Computes VO time + caption dwell + shot floor and
    /// compares against `--duration`. Exit 0 on `fits` or `under_budget`;
    /// non-zero exit on `over_budget`. Emits a JSON report on stdout.
    Validate {
        /// Path to the Fountain source file.
        path: PathBuf,
        /// Declared spot duration in seconds. The verdict is computed
        /// against this target (with a ±10% tolerance band).
        #[arg(long)]
        duration: f32,
        /// Pretty-print the emitted JSON.
        #[arg(long)]
        pretty: bool,
    },
    /// Extract the canonical character registry from a Fountain screenplay.
    /// Characters are deduplicated by normalized name so `ALEX`, `Alex`, and
    /// `ALEX (V.O.)` collapse into one entry. Outputs a pretty table by
    /// default; use `--json` for structured JSON or `--pretty` for
    /// pretty-printed JSON.
    Characters {
        /// Path to the Fountain source file.
        path: PathBuf,
        /// Emit structured JSON instead of the table.
        #[arg(long)]
        json: bool,
        /// Emit pretty-printed JSON instead of the table.
        #[arg(long)]
        pretty: bool,
    },
}
