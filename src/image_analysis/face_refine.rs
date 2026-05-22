//! Pure-local helpers for the face-crop refine paste-back pipeline
//! (HelloRob template). Three primitives:
//!
//! 1. [`crop_with_padding`] — extract a face region at the detector's
//!    bbox, padded by a configurable fraction on each side. Returns the
//!    crop *and* the padded bbox in original-image coordinates so the
//!    caller can paste it back exactly where it came from.
//! 2. [`grow_and_blur_mask`] — dilate a binary face-shaped alpha mask
//!    by N pixels and Gaussian-blur the edges, so the alpha-blended
//!    paste-back fades into the surrounding skin instead of leaving a
//!    visible seam.
//! 3. [`paste_back`] — alpha-composite a refined crop back onto the
//!    original at a given bbox using the blurred mask as the alpha
//!    channel.
//!
//! No external services, no GPU. The Gaussian blur is a separable 1D
//! pass over the alpha channel (independent of the existing
//! `shader::blur::gaussian_rgba` so we can keep this module free of
//! the wgpu-flavored dependency stack).

use image::{DynamicImage, GenericImageView, GrayImage, ImageBuffer, Luma, Rgba, RgbaImage};

/// Crop a face region with padding. `bbox` is `[x, y, w, h]` in the
/// original image's pixel coordinates (top-left origin). `pad_pct` is
/// the fraction of the bbox's width/height to add on each side — e.g.
/// `0.15` grows a 200×200 box to ~260×260. Returns the crop image and
/// the padded bbox in the original's coordinates so callers can paste
/// the refined crop back without re-deriving the offset.
///
/// The padded bbox is clamped to the image bounds — when the face is
/// near an edge, padding shrinks on that side rather than throwing.
pub fn crop_with_padding(
    image: &DynamicImage,
    bbox: [u32; 4],
    pad_pct: f32,
) -> (DynamicImage, [u32; 4]) {
    let (img_w, img_h) = image.dimensions();
    let [x, y, w, h] = bbox;
    let pad_w = ((w as f32) * pad_pct.max(0.0)).round() as i64;
    let pad_h = ((h as f32) * pad_pct.max(0.0)).round() as i64;
    let nx = (x as i64 - pad_w).max(0) as u32;
    let ny = (y as i64 - pad_h).max(0) as u32;
    let mut nw = w + (2.0 * pad_w as f32).round() as u32;
    let mut nh = h + (2.0 * pad_h as f32).round() as u32;
    if nx + nw > img_w {
        nw = img_w.saturating_sub(nx);
    }
    if ny + nh > img_h {
        nh = img_h.saturating_sub(ny);
    }
    if nw == 0 || nh == 0 {
        // Degenerate bbox — return a 1×1 black pixel and a zero-area
        // box so callers don't have to special-case None.
        return (
            DynamicImage::ImageRgba8(ImageBuffer::new(1, 1)),
            [nx.min(img_w.saturating_sub(1)), ny.min(img_h.saturating_sub(1)), 1, 1],
        );
    }
    let crop = image.crop_imm(nx, ny, nw, nh);
    (crop, [nx, ny, nw, nh])
}

/// Build a soft-edged grayscale alpha mask for the paste-back. Starts
/// from a centered ellipse covering most of the crop (faces are
/// roughly oval), dilates by `grow_px` pixels, then runs a separable
/// 1D Gaussian blur of `blur_radius` over the result. The output is
/// the alpha channel the paste-back uses — 0 = keep original,
/// 255 = take refined crop, gradient in between for the soft fade.
///
/// `width` × `height` is the crop's dimensions. The mask is always
/// returned at the same dimensions; resizing for paste-back is the
/// caller's responsibility if the crop got rescaled in transit.
pub fn build_face_mask(width: u32, height: u32, grow_px: u32, blur_radius: f32) -> GrayImage {
    let mut mask: GrayImage = ImageBuffer::new(width, height);
    // Centered ellipse covering ~80% of each axis. The mask doesn't
    // need to hug the face — paste-back is forgiving as long as the
    // blur radius is generous enough.
    let cx = (width as f32) * 0.5;
    let cy = (height as f32) * 0.5;
    let rx = (width as f32) * 0.40;
    let ry = (height as f32) * 0.45;
    for y in 0..height {
        for x in 0..width {
            let dx = (x as f32 - cx) / rx;
            let dy = (y as f32 - cy) / ry;
            let inside = dx * dx + dy * dy <= 1.0;
            mask.put_pixel(x, y, Luma([if inside { 255 } else { 0 }]));
        }
    }
    let mask = dilate_grow(&mask, grow_px);
    blur_gray(&mask, blur_radius)
}

/// Dilate (grow) a grayscale mask by `radius` pixels using a
/// max-filter — every output pixel becomes the max of its square
/// neighborhood. Two separable passes (horizontal max, vertical max)
/// keep this cheap. Returns a new buffer; the input is not modified.
///
/// When `radius` is 0 the input is returned unchanged.
pub fn dilate_grow(mask: &GrayImage, radius: u32) -> GrayImage {
    if radius == 0 {
        return mask.clone();
    }
    let (w, h) = mask.dimensions();
    let r = radius as i32;
    let mut tmp: GrayImage = ImageBuffer::new(w, h);
    // Horizontal pass.
    for y in 0..h {
        for x in 0..w {
            let mut best = 0u8;
            let x0 = (x as i32 - r).max(0);
            let x1 = (x as i32 + r).min(w as i32 - 1);
            for sx in x0..=x1 {
                let v = mask.get_pixel(sx as u32, y).0[0];
                if v > best {
                    best = v;
                }
            }
            tmp.put_pixel(x, y, Luma([best]));
        }
    }
    // Vertical pass.
    let mut out: GrayImage = ImageBuffer::new(w, h);
    for x in 0..w {
        for y in 0..h {
            let mut best = 0u8;
            let y0 = (y as i32 - r).max(0);
            let y1 = (y as i32 + r).min(h as i32 - 1);
            for sy in y0..=y1 {
                let v = tmp.get_pixel(x, sy as u32).0[0];
                if v > best {
                    best = v;
                }
            }
            out.put_pixel(x, y, Luma([best]));
        }
    }
    out
}

/// Separable 1D Gaussian blur over a grayscale image. `sigma` is the
/// standard deviation in pixels. When `sigma <= 0` the input is
/// returned unchanged. Kernel half-width is `ceil(3σ)` — that captures
/// >99% of the weight without paying for the long tails.
pub fn blur_gray(mask: &GrayImage, sigma: f32) -> GrayImage {
    if sigma <= 0.0 {
        return mask.clone();
    }
    let (w, h) = mask.dimensions();
    let kernel = build_kernel(sigma);
    let kr = (kernel.len() / 2) as i32;
    // Horizontal pass into an f32 scratch buffer.
    let mut scratch = vec![0.0f32; (w * h) as usize];
    for y in 0..h {
        for x in 0..w {
            let mut acc = 0.0f32;
            for k in -kr..=kr {
                let sx = (x as i32 + k).clamp(0, w as i32 - 1) as u32;
                let v = mask.get_pixel(sx, y).0[0] as f32;
                acc += v * kernel[(k + kr) as usize];
            }
            scratch[(y * w + x) as usize] = acc;
        }
    }
    // Vertical pass into the output buffer.
    let mut out: GrayImage = ImageBuffer::new(w, h);
    for y in 0..h {
        for x in 0..w {
            let mut acc = 0.0f32;
            for k in -kr..=kr {
                let sy = (y as i32 + k).clamp(0, h as i32 - 1) as u32;
                acc += scratch[(sy * w + x) as usize] * kernel[(k + kr) as usize];
            }
            out.put_pixel(x, y, Luma([acc.clamp(0.0, 255.0) as u8]));
        }
    }
    out
}

/// Alpha-composite the `refined_crop` back onto the `original` at the
/// `bbox` location, using `mask` as the alpha channel. The mask is
/// resampled (bilinear via Triangle) to match the crop dimensions if
/// they differ.
///
/// Coordinate convention: `bbox` is `[x, y, w, h]` in `original`'s
/// pixel coords. The crop is expected to be that size; if it isn't,
/// it's resized to match before blending.
///
/// Output is RGBA. The original's alpha (if any) is preserved.
pub fn paste_back(
    original: DynamicImage,
    refined_crop: DynamicImage,
    bbox: [u32; 4],
    mask: GrayImage,
) -> DynamicImage {
    let [bx, by, bw, bh] = bbox;
    let (img_w, img_h) = original.dimensions();
    let mut out: RgbaImage = original.to_rgba8();
    if bw == 0 || bh == 0 || bx >= img_w || by >= img_h {
        return DynamicImage::ImageRgba8(out);
    }
    // Resize crop + mask to the bbox dimensions so the paste-back math
    // doesn't need to special-case rescales.
    let crop_rgba = if refined_crop.dimensions() != (bw, bh) {
        image::imageops::resize(
            &refined_crop.to_rgba8(),
            bw,
            bh,
            image::imageops::FilterType::Lanczos3,
        )
    } else {
        refined_crop.to_rgba8()
    };
    let mask = if mask.dimensions() != (bw, bh) {
        image::imageops::resize(&mask, bw, bh, image::imageops::FilterType::Triangle)
    } else {
        mask
    };
    let last_x = bx.saturating_add(bw).min(img_w);
    let last_y = by.saturating_add(bh).min(img_h);
    for y in by..last_y {
        for x in bx..last_x {
            let cx = x - bx;
            let cy = y - by;
            let a_mask = mask.get_pixel(cx, cy).0[0] as f32 / 255.0;
            let src = out.get_pixel(x, y).0;
            let new = crop_rgba.get_pixel(cx, cy).0;
            let r = lerp_u8(src[0], new[0], a_mask);
            let g = lerp_u8(src[1], new[1], a_mask);
            let b = lerp_u8(src[2], new[2], a_mask);
            let a = src[3].max(new[3]);
            out.put_pixel(x, y, Rgba([r, g, b, a]));
        }
    }
    DynamicImage::ImageRgba8(out)
}

/// Linear interpolation between two u8 channels by an alpha in `[0,1]`.
fn lerp_u8(a: u8, b: u8, t: f32) -> u8 {
    let t = t.clamp(0.0, 1.0);
    let v = a as f32 * (1.0 - t) + b as f32 * t;
    v.clamp(0.0, 255.0).round() as u8
}

/// Normalized 1D Gaussian kernel with half-width `ceil(3σ)`. Sum of
/// weights = 1.0 within float precision.
fn build_kernel(sigma: f32) -> Vec<f32> {
    let radius = (sigma * 3.0).ceil() as i32;
    let n = (2 * radius + 1) as usize;
    let mut k = Vec::with_capacity(n);
    let two_sigma_sq = 2.0 * sigma * sigma;
    let norm = 1.0 / (std::f32::consts::TAU.sqrt() * sigma);
    for i in -radius..=radius {
        let x = i as f32;
        k.push(norm * (-x * x / two_sigma_sq).exp());
    }
    let s: f32 = k.iter().sum();
    if s > 0.0 {
        for v in &mut k {
            *v /= s;
        }
    }
    k
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgb;

    fn solid_rgb(w: u32, h: u32, color: [u8; 3]) -> DynamicImage {
        let mut img: image::RgbImage = ImageBuffer::new(w, h);
        for p in img.pixels_mut() {
            *p = Rgb(color);
        }
        DynamicImage::ImageRgb8(img)
    }

    #[test]
    fn crop_with_padding_stays_in_bounds_near_top_left() {
        let img = solid_rgb(100, 100, [128, 128, 128]);
        // Face flush against top-left, pad 0.5 → pad would shoot past
        // the corner. Result must clamp to 0,0 and shrink width/height.
        let (crop, [x, y, w, h]) = crop_with_padding(&img, [0, 0, 20, 20], 0.5);
        assert_eq!((x, y), (0, 0));
        // Padding only adds on the right and bottom; left/top can't.
        assert!(w <= 100 && h <= 100);
        assert!(w >= 20 && h >= 20);
        let (cw, ch) = crop.dimensions();
        assert_eq!((cw, ch), (w, h));
    }

    #[test]
    fn crop_with_padding_stays_in_bounds_near_bottom_right() {
        let img = solid_rgb(100, 100, [128, 128, 128]);
        let (_, [x, y, w, h]) = crop_with_padding(&img, [80, 80, 20, 20], 0.5);
        assert!(x + w <= 100);
        assert!(y + h <= 100);
    }

    #[test]
    fn crop_with_padding_centered_grows_correctly() {
        let img = solid_rgb(200, 200, [0, 0, 0]);
        let (_, [x, y, w, h]) = crop_with_padding(&img, [50, 50, 100, 100], 0.15);
        // 100 × 0.15 = 15 pad each side ⇒ 130×130 box.
        assert_eq!((x, y, w, h), (35, 35, 130, 130));
    }

    #[test]
    fn crop_with_padding_zero_pct_returns_exact_bbox() {
        let img = solid_rgb(200, 200, [0, 0, 0]);
        let (_, [x, y, w, h]) = crop_with_padding(&img, [50, 50, 100, 100], 0.0);
        assert_eq!((x, y, w, h), (50, 50, 100, 100));
    }

    #[test]
    fn dilate_grow_zero_radius_is_passthrough() {
        let mut m: GrayImage = ImageBuffer::new(5, 5);
        m.put_pixel(2, 2, Luma([255]));
        let d = dilate_grow(&m, 0);
        for (a, b) in m.pixels().zip(d.pixels()) {
            assert_eq!(a.0[0], b.0[0]);
        }
    }

    #[test]
    fn dilate_grow_expands_single_pixel_by_radius() {
        let mut m: GrayImage = ImageBuffer::new(10, 10);
        m.put_pixel(5, 5, Luma([255]));
        let d = dilate_grow(&m, 2);
        // Every pixel within Chebyshev distance 2 should be 255.
        for y in 3..=7 {
            for x in 3..=7 {
                assert_eq!(d.get_pixel(x, y).0[0], 255, "({x},{y})");
            }
        }
        // And pixels at Chebyshev distance > 2 should still be 0.
        assert_eq!(d.get_pixel(0, 0).0[0], 0);
        assert_eq!(d.get_pixel(9, 9).0[0], 0);
    }

    #[test]
    fn blur_gray_zero_sigma_is_passthrough() {
        let mut m: GrayImage = ImageBuffer::new(5, 5);
        m.put_pixel(2, 2, Luma([200]));
        let b = blur_gray(&m, 0.0);
        assert_eq!(b.get_pixel(2, 2).0[0], 200);
        assert_eq!(b.get_pixel(0, 0).0[0], 0);
    }

    #[test]
    fn blur_gray_spreads_a_single_bright_pixel() {
        let mut m: GrayImage = ImageBuffer::new(20, 20);
        m.put_pixel(10, 10, Luma([255]));
        let b = blur_gray(&m, 2.0);
        // Center peak got attenuated.
        assert!(b.get_pixel(10, 10).0[0] < 255);
        // Neighbors picked up some brightness.
        assert!(b.get_pixel(11, 10).0[0] > 0);
        assert!(b.get_pixel(10, 11).0[0] > 0);
        // Far corner stays dark.
        assert_eq!(b.get_pixel(0, 0).0[0], 0);
    }

    #[test]
    fn build_face_mask_is_brightest_at_center() {
        let m = build_face_mask(100, 100, 4, 3.0);
        let center = m.get_pixel(50, 50).0[0];
        let corner = m.get_pixel(0, 0).0[0];
        assert!(center > corner);
        assert!(center > 200, "center {center}");
        assert!(corner < 32, "corner {corner}");
    }

    #[test]
    fn build_face_mask_edges_are_soft() {
        // After blur there should be at least some pixels in the
        // gradient range — proving the edge faded rather than stayed
        // binary.
        let m = build_face_mask(80, 80, 4, 3.0);
        let intermediate_count = m
            .pixels()
            .filter(|p| {
                let v = p.0[0];
                v > 16 && v < 240
            })
            .count();
        assert!(
            intermediate_count > 100,
            "expected a soft gradient, got {intermediate_count} mid-tone pixels"
        );
    }

    #[test]
    fn paste_back_with_all_one_mask_replaces_region() {
        let original = solid_rgb(50, 50, [0, 0, 0]);
        let refined = solid_rgb(20, 20, [255, 0, 0]);
        let mut full: GrayImage = ImageBuffer::new(20, 20);
        for p in full.pixels_mut() {
            *p = Luma([255]);
        }
        let out = paste_back(original, refined, [15, 15, 20, 20], full);
        let rgba = out.to_rgba8();
        // Inside bbox: red.
        let p = rgba.get_pixel(20, 20).0;
        assert_eq!((p[0], p[1], p[2]), (255, 0, 0));
        // Outside bbox: still black.
        let q = rgba.get_pixel(0, 0).0;
        assert_eq!((q[0], q[1], q[2]), (0, 0, 0));
    }

    #[test]
    fn paste_back_with_zero_mask_preserves_original() {
        let original = solid_rgb(50, 50, [10, 20, 30]);
        let refined = solid_rgb(20, 20, [255, 255, 255]);
        let zero: GrayImage = ImageBuffer::new(20, 20);
        let out = paste_back(original, refined, [15, 15, 20, 20], zero);
        let rgba = out.to_rgba8();
        // Every pixel must still match the original color.
        for p in rgba.pixels() {
            assert_eq!((p.0[0], p.0[1], p.0[2]), (10, 20, 30));
        }
    }

    #[test]
    fn paste_back_round_trip_identical_crop_is_near_identical() {
        // Crop the center of the image, paste back unchanged, output
        // should match the input pixel-for-pixel inside the bbox.
        let mut original: image::RgbImage = ImageBuffer::new(60, 60);
        for (x, y, p) in original.enumerate_pixels_mut() {
            *p = Rgb([(x * 4) as u8, (y * 4) as u8, 128]);
        }
        let original = DynamicImage::ImageRgb8(original);
        let crop = original.crop_imm(20, 20, 20, 20);
        let mut full_mask: GrayImage = ImageBuffer::new(20, 20);
        for p in full_mask.pixels_mut() {
            *p = Luma([255]);
        }
        let out = paste_back(original.clone(), crop, [20, 20, 20, 20], full_mask);
        let in_rgba = original.to_rgba8();
        let out_rgba = out.to_rgba8();
        for y in 20..40 {
            for x in 20..40 {
                let a = in_rgba.get_pixel(x, y).0;
                let b = out_rgba.get_pixel(x, y).0;
                assert_eq!((a[0], a[1], a[2]), (b[0], b[1], b[2]), "({x},{y})");
            }
        }
    }

    #[test]
    fn paste_back_with_oversize_bbox_clamps() {
        // bbox extends past the image — should still produce valid
        // output (no panic) and only touch pixels inside the image.
        let original = solid_rgb(30, 30, [0, 0, 0]);
        let refined = solid_rgb(20, 20, [255, 255, 255]);
        let mut full_mask: GrayImage = ImageBuffer::new(20, 20);
        for p in full_mask.pixels_mut() {
            *p = Luma([255]);
        }
        let out = paste_back(original, refined, [20, 20, 20, 20], full_mask);
        assert_eq!(out.dimensions(), (30, 30));
    }
}
