//! PipelinesOp — extracted from the wavelet CLI clap definitions.

use std::path::PathBuf;
use clap::{Parser, Subcommand, ValueEnum, Args};



/// Declarative-pipeline subcommands.
#[derive(Subcommand)]
pub enum PipelinesOp {
    /// List every pipeline discovered under the search directory.
    List {
        /// Override the search directory. Defaults to
        /// `packages/wavelet/pipeline_defs/` (baked at compile time).
        #[arg(long)]
        dir: Option<PathBuf>,
        /// Emit machine-readable JSON instead of the text table.
        #[arg(long)]
        json: bool,
    },
    /// Show the parsed schema of one pipeline (by name or by path).
    Show {
        /// Pipeline name (matched against `name:` in each YAML) or a
        /// direct path to a `.yaml` file.
        name_or_path: String,
        /// Override the search directory used to resolve a bare name.
        #[arg(long)]
        dir: Option<PathBuf>,
        /// Emit JSON. Default is YAML round-trip.
        #[arg(long)]
        json: bool,
    },
    /// Parse + validate a YAML pipeline. Exits non-zero on any error.
    Validate {
        /// Path to the YAML file.
        path: PathBuf,
    },
    /// Print the execution plan for a pipeline + brief. Stub: actual
    /// stage execution lives in `wavelet workflow run` (wb-oemp).
    Run {
        /// Pipeline name or path.
        name_or_path: String,
        /// Optional brief JSON to bind into the plan. Surfaced in the
        /// plan output only — execution lands with wb-oemp.
        brief: Option<PathBuf>,
        /// Override the search directory used to resolve a bare name.
        #[arg(long)]
        dir: Option<PathBuf>,
    },
}
