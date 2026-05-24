//! Text-readability rule — flag text elements rendered too small for
//! mobile in-feed viewing. Pure layout walk; checks computed font-size
//! against a per-aspect cap-height floor. No pixel inspection (the
//! contrast pass is deferred — see RENDER_LINT_SYSTEM.md §1.3).

use super::report::{LintFinding, Severity};
use crate::query::{FrameSnapshot, NodeSnapshot};
use std::path::Path;

/// Identifier emitted in `LintFinding.rule`.
pub const RULE: &str = "text-readability";

const TEXT_BEARING_TAGS: &[&str] = &[
    "h1", "h2", "h3", "h4", "h5", "h6", "p", "span", "div", "button", "a", "li",
];

const HERO_CLASS_HINTS: &[&str] = &[
    "text", "headline", "cta", "caption", "hero", "title", "num", "label", "copy",
];

/// Cap-height as a fraction of font-size. Per-font-family in reality
/// (sans-serifs land 0.65–0.75); 0.7 is a safe middle that doesn't
/// over- or under-flag the common authoring fonts.
const CAP_HEIGHT_RATIO: f32 = 0.7;

/// Reference canvas the per-aspect minimums are stated against; all
/// thresholds scale linearly with the actual canvas height.
const REFERENCE_CANVAS_H: f32 = 1920.0;

/// Per-aspect minimum cap-height in CSS px on a 1080×1920 canvas. The
/// 9:16 floor is the platform-published autoplay-muted feed minimum
/// (~36 pt). 16:9 is the lower desktop-distance floor. Square / 4:5
/// sit between the two.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AspectClass {
    /// Tall canvases — TikTok / Reels / Shorts (9:16). Strictest floor.
    Vertical,
    /// Wide canvases — YouTube landscape, broadcast (16:9). Lowest floor.
    Widescreen,
    /// Square or 4:5 in-feed crops. Mid-range floor.
    Squareish,
}

impl AspectClass {
    fn min_cap_height_px_at_reference(self) -> f32 {
        match self {
            AspectClass::Vertical => 56.0,
            AspectClass::Widescreen => 32.0,
            AspectClass::Squareish => 44.0,
        }
    }

    fn label(self) -> &'static str {
        match self {
            AspectClass::Vertical => "9:16",
            AspectClass::Widescreen => "16:9",
            AspectClass::Squareish => "1:1 / 4:5",
        }
    }
}

/// Pick the aspect class from an explicit override or from the canvas
/// dimensions. The override matches the CLI surface (`--aspect`).
pub fn classify_aspect(explicit: Option<&str>, canvas_w: u32, canvas_h: u32) -> AspectClass {
    if let Some(a) = explicit {
        match a.trim() {
            "9:16" => return AspectClass::Vertical,
            "16:9" => return AspectClass::Widescreen,
            "1:1" | "4:5" => return AspectClass::Squareish,
            _ => {}
        }
    }
    if canvas_h == 0 {
        return AspectClass::Vertical;
    }
    let ratio = canvas_w as f32 / canvas_h as f32;
    if ratio < 0.7 {
        AspectClass::Vertical
    } else if ratio > 1.5 {
        AspectClass::Widescreen
    } else {
        AspectClass::Squareish
    }
}

/// Run the rule against one scene snapshot.
pub fn run(
    snap: &FrameSnapshot,
    scene_path: &Path,
    aspect: AspectClass,
) -> Vec<LintFinding> {
    let canvas_h = snap.viewport.1 as f32;
    if canvas_h <= 0.0 {
        return Vec::new();
    }
    let min_cap = scaled_min_cap_height(aspect, canvas_h);

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
        if node.computed_font_size_px <= 0.0 {
            continue;
        }
        let cap_height = node.computed_font_size_px * CAP_HEIGHT_RATIO;
        if cap_height >= min_cap {
            continue;
        }
        if seen.contains(&node.id) {
            continue;
        }
        seen.push(node.id);

        let severity = if cap_height < min_cap * 0.5 {
            Severity::Error
        } else {
            Severity::Warn
        };
        let target_font_size = (min_cap / CAP_HEIGHT_RATIO).ceil();
        let selector = best_selector(snap, node, idx);
        let canvas_label = format!(
            "{} for {} @ canvas {}x{}",
            fmt_px(min_cap),
            aspect.label(),
            snap.viewport.0,
            snap.viewport.1,
        );
        findings.push(LintFinding {
            rule: RULE.to_string(),
            severity,
            scene_path: scene_path.to_path_buf(),
            t_secs: snap.t_secs,
            element_selector: selector,
            element_bbox: node.bbox,
            message: format!(
                "cap-height {} (min {}); font-size {}",
                fmt_px(cap_height),
                canvas_label,
                fmt_px(node.computed_font_size_px),
            ),
            fix_hint: build_fix_hint(severity, target_font_size, aspect),
            subkind: Some(SUBKIND_CAP_HEIGHT.to_string()),
        });
    }

    findings
}

/// Subkind label written into `LintFinding.subkind` for cap-height findings.
pub const SUBKIND_CAP_HEIGHT: &str = "cap-height";

/// Subkind label written into `LintFinding.subkind` for contrast findings.
pub const SUBKIND_CONTRAST: &str = "contrast";

fn scaled_min_cap_height(aspect: AspectClass, canvas_h: f32) -> f32 {
    aspect.min_cap_height_px_at_reference() * (canvas_h / REFERENCE_CANVAS_H)
}

pub(crate) fn is_text_candidate(node: &NodeSnapshot) -> bool {
    if !TEXT_BEARING_TAGS.contains(&node.tag.as_str()) {
        return false;
    }
    let has_text = node.text.as_deref().map(|t| !t.is_empty()).unwrap_or(false);
    if has_text {
        return true;
    }
    node.classes.iter().any(|c| {
        let lc = c.to_ascii_lowercase();
        HERO_CLASS_HINTS.iter().any(|h| lc.contains(h))
    })
}

fn fmt_px(v: f32) -> String {
    format!("{} px", v.round() as i32)
}

fn build_fix_hint(severity: Severity, target_font_size: f32, aspect: AspectClass) -> String {
    let target = target_font_size.round() as i32;
    match severity {
        Severity::Error => format!(
            "this is < 50% of the readability floor — at this size the text \
             is unreadable on a {} feed thumbnail. Bump font-size to >= {} px \
             or remove the element entirely.",
            aspect.label(),
            target,
        ),
        _ => format!(
            "raise font-size to >= {} px to clear the {} cap-height floor for \
             {}. Check the parent container can hold it.",
            target,
            fmt_px(aspect.min_cap_height_px_at_reference()),
            aspect.label(),
        ),
    }
}

/// Build a best-effort selector for one node. Mirrors the other lint
/// rules: `#id` → `.first-class` → `tag[index]`.
pub(crate) fn best_selector(snap: &FrameSnapshot, node: &NodeSnapshot, idx: usize) -> String {
    if let Some(id) = &node.element_id {
        return format!("#{id}");
    }
    if let Some(c) = node.classes.first() {
        return format!(".{c}");
    }
    let same_tag = snap.nodes.iter().take(idx + 1).filter(|n| n.tag == node.tag).count();
    format!("{}[{}]", node.tag, same_tag.saturating_sub(1))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::query::{FrameSnapshot, NodeSnapshot, Rect};

    fn snap(nodes: Vec<NodeSnapshot>, viewport: (u32, u32)) -> FrameSnapshot {
        FrameSnapshot {
            t_secs: 1.0,
            frame_index: 30,
            viewport,
            active_scene: Some(0),
            nodes,
        }
    }

    fn n(
        id: usize,
        tag: &str,
        classes: Vec<&str>,
        text: Option<&str>,
        bbox: Rect,
        font_size_px: f32,
    ) -> NodeSnapshot {
        NodeSnapshot {
            id,
            semantic_id: format!("n{id}"),
            tag: tag.to_string(),
            element_id: None,
            classes: classes.iter().map(|s| s.to_string()).collect(),
            bbox,
            transform: None,
            clips_overflow: false,
            computed_opacity: 1.0,
            computed_font_size_px: font_size_px,
            text: text.map(|s| s.to_string()),
            children: vec![],
            parent: None,
            glyph_run: None,
            flex_axis: None,
        }
    }

    #[test]
    fn warn_when_below_floor_but_above_half() {
        let node = n(
            1,
            "p",
            vec!["ingredient-label"],
            Some("Whole milk"),
            Rect { x: 200.0, y: 400.0, w: 400.0, h: 60.0 },
            // cap = 40 * 0.7 = 28px, floor = 56 → above 50% of 56 (28 == 28, so warn).
            40.0,
        );
        let fs = run(&snap(vec![node], (1080, 1920)), Path::new("s.html"), AspectClass::Vertical);
        assert_eq!(fs.len(), 1);
        assert_eq!(fs[0].severity, Severity::Warn);
        assert!(fs[0].message.contains("cap-height 28 px"));
        assert!(fs[0].message.contains("min 56 px"));
        assert!(fs[0].fix_hint.contains(">= 80 px"));
    }

    #[test]
    fn error_when_below_half_floor() {
        let node = n(
            1,
            "p",
            vec!["recipe-note"],
            Some("Use chilled butter"),
            Rect { x: 200.0, y: 400.0, w: 400.0, h: 30.0 },
            20.0, // cap = 14, less than 28 (half of 56)
        );
        let fs = run(&snap(vec![node], (1080, 1920)), Path::new("s.html"), AspectClass::Vertical);
        assert_eq!(fs.len(), 1);
        assert_eq!(fs[0].severity, Severity::Error);
        assert!(fs[0].fix_hint.contains("unreadable"));
    }

    #[test]
    fn passes_when_above_floor() {
        let node = n(
            1,
            "h1",
            vec!["headline"],
            Some("TEN"),
            Rect { x: 100.0, y: 400.0, w: 800.0, h: 200.0 },
            120.0, // cap = 84 px, well above 56
        );
        let fs = run(&snap(vec![node], (1080, 1920)), Path::new("s.html"), AspectClass::Vertical);
        assert!(fs.is_empty());
    }

    #[test]
    fn widescreen_uses_lower_floor() {
        // 24 px font → cap 16.8. Widescreen floor = 32, so 16.8 < 16 → error.
        // Actually 16.8 > 16 (half of 32), so warn.
        let node = n(
            1,
            "span",
            vec!["caption"],
            Some("Cinematic"),
            Rect { x: 100.0, y: 400.0, w: 300.0, h: 40.0 },
            24.0,
        );
        let fs = run(&snap(vec![node], (1920, 1080)), Path::new("s.html"), AspectClass::Widescreen);
        assert_eq!(fs.len(), 1);
        assert_eq!(fs[0].severity, Severity::Warn);
        assert!(fs[0].message.contains("16:9"));
    }

    #[test]
    fn scales_with_canvas_height() {
        // Smaller canvas (540×960 = 1080×1920 / 2) → floor halves to 28.
        // 30 px font → cap = 21, less than 28 but more than 14 (half) → warn.
        let node = n(
            1,
            "p",
            vec!["copy"],
            Some("note"),
            Rect { x: 50.0, y: 200.0, w: 200.0, h: 40.0 },
            30.0,
        );
        let fs = run(&snap(vec![node], (540, 960)), Path::new("s.html"), AspectClass::Vertical);
        assert_eq!(fs.len(), 1);
        assert_eq!(fs[0].severity, Severity::Warn);
        // Threshold scaled to 28 px.
        assert!(fs[0].message.contains("min 28 px"));
    }

    #[test]
    fn skips_zero_opacity() {
        let mut node = n(
            1,
            "p",
            vec!["copy"],
            Some("invisible"),
            Rect { x: 100.0, y: 400.0, w: 200.0, h: 20.0 },
            10.0,
        );
        node.computed_opacity = 0.0;
        let fs = run(&snap(vec![node], (1080, 1920)), Path::new("s.html"), AspectClass::Vertical);
        assert!(fs.is_empty());
    }

    #[test]
    fn skips_zero_area_bbox() {
        let node = n(
            1,
            "p",
            vec!["copy"],
            Some("collapsed"),
            Rect { x: 100.0, y: 400.0, w: 0.0, h: 20.0 },
            10.0,
        );
        let fs = run(&snap(vec![node], (1080, 1920)), Path::new("s.html"), AspectClass::Vertical);
        assert!(fs.is_empty());
    }

    #[test]
    fn skips_zero_font_size() {
        // No cascade information — treat as not-applicable.
        let node = n(
            1,
            "p",
            vec!["copy"],
            Some("uncascaded"),
            Rect { x: 100.0, y: 400.0, w: 200.0, h: 20.0 },
            0.0,
        );
        let fs = run(&snap(vec![node], (1080, 1920)), Path::new("s.html"), AspectClass::Vertical);
        assert!(fs.is_empty());
    }

    #[test]
    fn classify_aspect_explicit_wins() {
        assert_eq!(classify_aspect(Some("9:16"), 1920, 1080), AspectClass::Vertical);
        assert_eq!(classify_aspect(Some("16:9"), 1080, 1920), AspectClass::Widescreen);
        assert_eq!(classify_aspect(Some("1:1"), 1920, 1080), AspectClass::Squareish);
        assert_eq!(classify_aspect(Some("4:5"), 1920, 1080), AspectClass::Squareish);
    }

    #[test]
    fn classify_aspect_infers_from_canvas() {
        assert_eq!(classify_aspect(None, 1080, 1920), AspectClass::Vertical);
        assert_eq!(classify_aspect(None, 1920, 1080), AspectClass::Widescreen);
        assert_eq!(classify_aspect(None, 1080, 1080), AspectClass::Squareish);
        assert_eq!(classify_aspect(None, 1080, 1350), AspectClass::Squareish);
    }
}
