//! Sobel edge-density assertion (wb-mxrk.5).
//!
//! Composition: pure-inline WGSL. The sibling `sobel` primitive returns a
//! `wgpu::Texture` bound to its own `wgpu::Device` instance, which the
//! `dispatch_assertion` device can't sample without an inter-device
//! readback. For the small regions edge-density runs on the inline 3x3
//! Sobel inside the assertion shader is the simpler, device-local path.
//! If wb-mxrk.7's wgsl_to_wgpu codegen lands a shared device + dispatch
//! pool we'll revisit and route through the primitive.

use std::path::PathBuf;

use anyhow::Result;
use serde_json::json;

use crate::shader::assert::contrast_in_region::Region;
use crate::shader::assert::{dispatch_assertion, AssertionOutcome, FrameSource};

const SHADER: &str = "src/shader/assert/sobel_edge_density/shader.wgsl";

/// Run the Sobel edge-density assertion over `frame` inside `region`.
/// `threshold` is the per-pixel Sobel magnitude floor (raw kernel
/// magnitude, typical range 0..~4 for normalized Rgba8 input);
/// `min_density` is the minimum fraction of region pixels that must
/// exceed `threshold` for the assertion to pass.
pub fn assert_sobel_edge_density(
    frame: FrameSource,
    region: Region,
    threshold: f32,
    min_density: f32,
) -> Result<AssertionOutcome> {
    let shader = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(SHADER);
    let params = json!([
        region.x, region.y, region.w, region.h,
        threshold, min_density
    ]);
    dispatch_assertion(&shader, frame, params)
}
