//! ShaderOp — extracted from the wavelet CLI clap definitions.

use std::path::PathBuf;
use clap::{Parser, Subcommand, ValueEnum, Args};



#[derive(Subcommand)]
pub enum ShaderOp {
    /// Validate a WGSL shader file for use with `<gm-shader>` or transitions.
    Validate {
        /// Path to the WGSL source.
        path: PathBuf,
    },
}
