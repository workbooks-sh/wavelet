//! WCAG contrast check with auto-scrim suggestion.
//!
//! Given an image, a candidate text region, and a text color, compute
//! the mean luminance under the region and the WCAG 2.x contrast
//! ratio. When the ratio is below `threshold` (default 4.5 — AA for
//! normal text), suggest a scrim color + opacity that would lift it
//! above the threshold. The scrim is composited via straight alpha
//! and is forced to the opposite of the text color (black scrim under
//! white text, white scrim under black text).

use super::{wcag_contrast_ratio, AnalysisError, BoundingRect, Rgb};
use image::GenericImageView;
use serde::{Deserialize, Serialize};
use std::path::Path;

/// One contrast-analysis result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContrastReport {
    /// Image width in pixels.
    pub image_width: u32,
    /// Image height in pixels.
    pub image_height: u32,
    /// Region actually analyzed (clipped to image bounds).
    pub region: BoundingRect,
    /// Text color used for the contrast computation.
    pub text_color: Rgb,
    /// Mean luminance in the region (`0.0..1.0`).
    pub mean_luminance: f32,
    /// Bare contrast ratio between text and mean luminance.
    pub contrast_ratio: f32,
    /// Threshold the caller asked us to clear.
    pub threshold: f32,
    /// `true` when `contrast_ratio >= threshold`.
    pub passes: bool,
    /// Recommended scrim when `passes` is false. `None` when the bare
    /// contrast already passes.
    pub suggested_scrim: Option<ScrimSuggestion>,
}

/// Recommended scrim overlay to push contrast above threshold.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScrimSuggestion {
    /// Scrim color (black under white text, white under black/dark text).
    pub color: Rgb,
    /// Required opacity `0.0..=1.0`.
    pub opacity: f32,
    /// Predicted contrast ratio after applying this scrim.
    pub predicted_ratio: f32,
}

/// Analyze the contrast between `text_color` and the mean luminance of
/// `region` in `image_path`. Suggests a scrim if needed.
pub fn analyze(
    image_path: &Path,
    region: BoundingRect,
    text_color: Rgb,
    threshold: f32,
) -> Result<ContrastReport, AnalysisError> {
    if threshold <= 1.0 {
        return Err(AnalysisError::InvalidArgument(format!(
            "threshold must be > 1.0, got {threshold}"
        )));
    }
    let img = image::open(image_path).map_err(|e| AnalysisError::Decode(e.to_string()))?;
    let (iw, ih) = img.dimensions();
    if iw == 0 || ih == 0 {
        return Err(AnalysisError::InvalidArgument(format!(
            "image has zero dimension ({iw}×{ih})"
        )));
    }
    if region.x >= iw || region.y >= ih {
        return Err(AnalysisError::InvalidArgument(format!(
            "region origin ({},{}) outside image ({iw}×{ih})",
            region.x, region.y
        )));
    }
    let clipped = region.clipped(iw, ih);
    if clipped.w == 0 || clipped.h == 0 {
        return Err(AnalysisError::InvalidArgument(
            "clipped region has zero area".into(),
        ));
    }
    let rgb = img.to_rgb8();

    let mut sum_l = 0.0f64;
    let mut n = 0u64;
    for y in clipped.y..clipped.y + clipped.h {
        for x in clipped.x..clipped.x + clipped.w {
            let p = rgb.get_pixel(x, y).0;
            let px = Rgb::new(p[0], p[1], p[2]);
            sum_l += px.relative_luminance() as f64;
            n += 1;
        }
    }
    let mean_l = (sum_l / n.max(1) as f64) as f32;
    let text_l = text_color.relative_luminance();
    let ratio = wcag_contrast_ratio(text_l, mean_l);

    let passes = ratio >= threshold;
    let suggested_scrim = if passes {
        None
    } else {
        Some(suggest_scrim(mean_l, text_color, threshold))
    };

    Ok(ContrastReport {
        image_width: iw,
        image_height: ih,
        region: clipped,
        text_color,
        mean_luminance: mean_l,
        contrast_ratio: ratio,
        threshold,
        passes,
        suggested_scrim,
    })
}

/// Binary-search the scrim opacity needed to clear `target`.
fn suggest_scrim(mean_l: f32, text_color: Rgb, target: f32) -> ScrimSuggestion {
    let scrim_color = if text_color.relative_luminance() > 0.5 {
        Rgb::BLACK
    } else {
        Rgb::WHITE
    };
    let scrim_l = scrim_color.relative_luminance();
    let text_l = text_color.relative_luminance();

    let mut lo = 0.0f32;
    let mut hi = 1.0f32;
    for _ in 0..40 {
        let mid = 0.5 * (lo + hi);
        let blended = mid * scrim_l + (1.0 - mid) * mean_l;
        let r = wcag_contrast_ratio(text_l, blended);
        if r >= target {
            hi = mid;
        } else {
            lo = mid;
        }
    }
    let opacity = hi;
    let blended = opacity * scrim_l + (1.0 - opacity) * mean_l;
    let predicted_ratio = wcag_contrast_ratio(text_l, blended);
    ScrimSuggestion {
        color: scrim_color,
        opacity,
        predicted_ratio,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::image_analysis::test_support::*;
    use std::path::PathBuf;

    fn write_tmp(img: image::RgbImage, name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join("wavelet-contrast-tests");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(name);
        img.save(&path).unwrap();
        path
    }

    #[test]
    fn white_text_on_black_image_passes() {
        let p = write_tmp(solid(40, 40, [0, 0, 0]), "black.png");
        let rep = analyze(&p, BoundingRect::new(0, 0, 40, 40), Rgb::WHITE, 4.5).unwrap();
        assert!(rep.passes);
        assert!(rep.contrast_ratio > 20.0);
        assert!(rep.suggested_scrim.is_none());
    }

    #[test]
    fn white_text_on_white_image_needs_full_scrim() {
        let p = write_tmp(solid(40, 40, [255, 255, 255]), "white.png");
        let rep = analyze(&p, BoundingRect::new(0, 0, 40, 40), Rgb::WHITE, 4.5).unwrap();
        assert!(!rep.passes);
        let scrim = rep.suggested_scrim.unwrap();
        assert_eq!(scrim.color.r, 0);
        assert!(scrim.opacity > 0.5, "needs substantial black scrim");
        assert!(scrim.predicted_ratio >= 4.5);
    }

    #[test]
    fn region_outside_image_errors() {
        let p = write_tmp(solid(40, 40, [0, 0, 0]), "tiny.png");
        let err = analyze(&p, BoundingRect::new(100, 100, 5, 5), Rgb::WHITE, 4.5).unwrap_err();
        assert!(matches!(err, AnalysisError::InvalidArgument(_)));
    }

    #[test]
    fn invalid_threshold_rejected() {
        let p = write_tmp(solid(10, 10, [0, 0, 0]), "tiny2.png");
        let err = analyze(&p, BoundingRect::new(0, 0, 10, 10), Rgb::WHITE, 0.5).unwrap_err();
        assert!(matches!(err, AnalysisError::InvalidArgument(_)));
    }

    #[test]
    fn mid_gray_with_white_text_needs_partial_scrim() {
        let p = write_tmp(solid(40, 40, [128, 128, 128]), "mid.png");
        let rep = analyze(&p, BoundingRect::new(0, 0, 40, 40), Rgb::WHITE, 4.5).unwrap();
        assert!(!rep.passes, "mid-gray + white text is ~3.95:1");
        let scrim = rep.suggested_scrim.unwrap();
        assert!(scrim.opacity > 0.0 && scrim.opacity < 1.0);
        assert!(scrim.predicted_ratio >= 4.5);
    }
}
