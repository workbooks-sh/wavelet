//! Per-frame video diff with motion-aware metrics. Phase 4 of epic wb-q4a6.
//!
//! Given two rendered MP4s (and optionally a comp.json for selector-based
//! masking), this module decodes both in lockstep, runs a perceptual or
//! pixelmatch metric per frame, and reports aggregate + worst-frame stats.
//!
//! Hand-rolled implementations of pixelmatch and SSIM (both MIT-friendly,
//! no copyleft deps). SSIMULACRA2 is intentionally deferred — the
//! reference impl is AGPL.

use rsmpeg::avcodec::{AVCodec, AVCodecContext};
use rsmpeg::avformat::AVFormatContextInput;
use rsmpeg::avutil::AVFrame;
use rsmpeg::error::RsmpegError;
use rsmpeg::ffi;
use rsmpeg::swscale::SwsContext;
use serde::{Deserialize, Serialize};
use std::ffi::CString;
use std::path::Path;

use super::snapshot::Rect;

/// Perceptual / pixel-level metric used for each frame comparison.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiffMetric {
    /// Fraction of pixels whose Manhattan RGBA distance exceeds the
    /// per-channel threshold (default 12/255 ≈ 5%). Cheap, sensitive.
    Pixelmatch,
    /// SSIM (Structural Similarity Index Measure). Returns 1.0 - mean_ssim
    /// so larger = more different. Closer to human perception than
    /// pixelmatch; tolerates compression noise.
    Ssim,
}

impl Default for DiffMetric {
    fn default() -> Self {
        Self::Ssim
    }
}

/// Options passed to [`diff_videos`].
#[derive(Debug, Clone)]
pub struct DiffOptions {
    /// Metric to apply per frame.
    pub metric: DiffMetric,
    /// Per-frame fail threshold. For `Pixelmatch`: fraction of differing
    /// pixels. For `Ssim`: 1.0 - mean_ssim. Lower threshold = stricter.
    pub threshold: f32,
    /// Optional region of interest in document pixels. If set, only this
    /// rect is compared. Selector-based masking via FrameSnapshot is a
    /// follow-on (it would require the comp.json + per-frame snapshot
    /// build, adding latency the diff loop doesn't otherwise pay).
    pub clip: Option<Rect>,
    /// Whole-video budget — fail if more than this fraction of frames
    /// exceeded `threshold`. 0.0 means "any failing frame fails the
    /// whole comparison."
    pub max_diff_ratio: f32,
}

impl Default for DiffOptions {
    fn default() -> Self {
        Self {
            metric: DiffMetric::default(),
            threshold: 0.05,
            clip: None,
            max_diff_ratio: 0.0,
        }
    }
}

/// Per-frame entry in the diff report.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FrameDiff {
    /// Frame index, 0-based.
    pub frame: u32,
    /// Metric score. Larger = more different. 0 means identical.
    pub score: f32,
    /// True when score > threshold.
    pub failed: bool,
}

/// Top-level diff result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiffResult {
    /// True when the whole comparison passed — no individual frame failure
    /// AND the cumulative failed-frame ratio is within `max_diff_ratio`.
    pub ok: bool,
    /// Metric used.
    pub metric: DiffMetric,
    /// Threshold used.
    pub threshold: f32,
    /// Number of frames compared (= min of the two videos' lengths).
    pub frames_compared: u32,
    /// Number of frames whose score exceeded `threshold`.
    pub frames_failed: u32,
    /// Median score across all frames.
    pub median_score: f32,
    /// 95th-percentile score.
    pub p95_score: f32,
    /// The single worst frame.
    pub worst: Option<FrameDiff>,
    /// Per-frame entries — useful for plotting / agent localization.
    /// Length matches `frames_compared`.
    pub per_frame: Vec<FrameDiff>,
}

/// Decode an MP4's video stream into a sequence of RGBA frames. Returns
/// `(width, height, frames)` where `frames[i]` is the i-th frame's RGBA
/// buffer `width*height*4` bytes long.
/// Decode every video frame from `path` to RGBA8 at the source resolution.
///
/// Returns `(width, height, fps, frames)`. `fps` is the source's
/// `avg_frame_rate` (or `r_frame_rate` when avg is missing), expressed
/// as a `f32`. Callers that don't care about timing can ignore it; the
/// offline renderer uses it to do time-based sampling so a 16fps Wan
/// clip composes correctly under a 30fps composition.
pub fn decode_rgba_frames(path: &Path) -> Result<(u32, u32, f32, Vec<Vec<u8>>), String> {
    let path_c =
        CString::new(path.to_string_lossy().into_owned()).map_err(|e| format!("path: {e}"))?;
    let mut input_ctx =
        AVFormatContextInput::open(&path_c).map_err(|e| format!("open: {e}"))?;

    // Find the first video stream.
    let video_stream_idx = input_ctx
        .streams()
        .iter()
        .position(|s| s.codecpar().codec_type == ffi::AVMEDIA_TYPE_VIDEO)
        .ok_or_else(|| format!("no video stream in {}", path.display()))?;
    let video_stream_idx = video_stream_idx as i32;

    let (codec_id, width, height, fps) = {
        let stream = &input_ctx.streams()[video_stream_idx as usize];
        let p = stream.codecpar();
        // Prefer `avg_frame_rate` (true average); fall back to
        // `r_frame_rate` (declared) when avg is missing.
        let avg = stream.avg_frame_rate;
        let r = stream.r_frame_rate;
        let pick = if avg.den != 0 && avg.num != 0 {
            avg
        } else {
            r
        };
        let fps = if pick.den != 0 {
            pick.num as f32 / pick.den as f32
        } else {
            30.0
        };
        (p.codec_id, p.width as u32, p.height as u32, fps)
    };

    let decoder =
        AVCodec::find_decoder(codec_id).ok_or_else(|| format!("no decoder for codec {codec_id}"))?;
    let mut dec_ctx = AVCodecContext::new(&decoder);
    {
        let stream = &input_ctx.streams()[video_stream_idx as usize];
        dec_ctx
            .apply_codecpar(&stream.codecpar())
            .map_err(|e| format!("apply_codecpar: {e}"))?;
    }
    dec_ctx.open(None).map_err(|e| format!("decoder open: {e}"))?;

    let mut sws = SwsContext::get_context(
        width as i32,
        height as i32,
        dec_ctx.pix_fmt,
        width as i32,
        height as i32,
        ffi::AV_PIX_FMT_RGBA,
        ffi::SWS_BILINEAR,
        None,
        None,
        None,
    )
    .ok_or_else(|| "sws_getContext failed".to_string())?;

    let mut frames: Vec<Vec<u8>> = Vec::new();
    loop {
        let packet_opt = input_ctx
            .read_packet()
            .map_err(|e| format!("read_packet: {e}"))?;
        let packet = match packet_opt {
            Some(p) => p,
            None => break, // EOF — fall through to flush.
        };
        if packet.stream_index != video_stream_idx {
            continue;
        }
        dec_ctx
            .send_packet(Some(&packet))
            .map_err(|e| format!("send_packet: {e}"))?;
        loop {
            match dec_ctx.receive_frame() {
                Ok(yuv) => frames.push(to_rgba(&mut sws, &yuv, width as i32, height as i32)?),
                Err(RsmpegError::DecoderDrainError) => break,
                Err(RsmpegError::DecoderFlushedError) => break,
                Err(e) => return Err(format!("receive_frame: {e}")),
            }
        }
    }

    // Flush.
    dec_ctx
        .send_packet(None)
        .map_err(|e| format!("flush send_packet: {e}"))?;
    loop {
        match dec_ctx.receive_frame() {
            Ok(yuv) => frames.push(to_rgba(&mut sws, &yuv, width as i32, height as i32)?),
            Err(RsmpegError::DecoderDrainError) => break,
            Err(RsmpegError::DecoderFlushedError) => break,
            Err(e) => return Err(format!("flush receive_frame: {e}")),
        }
    }

    Ok((width, height, fps, frames))
}

fn to_rgba(sws: &mut SwsContext, yuv: &AVFrame, w: i32, h: i32) -> Result<Vec<u8>, String> {
    let mut rgba = AVFrame::new();
    rgba.set_width(w);
    rgba.set_height(h);
    rgba.set_format(ffi::AV_PIX_FMT_RGBA);
    rgba.alloc_buffer().map_err(|e| format!("alloc: {e}"))?;
    sws.scale_frame(yuv, 0, h, &mut rgba)
        .map_err(|e| format!("sws scale: {e}"))?;

    let stride = rgba.linesize[0] as usize;
    let row_bytes = (w as usize) * 4;
    let mut out = Vec::with_capacity(row_bytes * h as usize);
    unsafe {
        for y in 0..h as usize {
            let row = std::slice::from_raw_parts(rgba.data[0].add(y * stride), row_bytes);
            out.extend_from_slice(row);
        }
    }
    Ok(out)
}

/// Compare two MP4s frame-by-frame per `opts`. Returns a structured result
/// with per-frame entries + aggregate stats. The shorter of the two videos
/// is the comparison length; surplus frames in the longer one are ignored.
pub fn diff_videos(a_path: &Path, b_path: &Path, opts: &DiffOptions) -> Result<DiffResult, String> {
    let (wa, ha, _fps_a, a_frames) = decode_rgba_frames(a_path)?;
    let (wb, hb, _fps_b, b_frames) = decode_rgba_frames(b_path)?;
    if (wa, ha) != (wb, hb) {
        return Err(format!(
            "dimensions differ: {}x{} vs {}x{}",
            wa, ha, wb, hb
        ));
    }
    let n = a_frames.len().min(b_frames.len()) as u32;

    let clip_rect = opts.clip.map(|r| clamp_rect(r, wa, ha));

    let mut per_frame = Vec::with_capacity(n as usize);
    let mut frames_failed = 0u32;
    for i in 0..n {
        let score = match opts.metric {
            DiffMetric::Pixelmatch => pixelmatch_score(
                &a_frames[i as usize],
                &b_frames[i as usize],
                wa,
                ha,
                clip_rect,
            ),
            DiffMetric::Ssim => ssim_score(
                &a_frames[i as usize],
                &b_frames[i as usize],
                wa,
                ha,
                clip_rect,
            ),
        };
        let failed = score > opts.threshold;
        if failed {
            frames_failed += 1;
        }
        per_frame.push(FrameDiff {
            frame: i,
            score,
            failed,
        });
    }

    let scores: Vec<f32> = per_frame.iter().map(|f| f.score).collect();
    let median = percentile(&scores, 0.5);
    let p95 = percentile(&scores, 0.95);
    let worst = per_frame
        .iter()
        .cloned()
        .max_by(|a, b| a.score.partial_cmp(&b.score).unwrap());

    let fail_ratio = if n > 0 {
        frames_failed as f32 / n as f32
    } else {
        0.0
    };
    let ok = fail_ratio <= opts.max_diff_ratio && frames_failed == 0;

    Ok(DiffResult {
        ok,
        metric: opts.metric,
        threshold: opts.threshold,
        frames_compared: n,
        frames_failed,
        median_score: median,
        p95_score: p95,
        worst,
        per_frame,
    })
}

/// Hand-rolled pixelmatch. Returns the fraction of pixels in the clip
/// region whose channel-summed RGB distance exceeds 36 (≈12/255 per channel).
/// Ignores alpha — most encode pipelines collapse alpha to opaque anyway.
pub fn pixelmatch_score(a: &[u8], b: &[u8], w: u32, h: u32, clip: Option<(u32, u32, u32, u32)>) -> f32 {
    let (x0, y0, x1, y1) = clip.unwrap_or((0, 0, w, h));
    let mut differing = 0u64;
    let mut total = 0u64;
    let thresh = 36; // sum of |dr|+|dg|+|db|
    for y in y0..y1 {
        for x in x0..x1 {
            let i = ((y * w + x) * 4) as usize;
            let d = (a[i] as i32 - b[i] as i32).unsigned_abs()
                + (a[i + 1] as i32 - b[i + 1] as i32).unsigned_abs()
                + (a[i + 2] as i32 - b[i + 2] as i32).unsigned_abs();
            if d > thresh {
                differing += 1;
            }
            total += 1;
        }
    }
    if total == 0 {
        0.0
    } else {
        differing as f32 / total as f32
    }
}

/// Hand-rolled SSIM on luminance. Wang et al. 2004, with the standard
/// constants K1=0.01, K2=0.03, L=255. Operates on 8x8 windows (no overlap
/// for speed; full-overlap reference impl uses 11x11 Gaussian and is ~10x
/// slower). Returns `1.0 - mean_ssim` so larger = more different.
pub fn ssim_score(a: &[u8], b: &[u8], w: u32, h: u32, clip: Option<(u32, u32, u32, u32)>) -> f32 {
    let (x0, y0, x1, y1) = clip.unwrap_or((0, 0, w, h));
    let ya: Vec<f32> = to_luma_region(a, w, x0, y0, x1, y1);
    let yb: Vec<f32> = to_luma_region(b, w, x0, y0, x1, y1);
    let region_w = (x1 - x0) as usize;
    let region_h = (y1 - y0) as usize;

    let c1 = (0.01_f32 * 255.0).powi(2);
    let c2 = (0.03_f32 * 255.0).powi(2);
    let win = 8usize;

    let mut sum = 0.0f64;
    let mut count = 0u64;
    let mut ix = 0;
    while ix + win <= region_w {
        let mut iy = 0;
        while iy + win <= region_h {
            let (mut sa, mut sb, mut s2a, mut s2b, mut sab) = (0.0f32, 0.0, 0.0, 0.0, 0.0);
            let n = (win * win) as f32;
            for dy in 0..win {
                for dx in 0..win {
                    let i = (iy + dy) * region_w + (ix + dx);
                    let va = ya[i];
                    let vb = yb[i];
                    sa += va;
                    sb += vb;
                    s2a += va * va;
                    s2b += vb * vb;
                    sab += va * vb;
                }
            }
            let mu_a = sa / n;
            let mu_b = sb / n;
            let sigma_a2 = s2a / n - mu_a * mu_a;
            let sigma_b2 = s2b / n - mu_b * mu_b;
            let sigma_ab = sab / n - mu_a * mu_b;
            let num = (2.0 * mu_a * mu_b + c1) * (2.0 * sigma_ab + c2);
            let den = (mu_a * mu_a + mu_b * mu_b + c1) * (sigma_a2 + sigma_b2 + c2);
            if den > 0.0 {
                sum += (num / den) as f64;
                count += 1;
            }
            iy += win;
        }
        ix += win;
    }
    if count == 0 {
        return 0.0;
    }
    let mean_ssim = (sum / count as f64) as f32;
    1.0 - mean_ssim.clamp(0.0, 1.0)
}

fn to_luma_region(rgba: &[u8], w: u32, x0: u32, y0: u32, x1: u32, y1: u32) -> Vec<f32> {
    let mut out = Vec::with_capacity(((x1 - x0) * (y1 - y0)) as usize);
    for y in y0..y1 {
        for x in x0..x1 {
            let i = ((y * w + x) * 4) as usize;
            // BT.709 luma; close to perceptual brightness.
            let l =
                0.2126 * rgba[i] as f32 + 0.7152 * rgba[i + 1] as f32 + 0.0722 * rgba[i + 2] as f32;
            out.push(l);
        }
    }
    out
}

fn clamp_rect(r: Rect, w: u32, h: u32) -> (u32, u32, u32, u32) {
    let x0 = r.x.max(0.0) as u32;
    let y0 = r.y.max(0.0) as u32;
    let x1 = ((r.x + r.w) as i32).min(w as i32).max(0) as u32;
    let y1 = ((r.y + r.h) as i32).min(h as i32).max(0) as u32;
    (x0, y0, x1.max(x0), y1.max(y0))
}

fn percentile(scores: &[f32], p: f32) -> f32 {
    if scores.is_empty() {
        return 0.0;
    }
    let mut sorted: Vec<f32> = scores.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let idx = ((sorted.len() as f32 - 1.0) * p) as usize;
    sorted[idx]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid(rgba: [u8; 4], w: u32, h: u32) -> Vec<u8> {
        rgba.iter().cycle().copied().take((w * h * 4) as usize).collect()
    }

    #[test]
    fn pixelmatch_identical_is_zero() {
        let a = solid([128, 128, 128, 255], 32, 32);
        let b = a.clone();
        assert_eq!(pixelmatch_score(&a, &b, 32, 32, None), 0.0);
    }

    #[test]
    fn pixelmatch_inverted_is_one() {
        let a = solid([0, 0, 0, 255], 32, 32);
        let b = solid([255, 255, 255, 255], 32, 32);
        assert_eq!(pixelmatch_score(&a, &b, 32, 32, None), 1.0);
    }

    #[test]
    fn pixelmatch_subthreshold_noise_is_zero() {
        // Two solid grays within 10/channel — under the 36 threshold.
        let a = solid([128, 128, 128, 255], 32, 32);
        let b = solid([135, 135, 135, 255], 32, 32);
        assert_eq!(pixelmatch_score(&a, &b, 32, 32, None), 0.0);
    }

    #[test]
    fn ssim_identical_is_zero() {
        let a = solid([100, 150, 200, 255], 32, 32);
        let b = a.clone();
        let s = ssim_score(&a, &b, 32, 32, None);
        assert!(s < 0.001, "expected ~0 for identical, got {s}");
    }

    #[test]
    fn ssim_inverted_is_large() {
        let a = solid([0, 0, 0, 255], 32, 32);
        let b = solid([255, 255, 255, 255], 32, 32);
        let s = ssim_score(&a, &b, 32, 32, None);
        assert!(s > 0.5, "expected large diff for black vs white, got {s}");
    }

    #[test]
    fn percentile_basic() {
        // Nearest-rank with idx = floor((n-1) * p):
        // 5 elements, p=0.5 → idx=2 → sorted[2] = 3.0
        // 5 elements, p=0.0 → idx=0 → 1.0
        // 5 elements, p=1.0 → idx=4 → 5.0
        // 5 elements, p=0.95 → idx=3 → 4.0
        assert_eq!(percentile(&[1.0, 2.0, 3.0, 4.0, 5.0], 0.5), 3.0);
        assert_eq!(percentile(&[1.0, 2.0, 3.0, 4.0, 5.0], 0.0), 1.0);
        assert_eq!(percentile(&[1.0, 2.0, 3.0, 4.0, 5.0], 1.0), 5.0);
        assert_eq!(percentile(&[1.0, 2.0, 3.0, 4.0, 5.0], 0.95), 4.0);
    }
}
