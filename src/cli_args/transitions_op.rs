//! TransitionsOp — extracted from the wavelet CLI clap definitions.

use std::path::PathBuf;
use clap::{Parser, Subcommand, ValueEnum, Args};



#[derive(Subcommand)]
pub enum TransitionsOp {
    /// Classify transitions from a screenplay + velocity profile.
    Classify {
        /// Path to the Fountain source file.
        screenplay: PathBuf,
        /// Path to the velocity profile JSON.
        #[arg(long)]
        velocity: PathBuf,
        /// Pretty-print the emitted JSON.
        #[arg(long)]
        pretty: bool,
        /// Optional output path. Without it, JSON goes to stdout.
        #[arg(short, long)]
        out: Option<PathBuf>,
    },
}
