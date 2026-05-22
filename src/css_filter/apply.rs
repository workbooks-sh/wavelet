//! CPU-side filter application — full-frame and bbox-scoped.

#![allow(missing_docs)]

use super::types::{FilterFn, LengthUnit};

pub fn apply_chain_cpu(
    buffer: &mut [u8],
    width: u32,
    height: u32,
    chain: &[FilterFn],
    viewport_w: f32,
    viewport_h: f32,
) {
    if chain.is_empty() {
        return;
    }

    // Collapse the chain into amount values + optional blur sigma.
    // Same shape as `shader::filter_pass::ChainPlan` — duplicated
    // for now because the CPU path doesn't link the GPU module.
    // De-dup when the GPU path lands as the primary execution venue.
    let mut brightness: f32 = 1.0;
    let mut contrast: f32 = 1.0;
    let mut saturate_amt: f32 = 1.0;
    let mut grayscale: f32 = 0.0;
    let mut sepia: f32 = 0.0;
    let mut invert: f32 = 0.0;
    let mut opacity: f32 = 1.0;
    let mut hue_rad: f32 = 0.0;
    let mut blur_sigma2: f32 = 0.0; // accumulated variance

    for f in chain {
        match f {
            FilterFn::Brightness(v) => brightness *= v,
            FilterFn::Contrast(v) => contrast *= v,
            FilterFn::Saturate(v) => saturate_amt *= v,
            FilterFn::Grayscale(v) => grayscale = grayscale.max(*v).min(1.0),
            FilterFn::Sepia(v) => sepia = sepia.max(*v).min(1.0),
            FilterFn::Invert(v) => invert = invert.max(*v).min(1.0),
            FilterFn::Opacity(v) => opacity *= v,
            FilterFn::HueRotate(deg) => hue_rad += deg.to_radians(),
            FilterFn::Blur(l) => {
                let px = l.to_px(viewport_w, viewport_h, 16.0);
                blur_sigma2 += px * px;
            }
            FilterFn::DropShadow { .. } => {
                eprintln!(
                    "warning: css filter: drop-shadow not yet supported \
                     in the wavelet-fx CPU path; skipping. Effect will \
                     render without the shadow."
                );
            }
        }
    }

    // Blur first (so per-pixel grading reads the blurred result).
    if blur_sigma2 > 0.0 {
        let sigma = blur_sigma2.sqrt();
        crate::shader::gaussian_rgba(buffer, width, height, sigma);
    }

    // Skip the per-pixel loop entirely if we're at identity.
    let need_per_pixel = brightness != 1.0
        || contrast != 1.0
        || saturate_amt != 1.0
        || grayscale > 0.0
        || sepia > 0.0
        || invert > 0.0
        || opacity != 1.0
        || hue_rad != 0.0;
    if !need_per_pixel {
        return;
    }

    // Precompute hue-rotate trig + sepia matrix rows so the per-pixel
    // loop stays inner-loop-friendly.
    let cos_a = hue_rad.cos();
    let sin_a = hue_rad.sin();
    // YIQ rotation matrix coefficients — same as the WGSL version in
    // shader::filter_pass.
    let hue_rr = 0.213 + cos_a * 0.787 - sin_a * 0.213;
    let hue_rg = 0.715 - cos_a * 0.715 - sin_a * 0.715;
    let hue_rb = 0.072 - cos_a * 0.072 + sin_a * 0.928;
    let hue_gr = 0.213 - cos_a * 0.213 + sin_a * 0.143;
    let hue_gg = 0.715 + cos_a * 0.285 + sin_a * 0.140;
    let hue_gb = 0.072 - cos_a * 0.072 - sin_a * 0.283;
    let hue_br = 0.213 - cos_a * 0.213 - sin_a * 0.787;
    let hue_bg = 0.715 - cos_a * 0.715 + sin_a * 0.715;
    let hue_bb = 0.072 + cos_a * 0.928 + sin_a * 0.072;

    let n = (width as usize) * (height as usize);
    for i in 0..n {
        let off = i * 4;
        let mut r = buffer[off] as f32 / 255.0;
        let mut g = buffer[off + 1] as f32 / 255.0;
        let mut b = buffer[off + 2] as f32 / 255.0;
        let a = buffer[off + 3] as f32 / 255.0;

        // brightness
        if brightness != 1.0 {
            r *= brightness;
            g *= brightness;
            b *= brightness;
        }
        // contrast (around 0.5 grey)
        if contrast != 1.0 {
            r = (r - 0.5) * contrast + 0.5;
            g = (g - 0.5) * contrast + 0.5;
            b = (b - 0.5) * contrast + 0.5;
        }
        // saturate (mix between luma and color)
        if saturate_amt != 1.0 {
            let l = 0.2126 * r + 0.7152 * g + 0.0722 * b;
            r = l + (r - l) * saturate_amt;
            g = l + (g - l) * saturate_amt;
            b = l + (b - l) * saturate_amt;
        }
        // grayscale (mix toward luma)
        if grayscale > 0.0 {
            let l = 0.2126 * r + 0.7152 * g + 0.0722 * b;
            r = r + (l - r) * grayscale;
            g = g + (l - g) * grayscale;
            b = b + (l - b) * grayscale;
        }
        // sepia
        if sepia > 0.0 {
            let sr = 0.393 * r + 0.769 * g + 0.189 * b;
            let sg = 0.349 * r + 0.686 * g + 0.168 * b;
            let sb = 0.272 * r + 0.534 * g + 0.131 * b;
            r = r + (sr - r) * sepia;
            g = g + (sg - g) * sepia;
            b = b + (sb - b) * sepia;
        }
        // invert
        if invert > 0.0 {
            let ir = 1.0 - r;
            let ig = 1.0 - g;
            let ib = 1.0 - b;
            r = r + (ir - r) * invert;
            g = g + (ig - g) * invert;
            b = b + (ib - b) * invert;
        }
        // hue-rotate (after grayscale/sepia/invert so it acts on whatever
        // chroma is left, matching CSS evaluation order)
        if hue_rad != 0.0 {
            let nr = r * hue_rr + g * hue_rg + b * hue_rb;
            let ng = r * hue_gr + g * hue_gg + b * hue_gb;
            let nb = r * hue_br + g * hue_bg + b * hue_bb;
            r = nr;
            g = ng;
            b = nb;
        }

        let a_final = a * opacity;
        buffer[off] = (r.clamp(0.0, 1.0) * 255.0) as u8;
        buffer[off + 1] = (g.clamp(0.0, 1.0) * 255.0) as u8;
        buffer[off + 2] = (b.clamp(0.0, 1.0) * 255.0) as u8;
        buffer[off + 3] = (a_final.clamp(0.0, 1.0) * 255.0) as u8;
    }
}

/// Apply a filter chain to a rectangular sub-region of an RGBA8 buffer
/// in place. Coordinates are absolute (in the buffer's pixel space);
/// out-of-bounds bbox is clipped to the buffer. No-op for empty chain
/// or zero-size bbox.
///
/// For blur, the implementation crops the bbox region into a scratch
/// buffer, blurs it, and writes back. Edge convolution uses clamp-to-
/// border (same as the full-frame `gaussian_rgba` primitive). Per-pixel
/// ops run row-by-row over the bbox slice without any copy.
pub fn apply_chain_cpu_bbox(
    buffer: &mut [u8],
    buf_w: u32,
    buf_h: u32,
    bbox_x: i32,
    bbox_y: i32,
    bbox_w: u32,
    bbox_h: u32,
    chain: &[FilterFn],
    viewport_w: f32,
    viewport_h: f32,
) {
    if chain.is_empty() || bbox_w == 0 || bbox_h == 0 {
        return;
    }
    // Clip the bbox to the buffer extents.
    let x0 = bbox_x.max(0) as u32;
    let y0 = bbox_y.max(0) as u32;
    let x1 = ((bbox_x + bbox_w as i32).max(0) as u32).min(buf_w);
    let y1 = ((bbox_y + bbox_h as i32).max(0) as u32).min(buf_h);
    if x1 <= x0 || y1 <= y0 {
        return;
    }
    let region_w = x1 - x0;
    let region_h = y1 - y0;

    // Crop region into a scratch buffer so blur + per-pixel ops can
    // operate on contiguous memory.
    let mut region = vec![0u8; (region_w * region_h * 4) as usize];
    for ry in 0..region_h {
        let src_off = (((y0 + ry) * buf_w + x0) * 4) as usize;
        let dst_off = (ry * region_w * 4) as usize;
        let row_bytes = (region_w * 4) as usize;
        region[dst_off..dst_off + row_bytes]
            .copy_from_slice(&buffer[src_off..src_off + row_bytes]);
    }

    // Reuse the full-frame chain apply on the cropped region.
    apply_chain_cpu(&mut region, region_w, region_h, chain, viewport_w, viewport_h);

    // Copy filtered region back into the buffer at the bbox position.
    for ry in 0..region_h {
        let src_off = (ry * region_w * 4) as usize;
        let dst_off = (((y0 + ry) * buf_w + x0) * 4) as usize;
        let row_bytes = (region_w * 4) as usize;
        buffer[dst_off..dst_off + row_bytes]
            .copy_from_slice(&region[src_off..src_off + row_bytes]);
    }
}
