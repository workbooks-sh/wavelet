//! Pure-local image analyses an agent runs after generating a still
//! to decide where to place text overlays. None of these modules call
//! external services (the OCR module is the lone exception and is
//! gated behind a feature probe; see `ocr`).
//!
//! Four entry points, all returning JSON-friendly reports:
//!
//! - [`negative_space::analyze`] — rank grid cells by how "clean" they
//!   are for text overlay (low edge density + low brightness variance).
//! - [`saliency::analyze`] — heuristic eye-attractor heatmap (center
//!   bias × inverted edge density). Subject likely lives in the top
//!   cells. Complements `negative_space`.
//! - [`ocr::analyze`] — detect baked-in text (signage, watermarks).
//!   Currently a stub; see module docs.
//! - [`contrast::analyze`] — WCAG contrast check for a candidate text
//!   region + color; suggests a scrim if below threshold.

pub mod concat;
pub mod contrast;
pub mod face_refine;
pub mod negative_space;
pub mod ocr;
pub mod saliency;
pub mod scrim;

use serde::{Deserialize, Serialize};

/// Axis-aligned rectangle in pixel coordinates. Used by `contrast` and
/// the OCR report.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct BoundingRect {
    /// Left edge in pixels.
    pub x: u32,
    /// Top edge in pixels.
    pub y: u32,
    /// Width in pixels.
    pub w: u32,
    /// Height in pixels.
    pub h: u32,
}

impl BoundingRect {
    /// Build a rect from its components.
    pub fn new(x: u32, y: u32, w: u32, h: u32) -> Self {
        Self { x, y, w, h }
    }

    /// Clip to image bounds.
    pub(crate) fn clipped(&self, img_w: u32, img_h: u32) -> Self {
        let x = self.x.min(img_w.saturating_sub(1));
        let y = self.y.min(img_h.saturating_sub(1));
        let w = self.w.min(img_w.saturating_sub(x));
        let h = self.h.min(img_h.saturating_sub(y));
        Self { x, y, w, h }
    }
}

/// 24-bit RGB color.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Rgb {
    /// Red channel (0–255).
    pub r: u8,
    /// Green channel (0–255).
    pub g: u8,
    /// Blue channel (0–255).
    pub b: u8,
}

impl Rgb {
    /// New from components.
    pub fn new(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b }
    }

    /// Pure white.
    pub const WHITE: Rgb = Rgb { r: 255, g: 255, b: 255 };
    /// Pure black.
    pub const BLACK: Rgb = Rgb { r: 0, g: 0, b: 0 };

    /// Parse `#RGB`, `#RRGGBB`, or `RRGGBB`.
    pub fn parse_hex(s: &str) -> Result<Rgb, String> {
        let h = s.trim_start_matches('#');
        match h.len() {
            3 => {
                let r = u8::from_str_radix(&h[0..1], 16).map_err(|e| e.to_string())?;
                let g = u8::from_str_radix(&h[1..2], 16).map_err(|e| e.to_string())?;
                let b = u8::from_str_radix(&h[2..3], 16).map_err(|e| e.to_string())?;
                Ok(Rgb::new(r * 17, g * 17, b * 17))
            }
            6 => {
                let r = u8::from_str_radix(&h[0..2], 16).map_err(|e| e.to_string())?;
                let g = u8::from_str_radix(&h[2..4], 16).map_err(|e| e.to_string())?;
                let b = u8::from_str_radix(&h[4..6], 16).map_err(|e| e.to_string())?;
                Ok(Rgb::new(r, g, b))
            }
            _ => Err(format!("invalid hex color '{s}', expected #RGB or #RRGGBB")),
        }
    }

    /// CSS `#rrggbb` rendering.
    pub fn to_hex(self) -> String {
        format!("#{:02x}{:02x}{:02x}", self.r, self.g, self.b)
    }

    /// sRGB-gamma-decoded relative luminance per WCAG 2.x.
    pub fn relative_luminance(self) -> f32 {
        let chan = |c: u8| -> f32 {
            let v = c as f32 / 255.0;
            if v <= 0.03928 {
                v / 12.92
            } else {
                ((v + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * chan(self.r) + 0.7152 * chan(self.g) + 0.0722 * chan(self.b)
    }
}

/// Failure modes shared across the four analyses.
#[derive(Debug, thiserror::Error)]
pub enum AnalysisError {
    /// IO / decode failure on the source image.
    #[error("image decode: {0}")]
    Decode(String),

    /// Invalid argument (zero grid dims, region outside image, etc.).
    #[error("invalid argument: {0}")]
    InvalidArgument(String),

    /// External backend probe failed or no provider available.
    #[error("backend unimplemented: {0}")]
    Unimplemented(&'static str),
}

/// WCAG 2.x contrast ratio between two luminances. `L1` is the lighter,
/// `L2` the darker — the order is normalized internally.
pub fn wcag_contrast_ratio(l1: f32, l2: f32) -> f32 {
    let (a, b) = if l1 >= l2 { (l1, l2) } else { (l2, l1) };
    (a + 0.05) / (b + 0.05)
}

#[cfg(test)]
pub(crate) mod test_support {
    use image::{Rgb, RgbImage};

    /// Solid-color square. Useful for "uniform image" edge-case tests.
    pub fn solid(w: u32, h: u32, color: [u8; 3]) -> RgbImage {
        let mut img = RgbImage::new(w, h);
        for px in img.pixels_mut() {
            *px = Rgb(color);
        }
        img
    }

    /// Sharp vertical bar in the right half of the image. Gives
    /// directional edge content with predictable placement.
    pub fn right_half_bar(w: u32, h: u32) -> RgbImage {
        let mut img = RgbImage::new(w, h);
        for (x, _y, px) in img.enumerate_pixels_mut() {
            *px = if x >= w / 2 {
                Rgb([255, 255, 255])
            } else {
                Rgb([0, 0, 0])
            };
        }
        img
    }

    /// Random-feeling checkerboard. Used as the "all edges" edge case —
    /// every pixel boundary is a step, so edge density saturates.
    pub fn checkerboard(w: u32, h: u32, cell: u32) -> RgbImage {
        let mut img = RgbImage::new(w, h);
        for (x, y, px) in img.enumerate_pixels_mut() {
            let on = ((x / cell) + (y / cell)) % 2 == 0;
            *px = if on {
                Rgb([255, 255, 255])
            } else {
                Rgb([0, 0, 0])
            };
        }
        img
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_roundtrip() {
        let c = Rgb::parse_hex("#ff8040").unwrap();
        assert_eq!(c.to_hex(), "#ff8040");
    }

    #[test]
    fn short_hex_parses() {
        let c = Rgb::parse_hex("#f80").unwrap();
        assert_eq!(c.r, 255);
        assert_eq!(c.g, 136);
        assert_eq!(c.b, 0);
    }

    #[test]
    fn luminance_extremes() {
        assert!(Rgb::WHITE.relative_luminance() > 0.99);
        assert!(Rgb::BLACK.relative_luminance() < 0.01);
    }

    #[test]
    fn contrast_white_on_black_is_21() {
        let l1 = Rgb::WHITE.relative_luminance();
        let l2 = Rgb::BLACK.relative_luminance();
        let ratio = wcag_contrast_ratio(l1, l2);
        assert!((ratio - 21.0).abs() < 0.01, "got {ratio}");
    }
}
