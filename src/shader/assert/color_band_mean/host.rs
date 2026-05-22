//! HSL color-band-mean assertion (wb-mxrk.5).
//!
//! Composition: pure-inline WGSL. The single region scan with HSL math
//! is straightforward in the shader; no sibling-primitive call buys us
//! anything. Color-blindness LMS simulation is DEFERRED — the ticket
//! calls it out as a variant, but it adds a parameter axis (Protan /
//! Deutan / Tritan / Achromat) and a pre-multiply step that's better
//! landed once the validator catalog has a normalized colorblind-mode
//! enum across other assertions too. For now, this validator is the
//! native-color path only.

use std::path::PathBuf;

use anyhow::Result;
use serde_json::json;

use crate::shader::assert::contrast_in_region::Region;
use crate::shader::assert::{dispatch_assertion, AssertionOutcome, FrameSource};

const SHADER: &str = "src/shader/assert/color_band_mean/shader.wgsl";

/// Target HSL band the region's mean color must fall within.
#[derive(Clone, Copy, Debug)]
pub struct HslTarget {
    /// Target hue in [0, 1).
    pub h: f32,
    /// Target saturation in [0, 1].
    pub s: f32,
    /// Target lightness in [0, 1].
    pub l: f32,
    /// Max absolute deviation allowed on each of H/S/L (hue on the wheel).
    pub tolerance: f32,
}

/// Run the color-band-mean assertion over `frame` inside `region`.
/// Passes iff |mean_h - target.h|_circular <= tolerance and same for s, l.
pub fn assert_color_band_mean(
    frame: FrameSource,
    region: Region,
    target: HslTarget,
) -> Result<AssertionOutcome> {
    let shader = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(SHADER);
    let params = json!([
        region.x, region.y, region.w, region.h,
        target.h, target.s, target.l, target.tolerance
    ]);
    dispatch_assertion(&shader, frame, params)
}
