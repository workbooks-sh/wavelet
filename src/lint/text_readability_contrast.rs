//! Contrast pass for the `text-readability` rule.
//!
//! Layered on top of the cap-height check in `text_readability.rs`. Where
//! the cap-height pass walks the resolved scene-graph alone, this one
//! also needs *pixels* — the foreground/background contrast under a text
//! element only becomes apparent once the underlying video / gradient /
//! image is composited.
//!
//! Pipeline per scene + sample-time:
//!   1. Render the scene HTML to RGBA via the existing `Renderer`. Cached
//!      across all text-bearing elements in the same `(scene_path, t_secs)`.
//!   2. For each text candidate (same predicate as cap-height), convert
//!      its document-space bbox into fractional canvas coords (0..1) and
//!      dispatch the `contrast_in_region` shader at `min_contrast = 4.5`
//!      (WCAG AA body-text floor).
//!   3. Translate the `AssertionOutcome` back into a `LintFinding` with
//!      `subkind = "contrast"` so the dedup logic in `handlers/lint.rs`
//!      keeps it independent of any cap-height finding on the same node.
//!
//! Findings emitted:
//!   - ratio < 3.0  → Error (below the large-text floor)
//!   - ratio < 4.5  → Warn  (below AA body, may still be OK for large display)
//!   - ratio >= 4.5 → no finding

use super::report::{LintFinding, Severity};
use super::text_readability::{best_selector, is_text_candidate, SUBKIND_CONTRAST, RULE};
use crate::query::{FrameSnapshot, GlyphInk, NodeSnapshot, Rect};
use crate::render::{load_html_with_base, Renderer};
use crate::shader::assert::contrast_in_region::{assert_contrast, Region};
use crate::shader::assert::FrameSource;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// WCAG 2.2 AA body-text contrast floor. Below this body text is hard to
/// read for users with low vision; large/bold display type can use 3.0.
pub const MIN_CONTRAST_AA: f32 = 4.5;

/// Below this contrast even large display type falls under the AA floor —
/// treat as an outright Error since the text is broadly illegible.
pub const LARGE_TEXT_FLOOR: f32 = 3.0;

/// Anti-aliasing fringe width in pixels. The halo measurement skips
/// pixels within this distance of the glyph ink boundary on both
/// sides — they're the rasterizer's blended alpha pixels, neither
/// pure glyph nor pure background. Without this exclusion, high-
/// contrast text reports artificially low ratios (black-on-white
/// dropped to ~2.5:1 in 008 because the fringe pulled the BG mean
/// toward gray). 2px covers the practical AA fringe for typography
/// in the 24-200px range we render.
pub const AA_FRINGE_PX: f32 = 2.0;

/// Lazy per-(scene_path, t_secs) RGBA cache. Multiple text elements in
/// one scene + frame share one render — the typical case is 3+ headlines
/// per scene, so caching turns N renders into 1.
pub struct ContrastFrameCache {
    cache: HashMap<(PathBuf, u32), Option<RenderedFrame>>,
}

/// One rendered RGBA frame, in row-major top-down sRGB layout. Used by
/// both the lint-time scene render and the post-render MP4 sampler.
pub struct RenderedFrame {
    /// Frame width in pixels.
    pub width: u32,
    /// Frame height in pixels.
    pub height: u32,
    /// Pixel buffer, 4 bytes (R, G, B, A) per pixel, row-major top-down.
    pub rgba: Vec<u8>,
}

impl ContrastFrameCache {
    /// Construct an empty cache.
    pub fn new() -> Self {
        Self { cache: HashMap::new() }
    }

    /// Render `scene_path` at `t_secs` to RGBA. `width` × `height` is the
    /// canvas the snapshot was taken against; using a different size here
    /// would invalidate the fractional bboxes downstream.
    fn frame_for(
        &mut self,
        scene_path: &Path,
        t_secs: f32,
        width: u32,
        height: u32,
    ) -> Option<&RenderedFrame> {
        // Quantize t_secs to milliseconds so floating-point jitter on the
        // sample-time list (0.11, 0.5, 1.0) doesn't bypass the cache.
        let key = (scene_path.to_path_buf(), (t_secs * 1000.0).round() as u32);
        if !self.cache.contains_key(&key) {
            let rendered = render_scene(scene_path, t_secs, width, height);
            self.cache.insert(key.clone(), rendered);
        }
        self.cache.get(&key).and_then(|opt| opt.as_ref())
    }
}

impl Default for ContrastFrameCache {
    fn default() -> Self {
        Self::new()
    }
}

fn render_scene(
    scene_path: &Path,
    t_secs: f32,
    width: u32,
    height: u32,
) -> Option<RenderedFrame> {
    let html = std::fs::read_to_string(scene_path).ok()?;
    let absolute = std::fs::canonicalize(scene_path).unwrap_or_else(|_| scene_path.to_path_buf());
    let base_url = url::Url::from_file_path(&absolute).ok().map(|u| u.to_string());
    let mut doc = load_html_with_base(&html, width, height, base_url);
    doc.as_mut().resolve(t_secs as f64);
    let mut renderer = Renderer::new(width, height);
    let rgba = renderer.render(doc.as_mut());
    Some(RenderedFrame { width, height, rgba })
}

/// Run the contrast pass against one scene snapshot. Returns one finding
/// per text element whose contrast under the rendered scene is below the
/// AA floor.
pub fn run(
    snap: &FrameSnapshot,
    scene_path: &Path,
    cache: &mut ContrastFrameCache,
) -> Vec<LintFinding> {
    let (canvas_w, canvas_h) = snap.viewport;
    if canvas_w == 0 || canvas_h == 0 {
        return Vec::new();
    }

    let Some(frame) = cache.frame_for(scene_path, snap.t_secs, canvas_w, canvas_h) else {
        return Vec::new();
    };

    run_against_frame(snap, scene_path, frame)
}

/// Run the contrast pass against a caller-provided frame buffer. This
/// is what the post-render pass uses to feed actual MP4 frames into
/// the same halo measurement the HTML-render pass uses. The scene
/// snapshot still provides text element positions; only the pixel
/// background changes.
pub fn run_against_frame(
    snap: &FrameSnapshot,
    scene_path: &Path,
    frame: &RenderedFrame,
) -> Vec<LintFinding> {
    let (canvas_w, canvas_h) = snap.viewport;
    if canvas_w == 0 || canvas_h == 0 {
        return Vec::new();
    }
    let mut findings = Vec::new();
    let mut seen: Vec<usize> = Vec::new();
    for (idx, node) in snap.nodes.iter().enumerate() {
        if !is_text_candidate(node) {
            continue;
        }
        if !node.bbox.has_area() {
            continue;
        }
        if node.computed_opacity <= 0.0 {
            continue;
        }
        if seen.contains(&node.id) {
            continue;
        }
        seen.push(node.id);

        // Primary path: when Parley gave us glyph ink rects, measure
        // contrast as mean luminance of foreground (glyph ink pixels)
        // vs background (a halo just outside the ink). Robust against
        // image / video backgrounds — sampling actual rendered pixels
        // in two well-defined zones is the only thing that catches
        // white-text-on-bright-counter cases that bbox-region min/max
        // can't see through. Fall back to the legacy shader region
        // scan when no glyph ink is available (non-Parley text paths).
        let (ratio, measurement_detail) = if let Some(run) = node.glyph_run.as_ref() {
            match halo_contrast_ratio(node, &run.glyphs, frame) {
                Some(r) => (r, "glyph vs halo"),
                // Halo computation can return None if every glyph
                // lands off-canvas or the halo collapses (e.g. tiny
                // text). Fall back to the legacy region scan rather
                // than silently dropping the element.
                None => match legacy_region_ratio(node, frame, canvas_w as f32, canvas_h as f32) {
                    Some(r) => (r, "region scan"),
                    None => continue,
                },
            }
        } else {
            match legacy_region_ratio(node, frame, canvas_w as f32, canvas_h as f32) {
                Some(r) => (r, "region scan"),
                None => continue,
            }
        };

        if ratio >= MIN_CONTRAST_AA {
            continue;
        }

        let severity = if ratio < LARGE_TEXT_FLOOR {
            Severity::Error
        } else {
            Severity::Warn
        };
        let selector = best_selector(snap, node, idx);
        findings.push(LintFinding {
            rule: RULE.to_string(),
            severity,
            scene_path: scene_path.to_path_buf(),
            t_secs: snap.t_secs,
            element_selector: selector,
            element_bbox: node.bbox,
            message: format!(
                "contrast-ratio {:.1} ({}) — text vs the actual rendered \
                 background fails the WCAG AA {:.1}:1 floor. Reads as muddy / \
                 illegible against the underlay.",
                ratio, measurement_detail, MIN_CONTRAST_AA,
            ),
            fix_hint: build_fix_hint(severity, ratio),
            subkind: Some(SUBKIND_CONTRAST.to_string()),
        });
    }

    findings
}

/// Halo-based contrast: mean luminance of pixels under the glyph ink
/// vs mean luminance of a halo ring just outside it. Returns `None`
/// when either mask is empty (text fully off-canvas, glyphs of zero
/// area, or halo dilation lands outside the frame).
///
/// Halo width scales with the element's computed font size — bigger
/// type gets a wider halo, matching how a human eye reads the
/// surrounding background context relative to the glyph stroke width.
/// We clamp to a sensible range so 12px captions and 200px display
/// type both produce useful halos.
///
/// Critical: an **exclusion ring** of `AA_FRINGE_PX` pixels straddles
/// the ink boundary on both sides. Pixels inside the ring are sampled
/// by NEITHER the foreground nor the background mask. This skips the
/// anti-aliased glyph fringe — the gray pixels just outside the ink
/// rect where the rasterizer blends the glyph color into the
/// background. Without the ring, black text on white reported ~2.5:1
/// contrast (the fringe pulled the BG mean toward gray) — see the v8
/// post-mortem and `flags_close_high_contrast_text_with_aa_fringe`
/// test below.
pub fn halo_contrast_ratio(
    node: &NodeSnapshot,
    glyphs: &[GlyphInk],
    frame: &RenderedFrame,
) -> Option<f32> {
    if glyphs.is_empty() {
        return None;
    }
    let halo_px = halo_width_for(node.computed_font_size_px);

    // Walk the inflated bbox + bbox-clamped halo region as before, but
    // collect raw pixel luminance into a single histogram instead of
    // splitting by ink-rect-vs-halo geometry.
    //
    // Why: Parley's per-glyph "ink rect" is the glyph's BOUNDING BOX,
    // not its painted pixels. Inside the ink rect of 'N' or 'O', the
    // majority of pixels are white background (the negative space
    // between strokes). The geometric FG mask included those whites
    // and reported gray means for what should be near-black glyphs.
    // The 008 wordmark wordmark debug confirmed: fg_count=27932
    // mean_L=0.347 for solid black text on white. Ratio collapsed to
    // 2.6:1 when the true contrast was ~21:1.
    //
    // Pixel-luminance class separation is geometry-free: sort all
    // sampled pixels by L, take the darker quartile as the foreground
    // class and the lighter quartile as the background class. Means
    // of each class give the perceptually-correct WCAG ratio.
    let inflated = inflated_bounds(node.bbox, glyphs, halo_px);
    let element_bbox = expand_bbox_if_tight(node.bbox, glyphs, halo_px);
    let clipped = Rect {
        x: inflated.x.max(element_bbox.x),
        y: inflated.y.max(element_bbox.y),
        w: (inflated.x + inflated.w).min(element_bbox.x + element_bbox.w)
            - inflated.x.max(element_bbox.x),
        h: (inflated.y + inflated.h).min(element_bbox.y + element_bbox.h)
            - inflated.y.max(element_bbox.y),
    };
    if clipped.w <= 0.0 || clipped.h <= 0.0 {
        return None;
    }
    let (x0, y0, x1, y1) = clamp_to_frame(clipped, frame.width, frame.height);
    if x1 <= x0 || y1 <= y0 {
        return None;
    }

    let ox = node.bbox.x;
    let oy = node.bbox.y;
    let mut lums: Vec<f32> = Vec::new();
    for py in y0..y1 {
        let py_f = py as f32 + 0.5;
        for px in x0..x1 {
            let px_f = px as f32 + 0.5;
            // Include a pixel iff it sits within the halo radius of
            // any glyph ink rect. This still keeps the measurement
            // tied to the text region — random pixels far from any
            // glyph (large empty bbox padding) don't pollute the
            // histogram.
            let mut in_region = false;
            for g in glyphs {
                let ix0 = ox + g.ink_min_x;
                let iy0 = oy + g.ink_min_y;
                let ix1 = ox + g.ink_max_x;
                let iy1 = oy + g.ink_max_y;
                if !(ix1 > ix0 && iy1 > iy0) {
                    continue;
                }
                if px_f >= ix0 - halo_px
                    && px_f < ix1 + halo_px
                    && py_f >= iy0 - halo_px
                    && py_f < iy1 + halo_px
                {
                    in_region = true;
                    break;
                }
            }
            if in_region {
                lums.push(pixel_luminance(&frame.rgba, frame.width, px, py));
            }
        }
    }

    if lums.len() < 64 {
        // Too few pixels for a stable measurement.
        return None;
    }
    // Sort ascending and take the extreme 5% of each tail as the
    // foreground / background luminance representatives. Smaller than
    // quartiles (25%) because thin / italic / sparse-stroke type only
    // paints a few percent of the bbox in real glyph ink; a 25% slice
    // dilutes the dark_mean with AA fringe pixels and collapses the
    // ratio (008 `.colorway-label` Bodoni italic dropped to 1.7 with
    // quartile; 5% recovers ~13). 5% keeps a stable sample size for
    // the typical bbox (44k pixels × 5% = 2200) while staying close
    // enough to the tails to land on actual painted ink.
    lums.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let n = lums.len();
    let q = (n / 20).max(32);
    let mut dark_sum = 0.0f64;
    let mut light_sum = 0.0f64;
    for i in 0..q {
        dark_sum += lums[i] as f64;
        light_sum += lums[n - 1 - i] as f64;
    }
    let dark_mean = (dark_sum / q as f64) as f32;
    let light_mean = (light_sum / q as f64) as f32;
    if std::env::var("WAVELET_LINT_DEBUG_HALO").is_ok() {
        eprintln!(
            "halo-debug bbox=({:.0},{:.0}) {:.0}x{:.0}  font={:.0}px halo={:.1}px  glyphs={}  pixels={} q={}  dark_mean={:.3} light_mean={:.3}  ratio={:.2}",
            node.bbox.x, node.bbox.y, node.bbox.w, node.bbox.h,
            node.computed_font_size_px, halo_px, glyphs.len(),
            n, q, dark_mean, light_mean, wcag_ratio(dark_mean, light_mean)
        );
    }
    Some(wcag_ratio(dark_mean, light_mean))
}

/// If the element's bbox is so tight against the glyph ink that
/// bbox-clamped halo measurement would have no background pixels
/// (rare in real text — line-height leaves margin), expand the bbox
/// outward by half a halo width so the measurement still has signal.
/// Otherwise return the bbox unchanged.
fn expand_bbox_if_tight(bbox: Rect, glyphs: &[GlyphInk], halo_px: f32) -> Rect {
    let ox = bbox.x;
    let oy = bbox.y;
    // Find the union of glyph ink rects to compare against the bbox.
    let mut ink_min_y = f32::INFINITY;
    let mut ink_max_y = f32::NEG_INFINITY;
    for g in glyphs {
        let y0 = oy + g.ink_min_y;
        let y1 = oy + g.ink_max_y;
        if y1 > y0 {
            ink_min_y = ink_min_y.min(y0);
            ink_max_y = ink_max_y.max(y1);
        }
    }
    if !ink_min_y.is_finite() || !ink_max_y.is_finite() {
        return bbox;
    }
    // Tight = less than `halo_px / 2` of slack on both top and bottom.
    let top_slack = ink_min_y - bbox.y;
    let bottom_slack = (bbox.y + bbox.h) - ink_max_y;
    let tight = top_slack < halo_px * 0.5 || bottom_slack < halo_px * 0.5;
    if !tight {
        return bbox;
    }
    let pad = halo_px * 0.5;
    Rect {
        x: bbox.x - pad,
        y: bbox.y - pad,
        w: bbox.w + 2.0 * pad,
        h: bbox.h + 2.0 * pad,
    }
}

/// Halo dilation width in CSS px, derived from the element's font size.
/// 35% of font size lands between roughly 0.5× and 1× cap-height for
/// typical type — wide enough to escape glyph anti-aliasing fringe,
/// narrow enough to stay within the immediate background context.
fn halo_width_for(font_size_px: f32) -> f32 {
    if font_size_px <= 0.0 {
        return 6.0;
    }
    (font_size_px * 0.35).clamp(3.0, 36.0)
}

/// Tight bounding rect that contains every glyph ink rect plus the
/// halo dilation. Returned in document pixels.
fn inflated_bounds(bbox: Rect, glyphs: &[GlyphInk], halo_px: f32) -> Rect {
    let ox = bbox.x;
    let oy = bbox.y;
    let mut min_x = f32::INFINITY;
    let mut min_y = f32::INFINITY;
    let mut max_x = f32::NEG_INFINITY;
    let mut max_y = f32::NEG_INFINITY;
    for g in glyphs {
        let ix0 = ox + g.ink_min_x;
        let iy0 = oy + g.ink_min_y;
        let ix1 = ox + g.ink_max_x;
        let iy1 = oy + g.ink_max_y;
        if !(ix1 > ix0 && iy1 > iy0) {
            continue;
        }
        if ix0 < min_x {
            min_x = ix0;
        }
        if iy0 < min_y {
            min_y = iy0;
        }
        if ix1 > max_x {
            max_x = ix1;
        }
        if iy1 > max_y {
            max_y = iy1;
        }
    }
    if !(min_x.is_finite() && min_y.is_finite() && max_x.is_finite() && max_y.is_finite()) {
        return Rect { x: 0.0, y: 0.0, w: 0.0, h: 0.0 };
    }
    Rect {
        x: min_x - halo_px,
        y: min_y - halo_px,
        w: (max_x - min_x) + 2.0 * halo_px,
        h: (max_y - min_y) + 2.0 * halo_px,
    }
}

fn clamp_to_frame(rect: Rect, w: u32, h: u32) -> (u32, u32, u32, u32) {
    let x0 = rect.x.floor().clamp(0.0, w as f32) as u32;
    let y0 = rect.y.floor().clamp(0.0, h as f32) as u32;
    let x1 = (rect.x + rect.w).ceil().clamp(0.0, w as f32) as u32;
    let y1 = (rect.y + rect.h).ceil().clamp(0.0, h as f32) as u32;
    (x0, y0, x1, y1)
}

/// Sample one RGBA pixel and return WCAG relative luminance (0..1).
/// Frame is assumed row-major top-down sRGB; we ignore alpha because
/// the renderer pre-multiplies onto an opaque background.
fn pixel_luminance(rgba: &[u8], frame_w: u32, x: u32, y: u32) -> f32 {
    let idx = ((y * frame_w + x) * 4) as usize;
    if idx + 2 >= rgba.len() {
        return 0.0;
    }
    let r = srgb_to_linear(rgba[idx] as f32 / 255.0);
    let g = srgb_to_linear(rgba[idx + 1] as f32 / 255.0);
    let b = srgb_to_linear(rgba[idx + 2] as f32 / 255.0);
    0.2126 * r + 0.7152 * g + 0.0722 * b
}

/// WCAG sRGB → linear transfer.
fn srgb_to_linear(c: f32) -> f32 {
    if c <= 0.03928 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

/// WCAG 2.2 contrast ratio between two relative-luminance values.
fn wcag_ratio(a: f32, b: f32) -> f32 {
    let (lo, hi) = if a < b { (a, b) } else { (b, a) };
    (hi + 0.05) / (lo + 0.05)
}

/// Legacy region-scan path. Calls the GPU `assert_contrast` shader
/// which reports `lightest_pixel / darkest_pixel` in the bbox. Used
/// as fallback for text elements without glyph ink data (non-Parley
/// pipelines, e.g. raw SVG text). Returns the measured ratio, or
/// `None` if the shader reports `region_not_found` / `numerical_issue`.
fn legacy_region_ratio(
    node: &NodeSnapshot,
    frame: &RenderedFrame,
    canvas_w: f32,
    canvas_h: f32,
) -> Option<f32> {
    let region = bbox_to_region(node, canvas_w, canvas_h)?;
    let frame_source = FrameSource::Rgba8 {
        width: frame.width,
        height: frame.height,
        pixels: frame.rgba.clone(),
    };
    let outcome = assert_contrast(frame_source, region, MIN_CONTRAST_AA).ok()?;
    if outcome.reason_code == 2 || outcome.reason_code == 4 {
        return None;
    }
    Some(outcome.evidence.first().copied().unwrap_or(1.0))
}

fn bbox_to_region(node: &NodeSnapshot, canvas_w: f32, canvas_h: f32) -> Option<Region> {
    if canvas_w <= 0.0 || canvas_h <= 0.0 {
        return None;
    }
    // Clip to [0, 1] — bboxes can extend off-canvas after CSS transforms;
    // the shader rejects out-of-range regions with reason_code 2 which we
    // then treat as "no signal" above.
    let x = (node.bbox.x / canvas_w).clamp(0.0, 1.0);
    let y = (node.bbox.y / canvas_h).clamp(0.0, 1.0);
    let r = ((node.bbox.x + node.bbox.w) / canvas_w).clamp(0.0, 1.0);
    let b = ((node.bbox.y + node.bbox.h) / canvas_h).clamp(0.0, 1.0);
    let w = (r - x).max(0.0);
    let h = (b - y).max(0.0);
    if w <= 0.0 || h <= 0.0 {
        return None;
    }
    Some(Region { x, y, w, h })
}

fn build_fix_hint(severity: Severity, ratio: f32) -> String {
    match severity {
        Severity::Error => format!(
            "contrast-ratio {:.1} is below the 3.0 large-text floor — even \
             oversized display type is unreadable here. Bump text fg to \
             white (#ffffff) or add a scrim layer (semi-transparent dark \
             rect) beneath the text. WCAG AA body text requires ≥ 4.5:1.",
            ratio,
        ),
        _ => format!(
            "contrast-ratio {:.1} is below the 4.5:1 WCAG AA body-text floor. \
             Bump text fg to white (#ffffff) or add a scrim layer \
             (semi-transparent dark rect) beneath the text. Bold or \
             ≥18pt-equivalent display type can use ≥ 3.0:1.",
            ratio,
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bbox_to_region_clips_off_canvas() {
        let n = NodeSnapshot {
            id: 1,
            semantic_id: "x".into(),
            tag: "p".into(),
            element_id: None,
            classes: vec![],
            bbox: crate::query::Rect { x: -50.0, y: -50.0, w: 200.0, h: 100.0 },
            transform: None,
            clips_overflow: false,
            computed_opacity: 1.0,
            computed_font_size_px: 40.0,
            text: Some("hi".into()),
            children: vec![],
            parent: None,
            glyph_run: None,
            flex_axis: None,
        };
        let r = bbox_to_region(&n, 1000.0, 1000.0).unwrap();
        assert!((r.x - 0.0).abs() < 1e-6);
        assert!((r.y - 0.0).abs() < 1e-6);
        assert!((r.w - 0.15).abs() < 1e-6);
        assert!((r.h - 0.05).abs() < 1e-6);
    }

    #[test]
    fn bbox_fully_off_canvas_skips() {
        let n = NodeSnapshot {
            id: 1,
            semantic_id: "x".into(),
            tag: "p".into(),
            element_id: None,
            classes: vec![],
            bbox: crate::query::Rect { x: -500.0, y: -500.0, w: 100.0, h: 100.0 },
            transform: None,
            clips_overflow: false,
            computed_opacity: 1.0,
            computed_font_size_px: 40.0,
            text: Some("hi".into()),
            children: vec![],
            parent: None,
            glyph_run: None,
            flex_axis: None,
        };
        assert!(bbox_to_region(&n, 1000.0, 1000.0).is_none());
    }

    #[test]
    fn error_severity_below_large_text_floor() {
        let hint = build_fix_hint(Severity::Error, 1.5);
        assert!(hint.contains("3.0 large-text floor"));
    }

    #[test]
    fn warn_severity_in_body_text_band() {
        let hint = build_fix_hint(Severity::Warn, 3.8);
        assert!(hint.contains("WCAG AA body-text floor"));
    }

    /// Build a synthetic flat-fill RGBA frame for halo tests.
    fn solid_frame(w: u32, h: u32, rgb: [u8; 3]) -> RenderedFrame {
        let mut rgba = Vec::with_capacity((w * h * 4) as usize);
        for _ in 0..(w * h) {
            rgba.push(rgb[0]);
            rgba.push(rgb[1]);
            rgba.push(rgb[2]);
            rgba.push(255);
        }
        RenderedFrame { width: w, height: h, rgba }
    }

    /// Paint a filled rect into an existing RGBA frame.
    fn paint_rect(frame: &mut RenderedFrame, rect: (u32, u32, u32, u32), rgb: [u8; 3]) {
        let (x0, y0, x1, y1) = rect;
        for y in y0..y1.min(frame.height) {
            for x in x0..x1.min(frame.width) {
                let idx = ((y * frame.width + x) * 4) as usize;
                frame.rgba[idx] = rgb[0];
                frame.rgba[idx + 1] = rgb[1];
                frame.rgba[idx + 2] = rgb[2];
                frame.rgba[idx + 3] = 255;
            }
        }
    }

    /// One glyph ink rect, with element-local coordinates relative to
    /// the parent text node's bbox origin.
    fn glyph(min_x: f32, min_y: f32, max_x: f32, max_y: f32) -> GlyphInk {
        GlyphInk {
            pen_x: min_x,
            pen_y: max_y,
            ink_min_x: min_x,
            ink_min_y: min_y,
            ink_max_x: max_x,
            ink_max_y: max_y,
        }
    }

    /// Build a text-element NodeSnapshot at a given absolute bbox.
    fn text_node(bbox: Rect, font_size_px: f32) -> NodeSnapshot {
        NodeSnapshot {
            id: 1,
            semantic_id: "n".into(),
            tag: "h1".into(),
            element_id: None,
            classes: vec!["cta".into()],
            bbox,
            transform: None,
            clips_overflow: false,
            computed_opacity: 1.0,
            computed_font_size_px: font_size_px,
            text: Some("HI".into()),
            children: vec![],
            parent: None,
            glyph_run: None,
            flex_axis: None,
        }
    }

    #[test]
    fn halo_flags_white_text_on_bright_background() {
        // The v8 KitchenAid failure mode: white text painted directly
        // onto a near-white kitchen background. WCAG ratio should
        // collapse to ~1.0 — well below the AA floor.
        let mut frame = solid_frame(200, 100, [240, 240, 240]); // near-white background
        paint_rect(&mut frame, (40, 40, 160, 60), [255, 255, 255]); // white text strokes
        let node = text_node(Rect { x: 40.0, y: 40.0, w: 120.0, h: 20.0 }, 40.0);
        // Glyph ink fills the painted white region inside the bbox.
        let glyphs = vec![glyph(0.0, 0.0, 120.0, 20.0)];
        let ratio = halo_contrast_ratio(&node, &glyphs, &frame).unwrap();
        assert!(
            ratio < MIN_CONTRAST_AA,
            "white-on-near-white must fail AA, got {ratio}"
        );
    }

    #[test]
    fn halo_passes_white_text_on_dark_background() {
        // The textbook readable case: white text on dark scrim. Ratio
        // should be close to maximum (white vs near-black ≈ 19:1).
        let mut frame = solid_frame(200, 100, [16, 16, 16]); // dark background
        paint_rect(&mut frame, (40, 40, 160, 60), [255, 255, 255]); // white text
        let node = text_node(Rect { x: 40.0, y: 40.0, w: 120.0, h: 20.0 }, 40.0);
        let glyphs = vec![glyph(0.0, 0.0, 120.0, 20.0)];
        let ratio = halo_contrast_ratio(&node, &glyphs, &frame).unwrap();
        assert!(
            ratio >= MIN_CONTRAST_AA,
            "white-on-dark must pass AA, got {ratio}"
        );
    }

    #[test]
    fn halo_flags_color_on_color_close_luminance() {
        // Author chose two colors that look distinct on a swatch but
        // have similar luminance — the rule should still flag because
        // they read as muddy when one is text on the other.
        let mut frame = solid_frame(200, 100, [200, 60, 60]); // saturated red bg
        paint_rect(&mut frame, (40, 40, 160, 60), [60, 130, 60]); // similar-luma green text
        let node = text_node(Rect { x: 40.0, y: 40.0, w: 120.0, h: 20.0 }, 40.0);
        let glyphs = vec![glyph(0.0, 0.0, 120.0, 20.0)];
        let ratio = halo_contrast_ratio(&node, &glyphs, &frame).unwrap();
        assert!(
            ratio < MIN_CONTRAST_AA,
            "color-on-color with close luminance must fail AA, got {ratio}"
        );
    }

    #[test]
    fn halo_width_scales_with_font_size() {
        // Sanity: 12px caption gets the floor, 200px display gets the
        // ceiling, mid-size scales linearly.
        assert!((halo_width_for(12.0) - 4.2).abs() < 0.001);
        assert_eq!(halo_width_for(200.0), 36.0);
        assert_eq!(halo_width_for(0.0), 6.0);
    }

    #[test]
    fn wcag_ratio_white_on_black_is_21() {
        // Sanity for the WCAG transfer: pure white vs pure black ≈ 21:1.
        let white = 1.0;
        let black = 0.0;
        let ratio = wcag_ratio(white, black);
        assert!((ratio - 21.0).abs() < 0.001, "got {ratio}");
    }

    /// Reproduce the 008 false-positive: black text on white background
    /// with an anti-aliased gray fringe around each glyph. Pre-fix the
    /// fringe got sampled as background and the ratio collapsed to
    /// ~2.5:1. Post-fix the exclusion ring skips the fringe and the
    /// ratio recovers to ~21:1.
    #[test]
    fn flags_close_high_contrast_text_with_aa_fringe() {
        let mut frame = solid_frame(200, 100, [255, 255, 255]); // white BG
        // Paint a 120×20 ink rect of pure black.
        paint_rect(&mut frame, (40, 40, 160, 60), [0, 0, 0]);
        // Paint a 1px gray AA fringe immediately outside the ink rect.
        paint_rect(&mut frame, (39, 40, 40, 60), [128, 128, 128]);
        paint_rect(&mut frame, (160, 40, 161, 60), [128, 128, 128]);
        paint_rect(&mut frame, (40, 39, 160, 40), [128, 128, 128]);
        paint_rect(&mut frame, (40, 60, 160, 61), [128, 128, 128]);
        let node = text_node(Rect { x: 40.0, y: 40.0, w: 120.0, h: 20.0 }, 40.0);
        let glyphs = vec![glyph(0.0, 0.0, 120.0, 20.0)];
        let ratio = halo_contrast_ratio(&node, &glyphs, &frame).unwrap();
        assert!(
            ratio >= 15.0,
            "black-on-white WITH AA fringe must still report high contrast, \
             got {ratio} (pre-fix this dropped to ~2.5)"
        );
    }
}
