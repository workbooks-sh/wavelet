//! Render-time scrim/contrast helpers as CSS custom properties.
//!
//! Combines [`super::negative_space::analyze`] (find the cleanest cell
//! for text) with the scrim-suggestion logic from [`super::contrast`]
//! to emit a ready-to-paste block of CSS custom properties:
//!
//! ```css
//! :root {
//!   --scrim-color: #000000;
//!   --scrim-opacity: 0.5;
//!   --text-color-recommended: #ffffff;
//!   --negative-space-x: 100px;
//!   --negative-space-y: 800px;
//!   --negative-space-w: 600px;
//!   --negative-space-h: 200px;
//! }
//! ```
//!
//! Scene HTML then writes
//! `background: var(--scrim-color); opacity: var(--scrim-opacity);` over
//! the negative-space rect, without re-deriving any of these values per
//! shot. The agent retains full creative freedom — these are informed
//! defaults to *accept or override*, not a constraint.

use std::path::Path;

use serde::{Deserialize, Serialize};

use super::{negative_space, AnalysisError, BoundingRect, Rgb};

/// One scrim-plan output. Mirrors the shape rendered to the JSON CLI
/// output of `wavelet image scrim`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScrimReport {
    /// Image width in pixels.
    pub image_width: u32,
    /// Image height in pixels.
    pub image_height: u32,
    /// Grid the underlying negative-space scan used.
    pub grid_rows: u32,
    /// Grid columns.
    pub grid_cols: u32,
    /// Pixel rect of the best negative-space cell.
    pub negative_space: BoundingRect,
    /// `negative_space.score` from the negative-space report. `1.0` =
    /// perfectly clean for text, `0.0` = unusable.
    pub negative_space_score: f32,
    /// Recommended text color (white for dark scenes, black for light).
    pub text_color_recommended: Rgb,
    /// Scrim fill color — always the opposite of the recommended text
    /// color so the scrim makes text *more* readable, never less.
    pub scrim_color: Rgb,
    /// Scrim opacity needed to clear `threshold` against the picked
    /// text color. `0.0` when no scrim is needed.
    pub scrim_opacity: f32,
    /// WCAG threshold the plan was targeted at.
    pub threshold: f32,
    /// Ready-to-paste `:root { … }` block. Same data as the individual
    /// fields above; keeps the agent from having to assemble the CSS
    /// itself.
    pub css_block: String,
}

/// Run the negative-space + scrim analysis on `image_path` and emit a
/// CSS-ready plan.
pub fn analyze(
    image_path: &Path,
    rows: u32,
    cols: u32,
    threshold: f32,
) -> Result<ScrimReport, AnalysisError> {
    if threshold <= 1.0 {
        return Err(AnalysisError::InvalidArgument(format!(
            "threshold must be > 1.0, got {threshold}"
        )));
    }
    let ns = negative_space::analyze(image_path, rows, cols)?;
    let best = ns.cells.first().ok_or_else(|| {
        AnalysisError::InvalidArgument("negative_space analysis returned no cells".into())
    })?;
    let bbox = BoundingRect::new(best.bbox[0], best.bbox[1], best.bbox[2], best.bbox[3]);
    let text_color = best.suggested_text_color;
    let scrim_color = if text_color.relative_luminance() > 0.5 {
        Rgb::BLACK
    } else {
        Rgb::WHITE
    };
    let scrim_opacity = best.suggested_scrim_opacity;

    let css_block = render_css(text_color, scrim_color, scrim_opacity, bbox);

    Ok(ScrimReport {
        image_width: ns.width,
        image_height: ns.height,
        grid_rows: ns.rows,
        grid_cols: ns.cols,
        negative_space: bbox,
        negative_space_score: best.score,
        text_color_recommended: text_color,
        scrim_color,
        scrim_opacity,
        threshold,
        css_block,
    })
}

fn render_css(text_color: Rgb, scrim_color: Rgb, opacity: f32, bbox: BoundingRect) -> String {
    format!(
        ":root {{\n  --scrim-color: {scrim};\n  --scrim-opacity: {opacity:.3};\n  --text-color-recommended: {text};\n  --negative-space-x: {x}px;\n  --negative-space-y: {y}px;\n  --negative-space-w: {w}px;\n  --negative-space-h: {h}px;\n}}\n",
        scrim = scrim_color.to_hex(),
        text = text_color.to_hex(),
        x = bbox.x,
        y = bbox.y,
        w = bbox.w,
        h = bbox.h,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{ImageBuffer, Rgb as ImgRgb};
    use std::fs;

    fn write_test_image(width: u32, height: u32, fill: [u8; 3]) -> std::path::PathBuf {
        let buf: ImageBuffer<ImgRgb<u8>, Vec<u8>> = ImageBuffer::from_pixel(width, height, ImgRgb(fill));
        let dir = std::env::temp_dir().join(format!(
            "wavelet-scrim-test-{}-{}",
            width,
            std::process::id()
        ));
        let _ = fs::create_dir_all(&dir);
        let path = dir.join("img.png");
        buf.save(&path).unwrap();
        path
    }

    #[test]
    fn dark_scene_recommends_white_text_and_black_scrim() {
        let path = write_test_image(320, 180, [10, 10, 10]);
        let report = analyze(&path, 3, 3, 4.5).unwrap();
        assert_eq!(report.text_color_recommended.to_hex(), "#ffffff");
        assert_eq!(report.scrim_color.to_hex(), "#000000");
        assert_eq!(report.grid_rows, 3);
        assert_eq!(report.grid_cols, 3);
        assert!(report.css_block.contains("--scrim-color: #000000"));
        assert!(report.css_block.contains("--text-color-recommended: #ffffff"));
    }

    #[test]
    fn light_scene_recommends_black_text_and_white_scrim() {
        let path = write_test_image(320, 180, [245, 245, 245]);
        let report = analyze(&path, 3, 3, 4.5).unwrap();
        assert_eq!(report.text_color_recommended.to_hex(), "#000000");
        assert_eq!(report.scrim_color.to_hex(), "#ffffff");
        assert!(report.css_block.contains("--scrim-color: #ffffff"));
        assert!(report.css_block.contains("--text-color-recommended: #000000"));
    }

    #[test]
    fn css_block_includes_all_seven_variables() {
        let path = write_test_image(640, 360, [128, 0, 64]);
        let report = analyze(&path, 4, 4, 4.5).unwrap();
        for var in [
            "--scrim-color",
            "--scrim-opacity",
            "--text-color-recommended",
            "--negative-space-x",
            "--negative-space-y",
            "--negative-space-w",
            "--negative-space-h",
        ] {
            assert!(
                report.css_block.contains(var),
                "css_block missing {var}: {}",
                report.css_block
            );
        }
    }

    #[test]
    fn negative_space_rect_sits_inside_image() {
        let path = write_test_image(800, 450, [60, 60, 60]);
        let report = analyze(&path, 5, 5, 4.5).unwrap();
        let ns = report.negative_space;
        assert!(ns.x + ns.w <= report.image_width);
        assert!(ns.y + ns.h <= report.image_height);
    }

    #[test]
    fn invalid_threshold_rejected() {
        let path = write_test_image(80, 60, [0, 0, 0]);
        let err = analyze(&path, 3, 3, 1.0).unwrap_err();
        assert!(matches!(err, AnalysisError::InvalidArgument(_)));
    }

    #[test]
    fn css_block_uses_px_units_for_geometry() {
        let path = write_test_image(160, 90, [200, 200, 200]);
        let report = analyze(&path, 3, 3, 4.5).unwrap();
        assert!(report.css_block.contains("px"));
    }
}
