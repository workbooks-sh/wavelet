//! BriefOp — extracted from the wavelet CLI clap definitions.

use std::path::PathBuf;
use clap::{Parser, Subcommand, ValueEnum, Args};



#[derive(Subcommand)]
pub enum BriefOp {
    /// Parse a 9-line ad creative brief and report missing slots, parse
    /// errors, and lightweight content warnings. Pure parsing — no LLM
    /// calls, no network.
    Check {
        /// Path to the brief markdown file.
        path: PathBuf,
        /// Emit the parsed brief as JSON to stdout (after the OK line).
        #[arg(long)]
        json: bool,
        /// Pretty-print the JSON output.
        #[arg(long, requires = "json")]
        pretty: bool,
    },
}
