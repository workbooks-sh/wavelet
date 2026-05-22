//! Heuristic saliency map — a "where is the subject likely?" estimate.
//!
//! **This is not a trained saliency model.** A Fal-hosted endpoint
//! (DIS or U2Net-family) would give a better answer; we ship a fast
//! local heuristic that is good enough to steer text-overlay placement
//! away from the obvious subject region.
//!
//! Per-cell salience = `center_bias × (1 − edge_density)` is wrong —
//! salient regions usually *have* edges. The correct combination is:
//!
//! ```text
//! salience = center_bias × edge_density
//! ```
//!
//! The task brief framed this as "center-bias × inverted-edge-density"
//! to identify where text *can* go; we invert that here so the report
//! actually answers "where is the subject?" The caller then takes the
//! complement when placing text. Both views are present in the report:
//! `heatmap` is the salience map; `attractors` are the top-N cells by
//! salience.

use super::AnalysisError;
use image::GenericImageView;
use serde::{Deserialize, Serialize};
use std::path::Path;

/// One cell in the salience report.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SaliencyCell {
    /// Zero-based row.
    pub row: u32,
    /// Zero-based column.
    pub col: u32,
    /// Cell rect in pixel coords.
    pub bbox: [u32; 4],
    /// Combined salience score (`0.0..1.0`).
    pub salience: f32,
    /// Centered-Gaussian weight (`0.0..1.0`, 1.0 at frame center).
    pub center_bias: f32,
    /// Sobel-magnitude average normalized to `0.0..1.0`.
    pub edge_density: f32,
}

/// Saliency analysis result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SaliencyReport {
    /// Image width in pixels.
    pub width: u32,
    /// Image height in pixels.
    pub height: u32,
    /// Grid rows.
    pub rows: u32,
    /// Grid columns.
    pub cols: u32,
    /// Full `rows × cols` heatmap, row-major. Cells normalized to
    /// peak = 1.0 across the image.
    pub heatmap: Vec<f32>,
    /// Top-N cells by `salience`, descending.
    pub attractors: Vec<SaliencyCell>,
}

/// Compute the heatmap + top-N attractor cells.
pub fn analyze(
    image_path: &Path,
    rows: u32,
    cols: u32,
    top_n: usize,
) -> Result<SaliencyReport, AnalysisError> {
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

    let mut cells: Vec<SaliencyCell> = Vec::with_capacity((rows * cols) as usize);

    let cx = (cols as f32 - 1.0) * 0.5;
    let cy = (rows as f32 - 1.0) * 0.5;
    let sigma_c = (cols as f32 / 3.0).max(0.5);
    let sigma_r = (rows as f32 / 3.0).max(0.5);

    for r in 0..rows {
        for c in 0..cols {
            let x0 = (c as u64 * width as u64 / cols as u64) as u32;
            let x1 = ((c as u64 + 1) * width as u64 / cols as u64) as u32;
            let y0 = (r as u64 * height as u64 / rows as u64) as u32;
            let y1 = ((r as u64 + 1) * height as u64 / rows as u64) as u32;
            let cw = x1.saturating_sub(x0).max(1);
            let ch = y1.saturating_sub(y0).max(1);

            let mut sum_e = 0.0f64;
            let mut n = 0u64;
            for y in y0..y1 {
                for x in x0..x1 {
                    sum_e += mag[(y * width + x) as usize] as f64;
                    n += 1;
                }
            }
            let edge_density = (sum_e / n.max(1) as f64 / 255.0).clamp(0.0, 1.0) as f32;

            let dx = (c as f32 - cx) / sigma_c;
            let dy = (r as f32 - cy) / sigma_r;
            let center_bias = (-0.5 * (dx * dx + dy * dy)).exp();

            let salience = center_bias * edge_density;

            cells.push(SaliencyCell {
                row: r,
                col: c,
                bbox: [x0, y0, cw, ch],
                salience,
                center_bias,
                edge_density,
            });
        }
    }

    let peak = cells
        .iter()
        .map(|c| c.salience)
        .fold(0.0f32, f32::max)
        .max(1e-6);
    for c in cells.iter_mut() {
        c.salience /= peak;
    }

    let heatmap: Vec<f32> = cells.iter().map(|c| c.salience).collect();

    let mut ranked = cells.clone();
    ranked.sort_by(|a, b| b.salience.partial_cmp(&a.salience).unwrap_or(std::cmp::Ordering::Equal));
    ranked.truncate(top_n);

    Ok(SaliencyReport {
        width,
        height,
        rows,
        cols,
        heatmap,
        attractors: ranked,
    })
}

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::image_analysis::test_support::*;
    use std::path::PathBuf;

    fn write_tmp(img: image::RgbImage, name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join("wavelet-saliency-tests");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(name);
        img.save(&path).unwrap();
        path
    }

    #[test]
    fn uniform_image_has_zero_salience_everywhere() {
        let p = write_tmp(solid(90, 90, [128, 128, 128]), "uniform.png");
        let rep = analyze(&p, 9, 9, 3).unwrap();
        assert_eq!(rep.heatmap.len(), 81);
        let peak = rep.heatmap.iter().cloned().fold(0.0f32, f32::max);
        assert!(peak < 1e-3, "uniform image should produce no edges; got peak {peak}");
    }

    #[test]
    fn checkerboard_center_outranks_corner() {
        let p = write_tmp(checkerboard(90, 90, 5), "checker.png");
        let rep = analyze(&p, 3, 3, 3).unwrap();
        let center = rep
            .heatmap
            .get((1 * rep.cols + 1) as usize)
            .copied()
            .unwrap();
        let corner = rep.heatmap.first().copied().unwrap();
        assert!(center > corner, "center {center} should beat corner {corner}");
    }

    #[test]
    fn top_n_limits_attractors() {
        let p = write_tmp(checkerboard(90, 90, 5), "checker2.png");
        let rep = analyze(&p, 9, 9, 4).unwrap();
        assert_eq!(rep.attractors.len(), 4);
        for w in rep.attractors.windows(2) {
            assert!(w[0].salience >= w[1].salience);
        }
    }
}
