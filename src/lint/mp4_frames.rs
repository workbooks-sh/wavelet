//! Sample one RGBA frame from an MP4 at an arbitrary timestamp.
//!
//! Pragmatic implementation: shells out to system `ffmpeg`. We could
//! drop down to rsmpeg for in-process decoding, but the lint stage is
//! batch-y enough (4-8 frames per commercial) that the subprocess cost
//! is rounding error, and ffmpeg is already a hard dep in eval / render
//! paths. Keeps this module to ~80 LOC of glue versus ~300 LOC of
//! libav lifetime juggling.

use super::text_readability_contrast::RenderedFrame;
use std::path::Path;
use std::process::{Command, Stdio};

/// Sample one RGBA frame from `mp4_path` at `t_secs`. The frame is
/// scaled to `width × height` and returned as a top-down sRGB RGBA
/// buffer matching the existing `RenderedFrame` contract. Returns
/// `None` on any ffmpeg error, missing binary, or short-read.
pub fn sample_frame_rgba(
    mp4_path: &Path,
    t_secs: f32,
    width: u32,
    height: u32,
) -> Option<RenderedFrame> {
    // Seek with `-ss` BEFORE `-i` for fast keyframe seek then a fine
    // grain decode. `-vframes 1` stops after one frame; rawvideo +
    // rgba pix-fmt + pipe:1 keeps everything in-memory.
    let mut child = Command::new("ffmpeg")
        .args([
            "-nostdin",
            "-hide_banner",
            "-loglevel",
            "error",
            "-ss",
            &format!("{t_secs:.3}"),
            "-i",
        ])
        .arg(mp4_path)
        .args([
            "-frames:v",
            "1",
            "-vf",
            &format!("scale={width}:{height}:flags=lanczos"),
            "-f",
            "rawvideo",
            "-pix_fmt",
            "rgba",
            "pipe:1",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .ok()?;

    let out = child.wait_with_output().ok()?;
    if !out.status.success() {
        return None;
    }
    let expected = (width as usize) * (height as usize) * 4;
    if out.stdout.len() < expected {
        return None;
    }
    // ffmpeg may include trailing padding on certain pix-fmts; truncate
    // to the exact buffer the caller will read.
    let rgba = out.stdout[..expected].to_vec();
    Some(RenderedFrame { width, height, rgba })
}

/// Probe the duration of an MP4 via `ffprobe`. Returns `None` on
/// missing binary or parse error.
pub fn probe_duration_secs(mp4_path: &Path) -> Option<f32> {
    let out = Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-show_entries",
            "format=duration",
            "-of",
            "default=noprint_wrappers=1:nokey=1",
        ])
        .arg(mp4_path)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let s = std::str::from_utf8(&out.stdout).ok()?.trim();
    s.parse::<f32>().ok()
}
