//! Per-platform safe-zone table — loaded once from the embedded JSON
//! and consulted by the `safe-zone` rule. Values are pixel-precise
//! against a 1080×1920 reference; the consumer linearly scales them to
//! whatever the scene's actual canvas is.

use crate::query::Rect;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

const SAFE_ZONES_JSON: &str = include_str!("../../data/safe_zones.json");

/// Per-edge safe-zone inset, in reference-canvas pixels (1080×1920).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct SafeZone {
    /// Pixels from the top edge reserved by platform chrome.
    pub top_px: f32,
    /// Pixels from the bottom edge reserved by platform chrome.
    pub bottom_px: f32,
    /// Pixels from the left edge reserved by platform chrome.
    pub left_px: f32,
    /// Pixels from the right edge reserved by platform chrome.
    pub right_px: f32,
}

#[derive(Debug, Clone, Deserialize)]
struct ReferenceCanvas {
    width: f32,
    height: f32,
}

#[derive(Debug, Clone, Deserialize)]
struct SafeZoneFile {
    #[allow(dead_code)]
    schema_version: u32,
    reference_canvas: ReferenceCanvas,
    platforms: HashMap<String, Option<SafeZone>>,
}

/// Errors surfaceable from the loader.
#[derive(Debug)]
pub enum LintError {
    /// The embedded JSON file failed to parse.
    BadTable(String),
    /// Caller asked for a platform that isn't in the table.
    UnknownPlatform(String),
}

impl std::fmt::Display for LintError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LintError::BadTable(e) => write!(f, "safe-zone table malformed: {e}"),
            LintError::UnknownPlatform(p) => write!(f, "unknown platform: {p}"),
        }
    }
}

impl std::error::Error for LintError {}

/// Loaded table — owns the parsed platforms map plus the reference
/// canvas it was authored against.
pub struct SafeZoneTable {
    /// Reference width the values are scaled from (typically 1080).
    pub ref_width: f32,
    /// Reference height the values are scaled from (typically 1920).
    pub ref_height: f32,
    /// Platform name → optional `SafeZone`. `None` = no chrome danger,
    /// rule short-circuits to PASS.
    pub platforms: HashMap<String, Option<SafeZone>>,
}

/// Load the embedded table. The JSON is `include_str!`'d so the
/// binary has no filesystem dependency at runtime.
pub fn load_table() -> Result<SafeZoneTable, LintError> {
    let file: SafeZoneFile =
        serde_json::from_str(SAFE_ZONES_JSON).map_err(|e| LintError::BadTable(e.to_string()))?;
    Ok(SafeZoneTable {
        ref_width: file.reference_canvas.width,
        ref_height: file.reference_canvas.height,
        platforms: file.platforms,
    })
}

impl SafeZoneTable {
    /// Look up a platform. `Ok(None)` means "platform is in the table
    /// but has no chrome danger" (e.g. `youtube`). `Err(_)` means the
    /// name isn't in the table at all.
    pub fn get(&self, platform: &str) -> Result<Option<&SafeZone>, LintError> {
        self.platforms
            .get(platform)
            .map(|v| v.as_ref())
            .ok_or_else(|| LintError::UnknownPlatform(platform.to_string()))
    }

    /// Scale a reference-canvas safe zone to the actual canvas the
    /// scene was authored against.
    pub fn scaled(&self, zone: &SafeZone, canvas_w: f32, canvas_h: f32) -> SafeZone {
        let sx = canvas_w / self.ref_width;
        let sy = canvas_h / self.ref_height;
        SafeZone {
            top_px: zone.top_px * sy,
            bottom_px: zone.bottom_px * sy,
            left_px: zone.left_px * sx,
            right_px: zone.right_px * sx,
        }
    }
}

/// Build the four danger-zone rectangles (top, bottom, left, right
/// chrome strips) for the given canvas dimensions. Returns them in a
/// fixed order so callers can label findings consistently.
pub fn danger_zones(zone: &SafeZone, canvas_w: f32, canvas_h: f32) -> [Rect; 4] {
    [
        Rect {
            x: 0.0,
            y: 0.0,
            w: canvas_w,
            h: zone.top_px,
        },
        Rect {
            x: 0.0,
            y: canvas_h - zone.bottom_px,
            w: canvas_w,
            h: zone.bottom_px,
        },
        Rect {
            x: 0.0,
            y: 0.0,
            w: zone.left_px,
            h: canvas_h,
        },
        Rect {
            x: canvas_w - zone.right_px,
            y: 0.0,
            w: zone.right_px,
            h: canvas_h,
        },
    ]
}

/// Human-readable label for each danger zone, matching the order of
/// `danger_zones`.
pub const DANGER_LABELS: [&str; 4] = ["top-chrome", "bottom-chrome", "left-chrome", "right-chrome"];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_loads() {
        let t = load_table().unwrap();
        assert_eq!(t.ref_width, 1080.0);
        assert_eq!(t.ref_height, 1920.0);
        let tt = t.get("tiktok").unwrap().unwrap();
        assert_eq!(tt.top_px, 108.0);
        assert_eq!(tt.bottom_px, 320.0);
    }

    #[test]
    fn youtube_has_no_chrome() {
        let t = load_table().unwrap();
        assert!(t.get("youtube").unwrap().is_none());
    }

    #[test]
    fn scaling_preserves_proportions() {
        let t = load_table().unwrap();
        let z = t.get("tiktok").unwrap().unwrap();
        let s = t.scaled(z, 720.0, 1280.0);
        // 108 * (1280/1920) = 72; 320 * (1280/1920) ≈ 213.33
        assert!((s.top_px - 72.0).abs() < 0.01);
        assert!((s.bottom_px - 213.333).abs() < 0.01);
    }

    #[test]
    fn danger_zones_cover_edges() {
        let z = SafeZone {
            top_px: 100.0,
            bottom_px: 200.0,
            left_px: 50.0,
            right_px: 75.0,
        };
        let zs = danger_zones(&z, 1080.0, 1920.0);
        assert_eq!(zs[0].h, 100.0);
        assert_eq!(zs[1].y, 1720.0);
        assert_eq!(zs[2].w, 50.0);
        assert_eq!(zs[3].x, 1005.0);
    }

    #[test]
    fn unknown_platform_errors() {
        let t = load_table().unwrap();
        assert!(matches!(t.get("blip"), Err(LintError::UnknownPlatform(_))));
    }
}
