//! Depth estimation for background / foreground discrimination.
//!
//! Exposes a 16×16 relative-depth grid per frame. Bright cells = close
//! (foreground); dark cells = far (background). Consumers use the grid
//! to pick text placement regions and to flag text overlays that land on
//! the subject rather than the negative space behind it.
//!
//! # Feature gate
//!
//! This module is compiled only when the `depth` Cargo feature is
//! enabled. Opt in with:
//!
//! ```toml
//! wavelet = { ..., features = ["depth"] }
//! ```
//!
//! or at the CLI level:
//!
//! ```text
//! cargo build -p wavelet --features depth
//! ```
//!
//! On first use the depth model (~25 MB fp16 ONNX) is fetched from
//! Hugging Face to `~/.wavelet/models/depth/depth-anything-v2-small.onnx`
//! via [`model::ensure_model`]. Subsequent runs use the cached file.
//!
//! # Architecture
//!
//! The implementation lives in two sub-modules:
//!
//! - [`model`] — model download, session management, CoreML EP on macOS.
//! - [`depth_anything`] — inference against Depth Anything V2 Small
//!   (ViT-S/14, 196×196 input, fp16 weights), grid pooling, min-max
//!   normalisation.

pub mod depth_anything;
pub mod model;

pub use depth_anything::{DepthGrid, estimate_depth, GRID_SIZE};
