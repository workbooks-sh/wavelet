//! WCAG luminance-contrast assertion (wb-mxrk.5).
//!
//! Composition: pure-inline WGSL. The shader walks the region once on a
//! single thread to find min/max relative luminance, computes the W3C
//! contrast ratio, and decides. No sibling-primitive calls — for a single
//! region scan the dispatch overhead of a multi-pass reduction is larger
//! than the naive loop.

use std::path::PathBuf;

use anyhow::Result;
use serde_json::json;

use crate::shader::assert::{dispatch_assertion, AssertionOutcome, FrameSource};

const SHADER: &str = "src/shader/assert/contrast_in_region/shader.wgsl";

/// Normalized 0..1 region rectangle.
#[derive(Clone, Copy, Debug)]
pub struct Region {
    /// Left edge as fraction of frame width.
    pub x: f32,
    /// Top edge as fraction of frame height.
    pub y: f32,
    /// Width as fraction of frame width.
    pub w: f32,
    /// Height as fraction of frame height.
    pub h: f32,
}

/// Run the contrast assertion over `frame`. Passes iff the WCAG 2.2
/// contrast ratio between the lightest and darkest pixel in the region
/// is >= `min_contrast` (4.5 = AA body text, 7.0 = AAA).
pub fn assert_contrast(
    frame: FrameSource,
    region: Region,
    min_contrast: f32,
) -> Result<AssertionOutcome> {
    let shader = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(SHADER);
    let params = json!([region.x, region.y, region.w, region.h, min_contrast]);
    dispatch_assertion(&shader, frame, params)
}
