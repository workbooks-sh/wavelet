//! RMSE-against-golden assertion (wb-mxrk.5).
//!
//! Composition: hybrid. PNG loading goes through host-side decode (using
//! the same path as `FrameSource::PngPath`) so both `frame` and `golden`
//! can be supplied as paths or in-memory buffers. The per-pixel
//! max-channel absolute diff is computed on the CPU and packed into a
//! single Rgba8 diff frame fed to binding 0. The shader then walks the
//! diff frame to compute global RMSE and the count of pixels exceeding
//! `max_diff`, decides pass/fail against `max_pixels`.
//!
//! Tolerance grammar follows wpt's `fuzzy(maxDifference; totalPixels)`:
//! a pixel is "different enough to count" iff its max-channel absolute
//! diff exceeds `max_diff` (0..=255). The assertion passes iff the
//! count of such pixels is `<= max_pixels`.
//!
//! Reason codes:
//!   0 = pass
//!   1 = fail (over_count > max_pixels)
//!   2 = empty frame
//!   5 = dimension mismatch frame vs golden (host-side, error path)
//!
//! Evidence:
//!   [0] = global RMSE on [0, 1] scale
//!   [1] = count of pixels exceeding max_diff
//!   [2] = max_diff_norm parameter
//!   [3] = max_pixels parameter

use std::path::PathBuf;

use anyhow::{anyhow, Result};
use serde_json::json;

use crate::shader::assert::{dispatch_assertion, AssertionOutcome, FrameSource};

const SHADER: &str = "src/shader/assert/golden_rmse/shader.wgsl";

/// Run the golden-RMSE assertion comparing `frame` against `golden`.
/// `max_diff` is the per-channel absolute byte threshold (0..=255), and
/// `max_pixels` is the maximum number of pixels allowed to exceed that
/// threshold for the assertion to pass.
pub fn assert_golden_rmse(
    frame: FrameSource,
    golden: FrameSource,
    max_diff: u32,
    max_pixels: u32,
) -> Result<AssertionOutcome> {
    let (w, h, frame_px) = decode_rgba8(frame)?;
    let (w2, h2, golden_px) = decode_rgba8(golden)?;
    if w != w2 || h != h2 {
        return Err(anyhow!(
            "golden_rmse: frame {w}x{h} != golden {w2}x{h2}"
        ));
    }

    let mut diff = Vec::with_capacity(frame_px.len());
    for i in 0..(w as usize * h as usize) {
        let dr = frame_px[i * 4].abs_diff(golden_px[i * 4]);
        let dg = frame_px[i * 4 + 1].abs_diff(golden_px[i * 4 + 1]);
        let db = frame_px[i * 4 + 2].abs_diff(golden_px[i * 4 + 2]);
        let m = dr.max(dg).max(db);
        diff.push(m);
        diff.push(m);
        diff.push(m);
        diff.push(255);
    }

    let frame = FrameSource::Rgba8 {
        width: w,
        height: h,
        pixels: diff,
    };
    let shader = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(SHADER);
    let max_diff_norm = (max_diff as f32) / 255.0;
    let params = json!([max_diff_norm, max_pixels]);
    dispatch_assertion(&shader, frame, params)
}

fn decode_rgba8(frame: FrameSource) -> Result<(u32, u32, Vec<u8>)> {
    match frame {
        FrameSource::Rgba8 { width, height, pixels } => Ok((width, height, pixels)),
        FrameSource::PngPath(path) => {
            let file = std::fs::File::open(&path)?;
            let decoder = png::Decoder::new(file);
            let mut reader = decoder.read_info()?;
            let mut buf = vec![0u8; reader.output_buffer_size()];
            let info = reader.next_frame(&mut buf)?;
            let pixels = match info.color_type {
                png::ColorType::Rgba => buf[..info.buffer_size()].to_vec(),
                png::ColorType::Rgb => {
                    let mut out = Vec::with_capacity(info.buffer_size() / 3 * 4);
                    for px in buf[..info.buffer_size()].chunks_exact(3) {
                        out.extend_from_slice(px);
                        out.push(255);
                    }
                    out
                }
                other => return Err(anyhow!("unsupported PNG color type: {other:?}")),
            };
            Ok((info.width, info.height, pixels))
        }
        FrameSource::Texture(_) => Err(anyhow!(
            "golden_rmse: pre-allocated wgpu Texture not yet supported"
        )),
    }
}
