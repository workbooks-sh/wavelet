//! Aspect-ratio awareness for the render pipeline.
//!
//! Older wavelet versions tested every shot at 16:9, 9:16, 1:1, and 4:5; the
//! v3 rewrite dropped that surface and we need it back before we can
//! produce social-media variants alongside the default 16:9 master.
//!
//! This module ships:
//!
//! - The `AspectRatio` enum (5 variants, kebab-case serde) covering the
//!   shapes the director cares about today.
//! - Default frame dimensions per aspect, parameterized on a `base`
//!   short-edge — 720 for delivery, 360 for tests.
//! - `safe_areas` math: title-safe is a rectangle inset 10% off every
//!   edge; action-safe is 5%; `full` is the entire frame. The inset is
//!   proportional, identical for every aspect.
//!
//! The actual multi-aspect render (one `comp.json` → N MP4s) lives at
//! issue wb-lnhl. This crate's render loop still consumes the existing
//! `width` / `height` fields on `Composition`; `aspect` is purely
//! informational + safe-area-math at this stage.

use serde::{Deserialize, Serialize};

/// Aspect ratios the pipeline understands.
///
/// Variant names spell out the shape ("Landscape16x9" rather than just
/// `_16x9`) because the enum participates in CLI parsing, JSON
/// serialization, and log lines — readability beats brevity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AspectRatio {
    /// 16:9 — default master format, YouTube / TV / standard hero video.
    #[serde(rename = "16:9")]
    Landscape16x9,
    /// 9:16 — vertical, TikTok / Reels / Stories.
    #[serde(rename = "9:16")]
    Vertical9x16,
    /// 1:1 — square, Instagram feed / podcast clips.
    #[serde(rename = "1:1")]
    Square1x1,
    /// 4:5 — portrait, Instagram feed (taller crop than 1:1).
    #[serde(rename = "4:5")]
    Portrait4x5,
    /// 21:9 — cinematic ultra-wide / letterbox.
    #[serde(rename = "21:9")]
    Cinematic21x9,
}

/// A rectangle in pixel coordinates from the frame's top-left.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct BoundingRect {
    /// Left edge, in pixels.
    pub x: u32,
    /// Top edge, in pixels.
    pub y: u32,
    /// Width, in pixels.
    pub w: u32,
    /// Height, in pixels.
    pub h: u32,
}

/// Standard broadcast-style safe areas inside a frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SafeAreas {
    /// Title-safe (~10% inset off every edge). Headlines, lower-thirds,
    /// any text the director cares about staying visible on cropped
    /// previews live here.
    pub title: BoundingRect,
    /// Action-safe (~5% inset off every edge). Anything important
    /// should at minimum sit inside this rectangle.
    pub action: BoundingRect,
    /// Full frame (zero inset). Useful as a reference rectangle and
    /// for code that just wants the frame's pixel dimensions packaged
    /// the same way as the safe areas.
    pub full: BoundingRect,
}

const TITLE_SAFE_INSET: f32 = 0.10;
const ACTION_SAFE_INSET: f32 = 0.05;

impl AspectRatio {
    /// Parse the canonical `"W:H"` string. Returns `None` for unknown
    /// shapes — callers handle the error context (CLI prints usage, JSON
    /// loader emits a friendly serde error).
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim() {
            "16:9" => Some(Self::Landscape16x9),
            "9:16" => Some(Self::Vertical9x16),
            "1:1" => Some(Self::Square1x1),
            "4:5" => Some(Self::Portrait4x5),
            "21:9" => Some(Self::Cinematic21x9),
            _ => None,
        }
    }

    /// The canonical `"W:H"` string for this aspect.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Landscape16x9 => "16:9",
            Self::Vertical9x16 => "9:16",
            Self::Square1x1 => "1:1",
            Self::Portrait4x5 => "4:5",
            Self::Cinematic21x9 => "21:9",
        }
    }

    /// Default render dimensions for this aspect, parameterized on a
    /// `base` short-edge in pixels.
    ///
    /// `base` is the *short edge* of the frame for landscape / portrait
    /// shapes and the only edge for square. At `base = 720` the
    /// defaults land at:
    ///
    /// | aspect | dimensions   |
    /// |--------|--------------|
    /// | 16:9   | 1280 × 720   |
    /// | 9:16   | 720 × 1280   |
    /// | 1:1    | 720 × 720    |
    /// | 4:5    | 720 × 900    |
    /// | 21:9   | 1680 × 720   |
    pub fn dimensions(self, base: u32) -> (u32, u32) {
        match self {
            Self::Landscape16x9 => (base * 16 / 9, base),
            Self::Vertical9x16 => (base, base * 16 / 9),
            Self::Square1x1 => (base, base),
            Self::Portrait4x5 => (base, base * 5 / 4),
            Self::Cinematic21x9 => (base * 21 / 9, base),
        }
    }
}

/// Compute title-safe / action-safe / full rectangles for a frame of
/// the given dimensions. Insets are proportional and identical across
/// every aspect — title-safe is 10% off each edge, action-safe is 5%.
///
/// Degenerate inputs (`width == 0` or `height == 0`) produce zero-sized
/// rectangles rather than panicking; downstream code can ignore them.
pub fn safe_areas(width: u32, height: u32) -> SafeAreas {
    SafeAreas {
        title: inset_rect(width, height, TITLE_SAFE_INSET),
        action: inset_rect(width, height, ACTION_SAFE_INSET),
        full: BoundingRect { x: 0, y: 0, w: width, h: height },
    }
}

fn inset_rect(width: u32, height: u32, frac: f32) -> BoundingRect {
    if width == 0 || height == 0 {
        return BoundingRect { x: 0, y: 0, w: 0, h: 0 };
    }
    let dx = (width as f32 * frac).round() as u32;
    let dy = (height as f32 * frac).round() as u32;
    BoundingRect {
        x: dx,
        y: dy,
        w: width.saturating_sub(dx * 2),
        h: height.saturating_sub(dy * 2),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_round_trips_each_variant() {
        for v in [
            AspectRatio::Landscape16x9,
            AspectRatio::Vertical9x16,
            AspectRatio::Square1x1,
            AspectRatio::Portrait4x5,
            AspectRatio::Cinematic21x9,
        ] {
            assert_eq!(AspectRatio::parse(v.as_str()), Some(v));
        }
    }

    #[test]
    fn parse_rejects_unknown_shapes() {
        assert!(AspectRatio::parse("3:2").is_none());
        assert!(AspectRatio::parse("").is_none());
        assert!(AspectRatio::parse("16x9").is_none());
    }

    #[test]
    fn dimensions_at_720_base_are_sane() {
        assert_eq!(AspectRatio::Landscape16x9.dimensions(720), (1280, 720));
        assert_eq!(AspectRatio::Vertical9x16.dimensions(720), (720, 1280));
        assert_eq!(AspectRatio::Square1x1.dimensions(720), (720, 720));
        assert_eq!(AspectRatio::Portrait4x5.dimensions(720), (720, 900));
        assert_eq!(AspectRatio::Cinematic21x9.dimensions(720), (1680, 720));
    }

    #[test]
    fn dimensions_scale_with_base() {
        let (w, h) = AspectRatio::Landscape16x9.dimensions(360);
        assert_eq!((w, h), (640, 360));
        let (w, h) = AspectRatio::Vertical9x16.dimensions(360);
        assert_eq!((w, h), (360, 640));
    }

    #[test]
    fn safe_areas_at_1280x720_title_is_128_inset() {
        let s = safe_areas(1280, 720);
        // 10% of 1280 = 128, 10% of 720 = 72.
        assert_eq!(s.title.x, 128);
        assert_eq!(s.title.y, 72);
        assert_eq!(s.title.w, 1024);
        assert_eq!(s.title.h, 576);
        // Action-safe is 5% → 64 / 36 inset.
        assert_eq!(s.action.x, 64);
        assert_eq!(s.action.y, 36);
        assert_eq!(s.action.w, 1152);
        assert_eq!(s.action.h, 648);
        // Full is the frame itself.
        assert_eq!(s.full, BoundingRect { x: 0, y: 0, w: 1280, h: 720 });
    }

    #[test]
    fn safe_areas_at_720x1280_vertical() {
        let s = safe_areas(720, 1280);
        // 10% of 720 = 72, 10% of 1280 = 128.
        assert_eq!(s.title.x, 72);
        assert_eq!(s.title.y, 128);
        assert_eq!(s.title.w, 576);
        assert_eq!(s.title.h, 1024);
    }

    #[test]
    fn safe_areas_zero_dims_dont_panic() {
        let s = safe_areas(0, 0);
        assert_eq!(s.title, BoundingRect { x: 0, y: 0, w: 0, h: 0 });
        assert_eq!(s.action, BoundingRect { x: 0, y: 0, w: 0, h: 0 });
        assert_eq!(s.full, BoundingRect { x: 0, y: 0, w: 0, h: 0 });

        let s = safe_areas(1920, 0);
        assert_eq!(s.title.w, 0);
        assert_eq!(s.action.w, 0);
    }

    #[test]
    fn aspect_serializes_as_canonical_string() {
        let j = serde_json::to_string(&AspectRatio::Landscape16x9).unwrap();
        assert_eq!(j, "\"16:9\"");
        let j = serde_json::to_string(&AspectRatio::Vertical9x16).unwrap();
        assert_eq!(j, "\"9:16\"");
        let back: AspectRatio = serde_json::from_str("\"4:5\"").unwrap();
        assert_eq!(back, AspectRatio::Portrait4x5);
    }
}
