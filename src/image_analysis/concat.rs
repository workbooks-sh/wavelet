//! Horizontal concat of two stills into a single RGB image, used as
//! the local pre-step for the `wavelet shot insert-into-scene` verb.
//!
//! The Insert-Anything spike (wb-j1ef.2) showed that Kontext Max can
//! merge a product into a scene when given a side-by-side input plus
//! an instruction that names which half is which. That requires:
//!   - Both halves at matching height (Kontext is scale-sensitive).
//!   - Solid RGB output — alpha on transparent product cutouts
//!     becomes a black halo otherwise.
//!   - Product on the LEFT, scene on the RIGHT (the spike's load-
//!     bearing instruction phrasing assumes that order).
//!
//! The helper is pure-local + free; the paid call is the Kontext step
//! that consumes its output.
//!
//! Height normalization matches the spike: both halves are resized
//! (preserving aspect) to the *smaller* source's height. Kontext clamps
//! to its supported sizes regardless, so working at the smaller height
//! keeps the input lean without changing the output.
//!
//! Mirrors the spike's Python `PIL.Image.new + paste` semantics — see
//! `docs/research/insert-anything-spike.md` §Setup.

use image::imageops::FilterType;
use image::{DynamicImage, GenericImage, GenericImageView, Rgb, RgbImage};
use std::path::{Path, PathBuf};

/// Errors raised while building the concat input.
#[derive(Debug, thiserror::Error)]
pub enum ConcatError {
    /// IO / decode failure on either source.
    #[error("read {path}: {source}")]
    Read {
        /// The path that failed.
        path: PathBuf,
        /// Underlying io / image-crate error.
        #[source]
        source: image::ImageError,
    },
    /// PNG encode failure on output.
    #[error("write {path}: {source}")]
    Write {
        /// The path that failed.
        path: PathBuf,
        /// Underlying image-crate error.
        #[source]
        source: image::ImageError,
    },
    /// Either input decoded to zero dimensions.
    #[error("image {0} has zero dimensions")]
    EmptyDimensions(PathBuf),
}

/// Concat result — dimensions of the saved PNG and the path it lives at.
#[derive(Debug, Clone)]
pub struct ConcatOutput {
    /// Final image width (left + right).
    pub width: u32,
    /// Final image height (both halves, normalized).
    pub height: u32,
    /// Width of the left (product) half after height-normalization.
    pub left_width: u32,
    /// Width of the right (scene) half after height-normalization.
    pub right_width: u32,
    /// Path the concatenated PNG was written to.
    pub path: PathBuf,
}

/// Decode both sources, normalize them to the taller image's height
/// (preserving each one's aspect), and write a side-by-side RGB PNG
/// with `left` on the left and `right` on the right.
///
/// Alpha-bearing inputs are composited onto opaque white before the
/// concat — Kontext interprets remaining black/zero alpha as a hole
/// otherwise, and the spike's product source had an RGBA halo.
pub fn concat_horizontal(
    left: &Path,
    right: &Path,
    out: &Path,
) -> Result<ConcatOutput, ConcatError> {
    let left_img = image::open(left).map_err(|source| ConcatError::Read {
        path: left.to_path_buf(),
        source,
    })?;
    let right_img = image::open(right).map_err(|source| ConcatError::Read {
        path: right.to_path_buf(),
        source,
    })?;

    let merged = concat_horizontal_in_memory(&left_img, &right_img, left, right)?;

    merged.image.save(out).map_err(|source| ConcatError::Write {
        path: out.to_path_buf(),
        source,
    })?;

    Ok(ConcatOutput {
        width: merged.image.width(),
        height: merged.image.height(),
        left_width: merged.left_width,
        right_width: merged.right_width,
        path: out.to_path_buf(),
    })
}

#[derive(Debug)]
struct MergedRgb {
    image: RgbImage,
    left_width: u32,
    right_width: u32,
}

fn concat_horizontal_in_memory(
    left: &DynamicImage,
    right: &DynamicImage,
    left_path: &Path,
    right_path: &Path,
) -> Result<MergedRgb, ConcatError> {
    let (lw, lh) = left.dimensions();
    let (rw, rh) = right.dimensions();
    if lw == 0 || lh == 0 {
        return Err(ConcatError::EmptyDimensions(left_path.to_path_buf()));
    }
    if rw == 0 || rh == 0 {
        return Err(ConcatError::EmptyDimensions(right_path.to_path_buf()));
    }

    let target_h = lh.min(rh);
    let left_scaled = scale_to_height(left, target_h, lw, lh);
    let right_scaled = scale_to_height(right, target_h, rw, rh);

    let left_rgb = flatten_to_rgb(&left_scaled);
    let right_rgb = flatten_to_rgb(&right_scaled);

    let left_width = left_rgb.width();
    let right_width = right_rgb.width();
    let mut canvas = RgbImage::new(left_width + right_width, target_h);
    canvas
        .copy_from(&left_rgb, 0, 0)
        .expect("left fits at origin by construction");
    canvas
        .copy_from(&right_rgb, left_width, 0)
        .expect("right fits at offset by construction");

    Ok(MergedRgb {
        image: canvas,
        left_width,
        right_width,
    })
}

fn scale_to_height(img: &DynamicImage, target_h: u32, w: u32, h: u32) -> DynamicImage {
    if h == target_h {
        return img.clone();
    }
    let new_w = ((w as f64) * (target_h as f64) / (h as f64)).round().max(1.0) as u32;
    img.resize_exact(new_w, target_h, FilterType::Lanczos3)
}

fn flatten_to_rgb(img: &DynamicImage) -> RgbImage {
    match img {
        DynamicImage::ImageRgb8(rgb) => rgb.clone(),
        _ => {
            let rgba = img.to_rgba8();
            let mut out = RgbImage::new(rgba.width(), rgba.height());
            for (x, y, px) in rgba.enumerate_pixels() {
                let a = px[0] as u16 * px[3] as u16 + 255 * (255 - px[3] as u16);
                let r_over = (a / 255) as u8;
                let g = (px[1] as u16 * px[3] as u16 + 255 * (255 - px[3] as u16)) / 255;
                let b = (px[2] as u16 * px[3] as u16 + 255 * (255 - px[3] as u16)) / 255;
                out.put_pixel(x, y, Rgb([r_over, g as u8, b as u8]));
            }
            out
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{Rgba, RgbaImage};

    fn solid_rgb(w: u32, h: u32, c: [u8; 3]) -> DynamicImage {
        let mut img = RgbImage::new(w, h);
        for px in img.pixels_mut() {
            *px = Rgb(c);
        }
        DynamicImage::ImageRgb8(img)
    }

    fn solid_rgba(w: u32, h: u32, c: [u8; 4]) -> DynamicImage {
        let mut img = RgbaImage::new(w, h);
        for px in img.pixels_mut() {
            *px = Rgba(c);
        }
        DynamicImage::ImageRgba8(img)
    }

    #[test]
    fn equal_height_concats_to_sum_of_widths() {
        let left = solid_rgb(100, 50, [255, 0, 0]);
        let right = solid_rgb(200, 50, [0, 255, 0]);
        let merged = concat_horizontal_in_memory(
            &left,
            &right,
            Path::new("l"),
            Path::new("r"),
        )
        .unwrap();
        assert_eq!(merged.image.width(), 300);
        assert_eq!(merged.image.height(), 50);
        assert_eq!(merged.left_width, 100);
        assert_eq!(merged.right_width, 200);
        assert_eq!(*merged.image.get_pixel(0, 0), Rgb([255, 0, 0]));
        assert_eq!(*merged.image.get_pixel(150, 0), Rgb([0, 255, 0]));
    }

    #[test]
    fn taller_right_scales_down_to_smaller_height() {
        let left = solid_rgb(100, 50, [255, 0, 0]);
        let right = solid_rgb(200, 200, [0, 255, 0]);
        let merged = concat_horizontal_in_memory(
            &left,
            &right,
            Path::new("l"),
            Path::new("r"),
        )
        .unwrap();
        assert_eq!(merged.image.height(), 50);
        assert_eq!(merged.left_width, 100);
        assert_eq!(merged.right_width, 50);
        assert_eq!(merged.image.width(), 150);
    }

    #[test]
    fn taller_left_scales_down_to_smaller_height() {
        let left = solid_rgb(200, 200, [255, 0, 0]);
        let right = solid_rgb(100, 50, [0, 255, 0]);
        let merged = concat_horizontal_in_memory(
            &left,
            &right,
            Path::new("l"),
            Path::new("r"),
        )
        .unwrap();
        assert_eq!(merged.image.height(), 50);
        assert_eq!(merged.left_width, 50);
        assert_eq!(merged.right_width, 100);
    }

    #[test]
    fn output_is_three_channel_rgb_even_with_rgba_input() {
        let left = solid_rgba(64, 64, [255, 0, 0, 0]);
        let right = solid_rgb(64, 64, [0, 255, 0]);
        let merged = concat_horizontal_in_memory(
            &left,
            &right,
            Path::new("l"),
            Path::new("r"),
        )
        .unwrap();
        let pixel = *merged.image.get_pixel(0, 0);
        assert_eq!(pixel, Rgb([255, 255, 255]), "transparent → white, not black");
    }

    #[test]
    fn opaque_rgba_pixels_keep_their_color() {
        let left = solid_rgba(32, 32, [200, 100, 50, 255]);
        let right = solid_rgb(32, 32, [0, 0, 0]);
        let merged = concat_horizontal_in_memory(
            &left,
            &right,
            Path::new("l"),
            Path::new("r"),
        )
        .unwrap();
        assert_eq!(*merged.image.get_pixel(0, 0), Rgb([200, 100, 50]));
    }

    #[test]
    fn zero_width_left_is_rejected() {
        let left = DynamicImage::ImageRgb8(RgbImage::new(0, 64));
        let right = solid_rgb(64, 64, [0, 0, 0]);
        let err = concat_horizontal_in_memory(&left, &right, Path::new("L"), Path::new("R"))
            .unwrap_err();
        match err {
            ConcatError::EmptyDimensions(p) => assert_eq!(p, PathBuf::from("L")),
            _ => panic!("expected EmptyDimensions, got {err:?}"),
        }
    }

    #[test]
    fn round_trip_to_disk() {
        let dir = std::env::temp_dir().join("wavelet-concat-roundtrip");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let lp = dir.join("l.png");
        let rp = dir.join("r.png");
        let op = dir.join("out.png");
        solid_rgb(80, 60, [255, 0, 0]).save(&lp).unwrap();
        solid_rgb(120, 60, [0, 0, 255]).save(&rp).unwrap();
        let out = concat_horizontal(&lp, &rp, &op).unwrap();
        assert_eq!(out.width, 200);
        assert_eq!(out.height, 60);
        assert_eq!(out.left_width, 80);
        let reread = image::open(&op).unwrap();
        assert_eq!(reread.width(), 200);
    }
}
