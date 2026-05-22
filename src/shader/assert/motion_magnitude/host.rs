//! Temporal motion-magnitude assertion (wb-mxrk.5).
//!
//! Composition: hybrid. The shader operates over a single texture, so
//! we pre-compute the per-pixel L2 RGB delta on the host (CPU) between
//! frames N-1 and N, pack it as a single Rgba8 frame (delta broadcast
//! across RGB), and hand THAT to `dispatch_assertion` as binding 0. The
//! shader then runs an 8-bucket histogram + mean reducer over the diff
//! frame, no extra binding slot needed and no ABI change. Documented
//! choice: keeps the assertion layer ABI-pure.
//!
//! Reason codes (set by the shader):
//!   0 = pass (mean motion >= min_mean, or both frames identical when threshold=0)
//!   1 = fail, mean motion below floor
//!   2 = empty region (degenerate frame dims)
//!
//! Evidence:
//!   [0..8]  = 8-bucket histogram of per-pixel motion magnitude
//!   [8]     = mean motion magnitude across the frame
//!   [9]     = min_mean threshold

use std::path::PathBuf;

use anyhow::{anyhow, Result};
use serde_json::json;

use crate::shader::assert::{dispatch_assertion, AssertionOutcome, FrameSource};

const SHADER: &str = "src/shader/assert/motion_magnitude/shader.wgsl";

/// Run the motion-magnitude assertion between `prev` and `curr` frames.
/// Passes iff the mean per-pixel L2 RGB delta across the frame is
/// `>= min_mean`. The two frames must share dimensions.
pub fn assert_motion(
    prev: FrameSource,
    curr: FrameSource,
    min_mean: f32,
) -> Result<AssertionOutcome> {
    let (w, h, prev_px) = decode_rgba8(prev)?;
    let (w2, h2, curr_px) = decode_rgba8(curr)?;
    if w != w2 || h != h2 {
        return Err(anyhow!(
            "motion_magnitude: prev {w}x{h} != curr {w2}x{h2}"
        ));
    }

    let mut diff = Vec::with_capacity(prev_px.len());
    for i in 0..(w as usize * h as usize) {
        let pr = prev_px[i * 4] as f32 / 255.0;
        let pg = prev_px[i * 4 + 1] as f32 / 255.0;
        let pb = prev_px[i * 4 + 2] as f32 / 255.0;
        let cr = curr_px[i * 4] as f32 / 255.0;
        let cg = curr_px[i * 4 + 1] as f32 / 255.0;
        let cb = curr_px[i * 4 + 2] as f32 / 255.0;
        let dr = cr - pr;
        let dg = cg - pg;
        let db = cb - pb;
        // L2 magnitude normalized so a full-channel swap = sqrt(3) → clamp 1.0.
        let mag = ((dr * dr + dg * dg + db * db).sqrt() / 3.0_f32.sqrt()).clamp(0.0, 1.0);
        let q = (mag * 255.0).round() as u8;
        diff.push(q);
        diff.push(q);
        diff.push(q);
        diff.push(255);
    }

    let frame = FrameSource::Rgba8 {
        width: w,
        height: h,
        pixels: diff,
    };
    let shader = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(SHADER);
    let params = json!([min_mean]);
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
            "motion_magnitude: pre-allocated wgpu Texture not yet supported (needs readback path)"
        )),
    }
}
