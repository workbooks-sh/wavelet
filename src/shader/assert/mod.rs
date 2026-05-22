//! Shader-as-validator ABI + dispatcher (wb-mxrk.1).
//!
//! See `ABI.md` for the WGSL convention every assertion shader follows.
//! Public surface: `dispatch_assertion` plus the result + frame-source
//! types it consumes and produces.

mod context;
mod dispatch;
pub mod histogram;
pub mod masked_reduce;
pub mod reduce;
mod runtime;
pub mod sobel;
mod types;
pub mod windowed_stats;

/// HSL color-band-mean assertion shader (wb-mxrk.5).
pub mod color_band_mean;
/// WCAG luminance-contrast assertion shader (wb-mxrk.5).
pub mod contrast_in_region;
/// RMSE-vs-golden assertion shader (wb-mxrk.5).
pub mod golden_rmse;
/// Temporal motion-magnitude assertion shader (wb-mxrk.5).
pub mod motion_magnitude;
/// Sobel edge-density assertion shader (wb-mxrk.5).
pub mod sobel_edge_density;

pub use context::GpuContext;
pub use dispatch::dispatch_assertion;
pub use histogram::{histogram, Channel};
pub use runtime::{run_assertion, run_assertion_batch, ShaderAssertion, TextureHandle};
pub use masked_reduce::{id_texture_from_u32, masked_reduce, MaskedReduceOp};
pub use reduce::{reduce, reduce_rgba, Rect, ReduceOp};
pub use sobel::{sobel, SobelOutput};
pub use types::{
    AssertionOutcome, AssertionResult, FrameSource, ReasonCode, EVIDENCE_CAPACITY,
    PARAMS_MAX_BYTES,
};
pub use windowed_stats::{windowed_stats, DEFAULT_SIGMA, DEFAULT_WINDOW};

#[cfg(test)]
mod tests;
