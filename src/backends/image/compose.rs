//! Pure-local image operations: apply-mask and composite-over.
//!
//! No backend calls. Used as the second step after text-prompted
//! segmentation (apply mask to original) and as the final step in
//! Path B (composite isolated subject over a generated environment).

use crate::backends::BackendError;
use image::{GenericImageView, ImageBuffer, Rgba, RgbaImage};
use std::path::Path;

/// Apply a grayscale mask to the source image, producing an RGBA PNG
/// where pixels outside the mask are fully transparent. The mask is
/// resized to match the source if dimensions differ.
///
/// Used as the second step after `fal-ai/evf-sam`, which returns a
/// grayscale mask rather than an alpha-cut PNG.
pub fn apply_mask(
    source: &Path,
    mask: &Path,
    out: &Path,
) -> Result<(u32, u32), BackendError> {
    let src = image::open(source)
        .map_err(|e| BackendError::Decode(format!("open source {}: {e}", source.display())))?;
    let msk = image::open(mask)
        .map_err(|e| BackendError::Decode(format!("open mask {}: {e}", mask.display())))?;

    let (w, h) = src.dimensions();
    let msk = if msk.dimensions() != (w, h) {
        image::imageops::resize(&msk.to_luma8(), w, h, image::imageops::FilterType::Triangle)
    } else {
        msk.to_luma8()
    };

    let src_rgba = src.to_rgba8();
    let mut out_img: RgbaImage = ImageBuffer::new(w, h);
    for (x, y, pixel) in out_img.enumerate_pixels_mut() {
        let s = src_rgba.get_pixel(x, y);
        let m = msk.get_pixel(x, y).0[0];
        *pixel = Rgba([s.0[0], s.0[1], s.0[2], m]);
    }
    out_img
        .save(out)
        .map_err(|e| BackendError::Cache(format!("write {}: {e}", out.display())))?;
    Ok((w, h))
}

/// Composite a foreground RGBA image over a background image, returning
/// the result as an RGBA PNG. The foreground is centered and scaled to
/// fit within the background; the background fills the whole frame.
///
/// `fg_scale` controls how big the foreground appears relative to the
/// background height (0.6 = 60% of bg height). Defaults to 0.7 if zero.
pub fn composite_over(
    foreground: &Path,
    background: &Path,
    out: &Path,
    fg_scale: f32,
    y_offset_frac: f32,
) -> Result<(u32, u32), BackendError> {
    let fg = image::open(foreground)
        .map_err(|e| BackendError::Decode(format!("open fg {}: {e}", foreground.display())))?;
    let bg = image::open(background)
        .map_err(|e| BackendError::Decode(format!("open bg {}: {e}", background.display())))?;

    let (bw, bh) = bg.dimensions();
    let scale = if fg_scale <= 0.0 { 0.7 } else { fg_scale };
    let target_h = (bh as f32 * scale).round() as u32;
    let (fw, fh) = fg.dimensions();
    let target_w = ((target_h as f32) * (fw as f32) / (fh as f32)).round() as u32;
    let fg_resized = image::imageops::resize(
        &fg.to_rgba8(),
        target_w.max(1),
        target_h.max(1),
        image::imageops::FilterType::Lanczos3,
    );

    let mut out_img = bg.to_rgba8();
    // Center horizontally; bias vertically by y_offset_frac (0.5 = vertical center).
    let off_y_frac = if y_offset_frac == 0.0 { 0.55 } else { y_offset_frac };
    let x0 = (bw as i64 - target_w as i64) / 2;
    let y0 = ((bh as f32) * off_y_frac - (target_h as f32) / 2.0).round() as i64;

    for (fx, fy, fp) in fg_resized.enumerate_pixels() {
        let dx = x0 + fx as i64;
        let dy = y0 + fy as i64;
        if dx < 0 || dy < 0 || dx >= bw as i64 || dy >= bh as i64 {
            continue;
        }
        let alpha = fp.0[3] as u32;
        if alpha == 0 {
            continue;
        }
        let inv = 255 - alpha;
        let dst = out_img.get_pixel_mut(dx as u32, dy as u32);
        dst.0[0] = ((fp.0[0] as u32 * alpha + dst.0[0] as u32 * inv) / 255) as u8;
        dst.0[1] = ((fp.0[1] as u32 * alpha + dst.0[1] as u32 * inv) / 255) as u8;
        dst.0[2] = ((fp.0[2] as u32 * alpha + dst.0[2] as u32 * inv) / 255) as u8;
        dst.0[3] = 255;
    }

    out_img
        .save(out)
        .map_err(|e| BackendError::Cache(format!("write {}: {e}", out.display())))?;
    Ok((bw, bh))
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{ImageBuffer, Luma, Rgba};

    fn tmp(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("wavelet-compose-test-{name}"))
    }

    #[test]
    fn apply_mask_writes_alpha() {
        let src_path = tmp("src.png");
        let mask_path = tmp("mask.png");
        let out_path = tmp("out.png");
        let _ = std::fs::remove_file(&src_path);
        let _ = std::fs::remove_file(&mask_path);
        let _ = std::fs::remove_file(&out_path);

        let src: RgbaImage = ImageBuffer::from_fn(8, 8, |_, _| Rgba([200, 50, 50, 255]));
        src.save(&src_path).unwrap();
        let mask: ImageBuffer<Luma<u8>, Vec<u8>> = ImageBuffer::from_fn(8, 8, |x, _| {
            if x < 4 { Luma([255]) } else { Luma([0]) }
        });
        mask.save(&mask_path).unwrap();

        apply_mask(&src_path, &mask_path, &out_path).unwrap();
        let out = image::open(&out_path).unwrap().to_rgba8();
        assert_eq!(out.get_pixel(0, 0).0[3], 255);
        assert_eq!(out.get_pixel(7, 0).0[3], 0);
    }

    #[test]
    fn composite_over_blends_with_alpha() {
        let fg_path = tmp("fg.png");
        let bg_path = tmp("bg.png");
        let out_path = tmp("out2.png");
        let _ = std::fs::remove_file(&fg_path);
        let _ = std::fs::remove_file(&bg_path);
        let _ = std::fs::remove_file(&out_path);

        // FG: 50x50 fully opaque red center, transparent everywhere else.
        let fg: RgbaImage = ImageBuffer::from_fn(50, 50, |x, y| {
            if x >= 20 && x < 30 && y >= 20 && y < 30 {
                Rgba([255, 0, 0, 255])
            } else {
                Rgba([0, 0, 0, 0])
            }
        });
        fg.save(&fg_path).unwrap();
        // BG: 200x200 solid blue.
        let bg: RgbaImage = ImageBuffer::from_fn(200, 200, |_, _| Rgba([0, 0, 200, 255]));
        bg.save(&bg_path).unwrap();

        composite_over(&fg_path, &bg_path, &out_path, 0.5, 0.5).unwrap();
        let out = image::open(&out_path).unwrap().to_rgba8();
        let (w, h) = out.dimensions();
        assert_eq!((w, h), (200, 200));
        // Center should have red overlaid; corners should still be blue.
        let center = out.get_pixel(100, 100);
        let corner = out.get_pixel(0, 0);
        assert_eq!(corner.0, [0, 0, 200, 255], "corner should stay bg blue");
        assert!(
            center.0[0] > 100,
            "center should be reddish from fg overlay, got {:?}",
            center.0
        );
    }
}
