//! ContinuityOp — extracted from the wavelet CLI clap definitions.

use std::path::PathBuf;
use clap::{Parser, Subcommand, ValueEnum, Args};



#[derive(Subcommand)]
pub enum ContinuityOp {
    /// Analyze a storyboard's cuts for grammar violations.
    Check {
        /// Path to the storyboard JSON.
        storyboard: PathBuf,
        /// Emit the full report as JSON instead of human-readable text.
        #[arg(long)]
        json: bool,
    },
}
