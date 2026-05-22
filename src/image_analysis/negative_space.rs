//! Negative-space scorer.
//!
//! Partitions the image into a `rows × cols` grid and ranks each cell
//! by how clean it is for a text overlay. "Clean" means low edge
//! density and low brightness variance — flat areas with no
//! competing detail.
//!
//! Score model:
//!
//! ```text
//! score = 1.0 − edge_density − 0.3 × (variance / max_variance)
//! ```
//!
//! Edge density uses a hand-rolled Sobel magnitude (no `imageproc`
//! dependency — the wavelet tree pins `image = =0.25.6` and the newer
//! `imageproc` versions don't resolve under that pin; see Cargo.toml
//! notes on `blur`). For each cell we also report the mean luminance,
//! the suggested text color (white-on-dark or black-on-light), and a
//! recommended scrim opacity that would lift the mean-to-text contrast
//! to WCAG AA (4.5:1).

use super::{wcag_contrast_ratio, AnalysisError, Rgb};
use image::GenericImageView;
use serde::{Deserialize, Serialize};
use std::path::Path;

/// Single grid-cell score.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CellScore {
    /// Zero-based row.
    pub row: u32,
    /// Zero-based column.
    pub col: u32,
    /// Cell rect in pixel coords (clipped to image bounds).
    pub bbox: [u32; 4],
    /// Final ranking score (higher = better for text). Normalized so
    /// `1.0` is "perfectly clean" and `0.0` is "too busy."
    pub score: f32,
    /// Sobel-magnitude average, normalized to `0.0..1.0`.
    pub edge_density: f32,
    /// Mean luminance under the cell (`0.0..1.0`, sRGB-gamma-decoded).
    pub mean_luminance: f32,
    /// Variance of luminance under the cell (`0.0..1.0`, raw — not
    /// normalized by `max_variance`).
    pub variance: f32,
    /// Suggested text color: white when the cell is darker than mid,
    /// black otherwise.
    pub suggested_text_color: Rgb,
    /// Scrim opacity (`0.0..1.0`) needed to reach WCAG AA (4.5:1)
    /// against `suggested_text_color`. `0.0` means no scrim needed.
    pub suggested_scrim_opacity: f32,
}

/// Negative-space analysis result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NegativeSpaceReport {
    /// Image width in pixels.
    pub width: u32,
    /// Image height in pixels.
    pub height: u32,
    /// Grid rows.
    pub rows: u32,
    /// Grid columns.
    pub cols: u32,
    /// Cells, ranked best-to-worst.
    pub cells: Vec<CellScore>,
}

/// Score every cell and return them ranked by descending `score`.
pub fn analyze(
    image_path: &Path,
    rows: u32,
    cols: u32,
) -> Result<NegativeSpaceReport, AnalysisError> {
    if rows == 0 || cols == 0 {
        return Err(AnalysisError::InvalidArgument(
            "rows and cols must be > 0".into(),
        ));
    }
    let img = image::open(image_path).map_err(|e| AnalysisError::Decode(e.to_string()))?;
    let (width, height) = img.dimensions();
    if width == 0 || height == 0 {
        return Err(AnalysisError::InvalidArgument(format!(
            "image has zero dimension ({width}×{height})"
        )));
    }
    let luma = img.to_luma8();
    let mag = sobel_magnitude(&luma);

    let mut cells: Vec<CellScore> = Vec::with_capacity((rows * cols) as usize);
    let mut max_variance = 0.0f32;

    let mut raw: Vec<(u32, u32, [u32; 4], f32, f32, f32)> =
        Vec::with_capacity((rows * cols) as usize);

    for r in 0..rows {
        for c in 0..cols {
            let x0 = (c as u64 * width as u64 / cols as u64) as u32;
            let x1 = ((c as u64 + 1) * width as u64 / cols as u64) as u32;
            let y0 = (r as u64 * height as u64 / rows as u64) as u32;
            let y1 = ((r as u64 + 1) * height as u64 / rows as u64) as u32;
            let cw = x1.saturating_sub(x0).max(1);
            let ch = y1.saturating_sub(y0).max(1);

            let mut sum_e = 0.0f64;
            let mut sum_l = 0.0f64;
            let mut sum_l2 = 0.0f64;
            let mut n = 0u64;
            for y in y0..y1 {
                for x in x0..x1 {
                    let m = mag[(y * width + x) as usize] as f64;
                    sum_e += m;
                    let pix = luma.get_pixel(x, y).0[0] as f64 / 255.0;
                    sum_l += pix;
                    sum_l2 += pix * pix;
                    n += 1;
                }
            }
            let n_f = n.max(1) as f64;
            let edge_density = (sum_e / n_f / 255.0).clamp(0.0, 1.0) as f32;
            let mean_l = (sum_l / n_f) as f32;
            let var_l = ((sum_l2 / n_f) - (mean_l as f64).powi(2)).max(0.0) as f32;
            if var_l > max_variance {
                max_variance = var_l;
            }
            raw.push((r, c, [x0, y0, cw, ch], edge_density, mean_l, var_l));
        }
    }

    let denom = if max_variance < 1e-6 { 1.0 } else { max_variance };

    for (row, col, bbox, edge_density, mean_l, variance) in raw {
        let score = (1.0 - edge_density - 0.3 * (variance / denom)).clamp(0.0, 1.0);
        let text = if mean_l < 0.5 { Rgb::WHITE } else { Rgb::BLACK };
        let scrim = scrim_opacity_for_target_ratio(mean_l, text, 4.5);
        cells.push(CellScore {
            row,
            col,
            bbox,
            score,
            edge_density,
            mean_luminance: mean_l,
            variance,
            suggested_text_color: text,
            suggested_scrim_opacity: scrim,
        });
    }

    cells.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));

    Ok(NegativeSpaceReport {
        width,
        height,
        rows,
        cols,
        cells,
    })
}

/// Hand-rolled Sobel on an 8-bit luma buffer. Returns a flat
/// `width × height` byte vector of magnitudes clamped to `0..=255`.
/// Border pixels get magnitude zero (one-pixel inset).
fn sobel_magnitude(luma: &image::GrayImage) -> Vec<u8> {
    let (w, h) = luma.dimensions();
    let mut out = vec![0u8; (w * h) as usize];
    if w < 3 || h < 3 {
        return out;
    }
    let at = |x: u32, y: u32| -> i32 { luma.get_pixel(x, y).0[0] as i32 };
    for y in 1..h - 1 {
        for x in 1..w - 1 {
            let gx = -at(x - 1, y - 1) + at(x + 1, y - 1) - 2 * at(x - 1, y) + 2 * at(x + 1, y)
                - at(x - 1, y + 1)
                + at(x + 1, y + 1);
            let gy = -at(x - 1, y - 1) - 2 * at(x, y - 1) - at(x + 1, y - 1)
                + at(x - 1, y + 1)
                + 2 * at(x, y + 1)
                + at(x + 1, y + 1);
            let m = (((gx * gx + gy * gy) as f32).sqrt()).clamp(0.0, 255.0) as u8;
            out[(y * w + x) as usize] = m;
        }
    }
    out
}

/// Compute the scrim opacity needed for `text_color` to clear
/// `target_ratio` against a background of given mean luminance. The
/// scrim is assumed to be the same hue as `text_color`'s opposite
/// (i.e. black scrim under white text; white scrim under black text)
/// composited via straight alpha. Returns `0.0` when the bare
/// background already clears the target, `1.0` when no opacity gets
/// it there.
fn scrim_opacity_for_target_ratio(mean_l: f32, text_color: Rgb, target_ratio: f32) -> f32 {
    let scrim = if text_color.relative_luminance() > 0.5 {
        Rgb::BLACK
    } else {
        Rgb::WHITE
    };
    let scrim_l = scrim.relative_luminance();
    let text_l = text_color.relative_luminance();

    let bare = wcag_contrast_ratio(text_l, mean_l);
    if bare >= target_ratio {
        return 0.0;
    }

    let mut lo = 0.0f32;
    let mut hi = 1.0f32;
    for _ in 0..30 {
        let mid = 0.5 * (lo + hi);
        let blended = mid * scrim_l + (1.0 - mid) * mean_l;
        let r = wcag_contrast_ratio(text_l, blended);
        if r >= target_ratio {
            hi = mid;
        } else {
            lo = mid;
        }
    }
    let blended = hi * scrim_l + (1.0 - hi) * mean_l;
    if wcag_contrast_ratio(text_l, blended) >= target_ratio {
        hi
    } else {
        1.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::image_analysis::test_support::*;
    use std::path::PathBuf;

    fn write_tmp(img: image::RgbImage, name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join("wavelet-negspace-tests");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(name);
        img.save(&path).unwrap();
        path
    }

    #[test]
    fn uniform_image_top_cell_is_max_score() {
        let p = write_tmp(solid(60, 60, [128, 128, 128]), "uniform.png");
        let rep = analyze(&p, 3, 3).unwrap();
        assert_eq!(rep.cells.len(), 9);
        assert!(rep.cells[0].score > 0.99);
        assert!(rep.cells[0].edge_density < 0.01);
    }

    #[test]
    fn vertical_bar_left_side_outranks_right() {
        let p = write_tmp(right_half_bar(60, 60), "rightbar.png");
        let rep = analyze(&p, 1, 3).unwrap();
        let left = rep.cells.iter().find(|c| c.col == 0).unwrap();
        let right = rep.cells.iter().find(|c| c.col == 2).unwrap();
        assert!(left.score > right.score - 0.01, "{} vs {}", left.score, right.score);
        assert!(left.mean_luminance < right.mean_luminance);
    }

    #[test]
    fn zero_rows_rejected() {
        let p = write_tmp(solid(10, 10, [0, 0, 0]), "zero.png");
        let err = analyze(&p, 0, 3).unwrap_err();
        assert!(matches!(err, AnalysisError::InvalidArgument(_)));
    }

    #[test]
    fn dark_cells_suggest_white_text() {
        let p = write_tmp(solid(30, 30, [5, 5, 5]), "dark.png");
        let rep = analyze(&p, 1, 1).unwrap();
        let c = &rep.cells[0];
        assert_eq!(c.suggested_text_color.r, 255);
        assert!(c.suggested_scrim_opacity < 0.01, "dark bg + white text needs no scrim");
    }
}
