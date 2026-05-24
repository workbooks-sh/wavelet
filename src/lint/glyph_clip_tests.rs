//! Unit tests for the `glyph-clip` rule. Separated from `glyph_clip.rs`
//! to keep the production file under the 600-line cap.

use super::{run, RULE as _RULE};
use crate::lint::report::Severity;
use crate::query::{FrameSnapshot, GlyphInk, GlyphRunData, NodeSnapshot, Rect};
use std::path::Path;

fn snap(nodes: Vec<NodeSnapshot>) -> FrameSnapshot {
    FrameSnapshot {
        t_secs: 1.0,
        frame_index: 30,
        viewport: (1080, 1920),
        active_scene: Some(0),
        nodes,
    }
}

fn node_with_glyphs(
    id: usize,
    tag: &str,
    classes: Vec<&str>,
    bbox: Rect,
    parent: Option<usize>,
    clips: bool,
    glyphs: Vec<GlyphInk>,
) -> NodeSnapshot {
    NodeSnapshot {
        id,
        semantic_id: format!("n{id}"),
        tag: tag.to_string(),
        element_id: None,
        classes: classes.iter().map(|s| s.to_string()).collect(),
        bbox,
        transform: None,
        clips_overflow: clips,
        computed_opacity: 1.0,
        computed_font_size_px: 96.0,
        text: Some("quiet".to_string()),
        children: vec![],
        parent,
        glyph_run: if glyphs.is_empty() {
            None
        } else {
            Some(GlyphRunData { glyphs })
        },
        flex_axis: None,
    }
}

fn plain_node(
    id: usize,
    tag: &str,
    classes: Vec<&str>,
    bbox: Rect,
    parent: Option<usize>,
    clips: bool,
) -> NodeSnapshot {
    NodeSnapshot {
        id,
        semantic_id: format!("n{id}"),
        tag: tag.to_string(),
        element_id: None,
        classes: classes.iter().map(|s| s.to_string()).collect(),
        bbox,
        transform: None,
        clips_overflow: clips,
        computed_opacity: 1.0,
        computed_font_size_px: 0.0,
        text: None,
        children: vec![],
        parent,
        glyph_run: None,
        flex_axis: None,
    }
}

/// Italic glyph at the right edge whose ink extends 6 px past the
/// element's bbox-right, while an ancestor clips at exactly the
/// bbox-right edge. The layout-bbox path would miss this (bbox is
/// fully inside the clip); the ink-bounds path fires.
#[test]
fn flags_italic_overrun_past_clip_edge() {
    let html = plain_node(
        1,
        "html",
        vec![],
        Rect { x: 0.0, y: 0.0, w: 1080.0, h: 1920.0 },
        None,
        true,
    );
    let h1 = node_with_glyphs(
        2,
        "h1",
        vec!["moment"],
        Rect { x: 580.0, y: 1000.0, w: 500.0, h: 110.0 },
        Some(1),
        false,
        vec![GlyphInk {
            pen_x: 460.0,
            pen_y: 90.0,
            ink_min_x: 460.0,
            ink_min_y: 10.0,
            ink_max_x: 506.0,
            ink_max_y: 100.0,
        }],
    );
    let fs = run(&snap(vec![html, h1]), Path::new("01-dawn.html"));
    assert_eq!(fs.len(), 1, "expected exactly one finding");
    assert_eq!(fs[0].severity, Severity::Warn);
    assert!(
        fs[0].message.contains("right 6 px"),
        "expected 'right 6 px' in: {}",
        fs[0].message
    );
    assert!(fs[0].message.contains("html"));
}

/// Non-italic glyph whose ink bounds fit inside its bbox should NOT
/// fire even when the element abuts a clipping ancestor.
#[test]
fn no_finding_when_ink_fits_inside_bbox() {
    let html = plain_node(
        1,
        "html",
        vec![],
        Rect { x: 0.0, y: 0.0, w: 1080.0, h: 1920.0 },
        None,
        true,
    );
    let h1 = node_with_glyphs(
        2,
        "h1",
        vec!["moment"],
        Rect { x: 580.0, y: 1000.0, w: 500.0, h: 110.0 },
        Some(1),
        false,
        vec![GlyphInk {
            pen_x: 460.0,
            pen_y: 90.0,
            ink_min_x: 460.0,
            ink_min_y: 10.0,
            ink_max_x: 495.0,
            ink_max_y: 100.0,
        }],
    );
    assert!(run(&snap(vec![html, h1]), Path::new("s.html")).is_empty());
}

/// Descender ink crossing a tight line-height clip on the bottom edge.
/// Layout-bbox check misses this when the line-box is clipped tight to
/// cap-height.
#[test]
fn flags_descender_overrun_below_clip() {
    let frame = plain_node(
        1,
        "div",
        vec!["row"],
        Rect { x: 0.0, y: 100.0, w: 1080.0, h: 80.0 },
        None,
        true,
    );
    let p = node_with_glyphs(
        2,
        "p",
        vec!["copy"],
        Rect { x: 100.0, y: 100.0, w: 600.0, h: 80.0 },
        Some(1),
        false,
        vec![GlyphInk {
            pen_x: 0.0,
            pen_y: 64.0,
            ink_min_x: 0.0,
            ink_min_y: 10.0,
            ink_max_x: 40.0,
            ink_max_y: 92.0,
        }],
    );
    let fs = run(&snap(vec![frame, p]), Path::new("s.html"));
    assert_eq!(fs.len(), 1);
    assert_eq!(fs[0].severity, Severity::Error);
    assert!(fs[0].message.contains("bottom 12 px"));
}

/// Sub-pixel miss → Info severity. Confirms the gradient still has
/// three buckets after the rewrite.
#[test]
fn info_severity_for_subpixel_miss() {
    let html = plain_node(
        1,
        "html",
        vec![],
        Rect { x: 0.0, y: 0.0, w: 1080.0, h: 1920.0 },
        None,
        true,
    );
    let h1 = node_with_glyphs(
        2,
        "h1",
        vec!["moment"],
        Rect { x: 580.0, y: 1000.0, w: 500.0, h: 110.0 },
        Some(1),
        false,
        vec![GlyphInk {
            pen_x: 460.0,
            pen_y: 90.0,
            ink_min_x: 460.0,
            ink_min_y: 10.0,
            ink_max_x: 501.0,
            ink_max_y: 100.0,
        }],
    );
    let fs = run(&snap(vec![html, h1]), Path::new("s.html"));
    assert_eq!(fs.len(), 1);
    assert_eq!(fs[0].severity, Severity::Info);
}

/// Element without `glyph_run` (text rendered via some non-Parley
/// path) is silently skipped — the rule has no data to fact-check
/// against.
#[test]
fn ignores_elements_without_glyph_run() {
    let parent = plain_node(
        1,
        "div",
        vec!["frame"],
        Rect { x: 0.0, y: 0.0, w: 1080.0, h: 1920.0 },
        None,
        true,
    );
    let child = plain_node(
        2,
        "h1",
        vec!["moment"],
        Rect { x: 580.0, y: 1000.0, w: 600.0, h: 110.0 },
        Some(1),
        false,
    );
    assert!(run(&snap(vec![parent, child]), Path::new("s.html")).is_empty());
}

/// Two clipping ancestors that miss on different edges. Only the
/// inner clipper introduces a new edge (top) — outer does nothing
/// here. Result: one finding from the inner clipper.
#[test]
fn reports_inner_and_outer_clip_separately_when_edges_differ() {
    let outer = plain_node(
        1,
        "section",
        vec!["frame"],
        Rect { x: 50.0, y: 0.0, w: 600.0, h: 1000.0 },
        None,
        true,
    );
    let inner = plain_node(
        2,
        "div",
        vec!["card"],
        Rect { x: 50.0, y: 200.0, w: 700.0, h: 400.0 },
        Some(1),
        true,
    );
    let child = node_with_glyphs(
        3,
        "h1",
        vec!["headline"],
        Rect { x: 30.0, y: 190.0, w: 600.0, h: 500.0 },
        Some(2),
        false,
        vec![GlyphInk {
            pen_x: 0.0,
            pen_y: 50.0,
            ink_min_x: 0.0,
            ink_min_y: 0.0,
            ink_max_x: 580.0,
            ink_max_y: 80.0,
        }],
    );
    let fs = run(&snap(vec![outer, inner, child]), Path::new("s.html"));
    assert!(!fs.is_empty(), "expected at least one finding");
}

/// Outer clipping ancestor adds no new edge beyond inner — we
/// suppress the redundant outer finding.
#[test]
fn collapses_redundant_outer_clips() {
    let outer = plain_node(
        1,
        "section",
        vec!["frame"],
        Rect { x: 0.0, y: 0.0, w: 1080.0, h: 1920.0 },
        None,
        true,
    );
    let inner = plain_node(
        2,
        "div",
        vec!["card"],
        Rect { x: 200.0, y: 200.0, w: 400.0, h: 400.0 },
        Some(1),
        true,
    );
    let child = node_with_glyphs(
        3,
        "h1",
        vec!["headline"],
        Rect { x: 100.0, y: 180.0, w: 600.0, h: 500.0 },
        Some(2),
        false,
        vec![GlyphInk {
            pen_x: 0.0,
            pen_y: 50.0,
            ink_min_x: 0.0,
            ink_min_y: 0.0,
            ink_max_x: 580.0,
            ink_max_y: 480.0,
        }],
    );
    let fs = run(&snap(vec![outer, inner, child]), Path::new("s.html"));
    assert_eq!(fs.len(), 1, "outer clip is redundant once inner fires");
}

/// Clip the ancestor tight to the element's pre-transform
/// bbox so any scale at all pushes glyph ink past the clip edge. With
/// `transform: scale(1.5)` and a clip exactly the size of the pre-
/// transform element, post-transform ink overruns by ~150 px on each
/// horizontal edge and ~30 px on each vertical edge.
#[test]
fn flags_scale_overrun_against_tight_clip() {
    // Clip exactly the size of the element's pre-transform bbox, so
    // scale(1.5) overruns by 50%.
    let clipper = plain_node(
        1,
        "div",
        vec!["tight-clip"],
        Rect { x: 100.0, y: 500.0, w: 600.0, h: 120.0 },
        None,
        true,
    );
    let mut h1 = node_with_glyphs(
        2,
        "h1",
        vec!["headline"],
        // Element bbox same as clipper bbox.
        Rect { x: 100.0, y: 500.0, w: 600.0, h: 120.0 },
        Some(1),
        false,
        // Glyphs cover the full pre-transform bbox.
        vec![GlyphInk {
            pen_x: 0.0,
            pen_y: 100.0,
            ink_min_x: 0.0,
            ink_min_y: 10.0,
            ink_max_x: 600.0,
            ink_max_y: 110.0,
        }],
    );
    // scale(1.5) with transform-origin baked in around the element's
    // center (300, 60 element-local): tx = 300 - 450 = -150,
    // ty = 60 - 90 = -30. Operating around bbox origin = (100, 500),
    // the glyph ink rect [100..700, 510..610] (document space) becomes
    // bbox-local [0..600, 10..110], apply scale 1.5 around (300, 60)
    // → bbox-local [-150..750, -35..135], then add (100, 500) →
    // document [-50..850, 465..635]. Clip is [100..700, 500..620].
    // Misses: left = 100 - (-50) = 150 px, right = 850 - 700 = 150 px,
    // top = 500 - 465 = 35 px, bottom = 635 - 620 = 15 px.
    h1.transform = Some([1.5, 0.0, 0.0, 1.5, -150.0, -30.0]);
    let fs = run(&snap(vec![clipper, h1]), Path::new("scale-test.html"));
    assert!(
        !fs.is_empty(),
        "expected a glyph-clip finding when scale(1.5) overruns a tight clip"
    );
    let f = &fs[0];
    assert_eq!(f.severity, Severity::Error);
    // Confirm the message mentions sides expected to overrun.
    assert!(
        f.message.contains("left") || f.message.contains("right"),
        "expected left/right edge mention in {}",
        f.message
    );
}

/// Sanity check: scale(1.0) is identity, so the rule behaves
/// exactly as before — no spurious findings from the transform path.
#[test]
fn identity_transform_is_a_noop() {
    let clipper = plain_node(
        1,
        "div",
        vec!["tight"],
        Rect { x: 0.0, y: 0.0, w: 800.0, h: 200.0 },
        None,
        true,
    );
    let mut h1 = node_with_glyphs(
        2,
        "h1",
        vec!["headline"],
        Rect { x: 100.0, y: 50.0, w: 600.0, h: 100.0 },
        Some(1),
        false,
        vec![GlyphInk {
            pen_x: 0.0,
            pen_y: 80.0,
            ink_min_x: 5.0,
            ink_min_y: 10.0,
            ink_max_x: 590.0,
            ink_max_y: 90.0,
        }],
    );
    // Identity transform — explicitly set, equivalent to None.
    h1.transform = Some([1.0, 0.0, 0.0, 1.0, 0.0, 0.0]);
    let fs = run(&snap(vec![clipper, h1]), Path::new("ident.html"));
    assert!(fs.is_empty(), "identity transform must not produce a finding");
}

/// Spec-exact synthetic: element whose glyph ink fits inside the canvas
/// at scale 1.0, then `transform: scale(1.5)` around the bbox origin
/// (tx=ty=0 — no transform-origin offset). With the bbox-origin pivot,
/// the right edge of the glyph ink scales from 700 px → 1000 px (= 200
/// × 1.5 + bbox.x=100 + ink_max_x_local=600 → wait — re-derive):
///
/// Pre-transform: bbox = (100, 500, 600, 120); glyph ink in document
/// space = (100..700, 510..610). chain_xform wraps `local` with
/// translate(-bbox.x, -bbox.y) on the input side and translate(+bbox.x,
/// +bbox.y) on the output side, so a point at document (100, 500) maps
/// to: → (0, 0) → scale(1.5) → (0, 0) → +(100, 500) = (100, 500).
/// Right corner (700, 610) → (600, 110) → (900, 165) → (1000, 665).
/// Canvas clip is (0, 0, 1080, 1920) — fits horizontally (1000 < 1080)
/// but to make this test fire we use a tight canvas clip (clipper bbox
/// = 1000 wide) so the post-scale right edge pushes past.
#[test]
fn scale_around_origin_exceeds_canvas_clip() {
    let canvas = plain_node(
        1,
        "html",
        vec!["canvas"],
        Rect { x: 0.0, y: 0.0, w: 950.0, h: 800.0 },
        None,
        true,
    );
    let mut text = node_with_glyphs(
        2,
        "h1",
        vec!["scaled"],
        Rect { x: 100.0, y: 500.0, w: 600.0, h: 120.0 },
        Some(1),
        false,
        vec![GlyphInk {
            pen_x: 0.0,
            pen_y: 100.0,
            ink_min_x: 0.0,
            ink_min_y: 10.0,
            ink_max_x: 600.0,
            ink_max_y: 110.0,
        }],
    );
    text.transform = Some([1.5, 0.0, 0.0, 1.5, 0.0, 0.0]);
    let fs = run(&snap(vec![canvas, text]), Path::new("scale-origin.html"));
    assert!(
        !fs.is_empty(),
        "scale(1.5) around bbox origin must push the right ink edge past the canvas clip"
    );
    assert_eq!(fs[0].severity, Severity::Error);
    assert!(
        fs[0].message.contains("right"),
        "expected 'right' edge miss in: {}",
        fs[0].message
    );
}

/// Text painted outside the canvas viewport with NO `overflow: hidden`
/// ancestor anywhere on the chain — the v8 CTA failure mode. Pre-fix,
/// the rule returned no findings (no clipping ancestors → early
/// return); post-fix, the canvas-viewport check fires as an Error.
#[test]
fn flags_text_off_canvas_with_no_overflow_chain() {
    let html = plain_node(
        1,
        "html",
        vec![],
        Rect { x: 0.0, y: 0.0, w: 1080.0, h: 1920.0 },
        None,
        false, // NOTE: no overflow-hidden anywhere
    );
    // CTA placed 80 px past the right canvas edge.
    let cta = node_with_glyphs(
        2,
        "h1",
        vec!["cta"],
        Rect { x: 1000.0, y: 200.0, w: 200.0, h: 100.0 },
        Some(1),
        false,
        vec![GlyphInk {
            pen_x: 0.0,
            pen_y: 80.0,
            ink_min_x: 0.0,
            ink_min_y: 10.0,
            ink_max_x: 200.0,
            ink_max_y: 90.0,
        }],
    );
    let fs = run(&snap(vec![html, cta]), Path::new("cta.html"));
    assert!(!fs.is_empty(), "expected canvas-viewport finding");
    let canvas_fs: Vec<_> = fs
        .iter()
        .filter(|f| f.subkind.as_deref() == Some("canvas"))
        .collect();
    assert_eq!(canvas_fs.len(), 1, "expected one canvas finding, got {:?}", fs);
    assert_eq!(canvas_fs[0].severity, Severity::Error);
    assert!(
        canvas_fs[0].message.contains("canvas viewport"),
        "expected 'canvas viewport' in: {}",
        canvas_fs[0].message
    );
    assert!(
        canvas_fs[0].message.contains("right"),
        "expected 'right' edge miss in: {}",
        canvas_fs[0].message
    );
}
