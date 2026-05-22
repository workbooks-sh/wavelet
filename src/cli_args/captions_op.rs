//! CaptionsOp — extracted from the wavelet CLI clap definitions.

use std::path::PathBuf;
use clap::{Parser, Subcommand, ValueEnum, Args};



#[derive(Subcommand)]
pub enum CaptionsOp {
    /// Render a CapCut / Hormozi / minimal-style HTML overlay from a
    /// `captions.json` produced by `wavelet dialogue captions`. The
    /// output HTML is self-contained — drop it into the
    /// workbook-video scene-overlay flow as-is.
    Overlay {
        /// Path to the captions JSON. Reads the schema emitted by
        /// `wavelet dialogue captions` (the `result` block).
        #[arg(long)]
        r#in: PathBuf,
        /// Style preset. `hormozi` | `capcut` | `minimal`. When the
        /// captions JSON carries a `style` hint, this flag overrides it.
        #[arg(long, default_value = "hormozi")]
        style: String,
        /// Total animation duration in milliseconds. `0` uses the last
        /// word's `end_ms`.
        #[arg(long, default_value_t = 0)]
        duration: u32,
        /// Output canvas width in CSS pixels.
        #[arg(long, default_value_t = 1080)]
        width: u32,
        /// Output canvas height in CSS pixels.
        #[arg(long, default_value_t = 1920)]
        height: u32,
        /// Output HTML path.
        #[arg(short, long)]
        out: PathBuf,
    },
}
