//! WorkflowOp — extracted from the wavelet CLI clap definitions.

use std::path::PathBuf;
use clap::{Parser, Subcommand, ValueEnum, Args};



/// Workflow subcommands.
#[derive(Subcommand)]
pub enum WorkflowOp {
    /// Compute and emit the workflow state report for a pipeline.
    Run {
        /// Pipeline name (resolved via `pipelines list`) or a direct
        /// path to a `.yaml`.
        name_or_path: String,
        /// Working directory the report is computed against. Defaults
        /// to the current directory.
        #[arg(long)]
        workdir: Option<PathBuf>,
        /// Override the pipeline search directory used to resolve a
        /// bare name.
        #[arg(long)]
        dir: Option<PathBuf>,
        /// Emit pretty JSON (default). With `--text`, prints a
        /// human-readable summary instead.
        #[arg(long)]
        text: bool,
    },
}
