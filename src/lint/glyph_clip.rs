//! Glyph-clip rule — flag text-bearing elements whose layout bbox is
//! cropped by an ancestor's `overflow: hidden|clip` / `clip-path` /
//! `mask-image`. Pure layout walk; no pixel inspection.

use super::report::{LintFinding, Severity};
use crate::query::{FrameSnapshot, NodeSnapshot, Rect};
use std::path::Path;

/// Identifier emitted in `LintFinding.rule`.
pub const RULE: &str = "glyph-clip";

const TEXT_BEARING_TAGS: &[&str] = &[
    "h1", "h2", "h3", "h4", "h5", "h6", "p", "span", "div", "button", "a", "li",
];

const HERO_CLASS_HINTS: &[&str] = &["text", "headline", "cta", "caption", "hero", "title", "num"];

const ERROR_PX: f32 = 8.0;
const WARN_PX: f32 = 2.0;

#[derive(Debug, Clone, Copy)]
struct EdgeMiss {
    top: f32,
    bottom: f32,
    left: f32,
    right: f32,
}

impl EdgeMiss {
    fn max(&self) -> f32 {
        self.top.max(self.bottom).max(self.left).max(self.right)
    }
    fn any(&self) -> bool {
        self.top > 0.0 || self.bottom > 0.0 || self.left > 0.0 || self.right > 0.0
    }
}

/// Run the rule against one scene snapshot.
pub fn run(snap: &FrameSnapshot, scene_path: &Path) -> Vec<LintFinding> {
    let nodes_by_id = index_by_id(&snap.nodes);
    let mut findings = Vec::new();
    let mut seen: Vec<(usize, usize)> = Vec::new();

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

        let clippers = collect_clipping_ancestors(node, &snap.nodes, &nodes_by_id);
        if clippers.is_empty() {
            continue;
        }

        let mut accum = unbounded_rect();
        let mut last_miss = EdgeMiss { top: 0.0, bottom: 0.0, left: 0.0, right: 0.0 };
        for &anc_idx in &clippers {
            let anc = &snap.nodes[anc_idx];
            accum = intersect(accum, anc.bbox);
            let miss = edge_miss(node.bbox, accum);
            if !miss.any() {
                continue;
            }
            if !adds_new_edge(&miss, &last_miss) {
                continue;
            }
            last_miss = miss;
            if seen.contains(&(node.id, anc.id)) {
                continue;
            }
            seen.push((node.id, anc.id));

            let severity = severity_for(miss.max());
            let anc_selector = best_selector(snap, anc, anc_idx);
            let elem_selector = best_selector(snap, node, idx);
            findings.push(LintFinding {
                rule: RULE.to_string(),
                severity,
                scene_path: scene_path.to_path_buf(),
                t_secs: snap.t_secs,
                element_selector: elem_selector,
                element_bbox: node.bbox,
                message: format!(
                    "clipped by {} (clips its descendants, bbox {}); missing {}",
                    anc_selector,
                    fmt_rect(anc.bbox),
                    describe_miss(&miss)
                ),
                fix_hint: build_fix_hint(&miss, &anc_selector),
            });
        }
    }

    findings
}

fn is_text_candidate(node: &NodeSnapshot) -> bool {
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

fn index_by_id(nodes: &[NodeSnapshot]) -> Vec<(usize, usize)> {
    let mut v: Vec<(usize, usize)> = nodes.iter().enumerate().map(|(i, n)| (n.id, i)).collect();
    v.sort_by_key(|p| p.0);
    v
}

fn lookup(map: &[(usize, usize)], node_id: usize) -> Option<usize> {
    map.binary_search_by_key(&node_id, |p| p.0).ok().map(|i| map[i].1)
}

fn collect_clipping_ancestors(
    node: &NodeSnapshot,
    nodes: &[NodeSnapshot],
    map: &[(usize, usize)],
) -> Vec<usize> {
    let mut out = Vec::new();
    let mut cursor = node.parent;
    while let Some(parent_id) = cursor {
        let Some(parent_idx) = lookup(map, parent_id) else {
            break;
        };
        let parent = &nodes[parent_idx];
        if parent.clips_overflow {
            out.push(parent_idx);
        }
        cursor = parent.parent;
    }
    out
}

fn unbounded_rect() -> Rect {
    Rect {
        x: f32::MIN / 4.0,
        y: f32::MIN / 4.0,
        w: f32::MAX / 2.0,
        h: f32::MAX / 2.0,
    }
}

fn intersect(a: Rect, b: Rect) -> Rect {
    let x0 = a.x.max(b.x);
    let y0 = a.y.max(b.y);
    let x1 = (a.x + a.w).min(b.x + b.w);
    let y1 = (a.y + a.h).min(b.y + b.h);
    Rect {
        x: x0,
        y: y0,
        w: (x1 - x0).max(0.0),
        h: (y1 - y0).max(0.0),
    }
}

fn edge_miss(child: Rect, clip: Rect) -> EdgeMiss {
    EdgeMiss {
        top: (clip.y - child.y).max(0.0),
        bottom: ((child.y + child.h) - (clip.y + clip.h)).max(0.0),
        left: (clip.x - child.x).max(0.0),
        right: ((child.x + child.w) - (clip.x + clip.w)).max(0.0),
    }
}

/// True if `next` reports a miss on at least one edge that `prev` did
/// not. Used to suppress reporting a chain of outer ancestors that all
/// produce the exact same clip rectangle (the innermost one already
/// describes the problem).
fn adds_new_edge(next: &EdgeMiss, prev: &EdgeMiss) -> bool {
    next.top > prev.top + 0.5
        || next.bottom > prev.bottom + 0.5
        || next.left > prev.left + 0.5
        || next.right > prev.right + 0.5
}

fn severity_for(max_miss_px: f32) -> Severity {
    if max_miss_px > ERROR_PX {
        Severity::Error
    } else if max_miss_px > WARN_PX {
        Severity::Warn
    } else {
        Severity::Info
    }
}

fn describe_miss(miss: &EdgeMiss) -> String {
    let mut parts: Vec<String> = Vec::new();
    if miss.top > 0.0 {
        parts.push(format!("top {} px", miss.top.round() as i32));
    }
    if miss.right > 0.0 {
        parts.push(format!("right {} px", miss.right.round() as i32));
    }
    if miss.bottom > 0.0 {
        parts.push(format!("bottom {} px", miss.bottom.round() as i32));
    }
    if miss.left > 0.0 {
        parts.push(format!("left {} px", miss.left.round() as i32));
    }
    parts.join(", ")
}

fn build_fix_hint(miss: &EdgeMiss, anc_selector: &str) -> String {
    let mut sides: Vec<&str> = Vec::new();
    if miss.top > 0.0 {
        sides.push("padding-top");
    }
    if miss.right > 0.0 {
        sides.push("padding-right");
    }
    if miss.bottom > 0.0 {
        sides.push("padding-bottom");
    }
    if miss.left > 0.0 {
        sides.push("padding-left");
    }
    let padding_list = if sides.is_empty() {
        "padding".to_string()
    } else {
        sides.join(" + ")
    };
    format!(
        "increase {anc_selector}'s {padding_list}, OR remove the clip \
         (overflow:hidden/clip-path/mask-image) if it wasn't intentional, \
         OR shrink the child element to fit"
    )
}

fn fmt_rect(r: Rect) -> String {
    format!(
        "x={} y={} w={} h={}",
        r.x.round() as i32,
        r.y.round() as i32,
        r.w.round() as i32,
        r.h.round() as i32,
    )
}

fn best_selector(snap: &FrameSnapshot, node: &NodeSnapshot, idx: usize) -> String {
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

    fn snap(nodes: Vec<NodeSnapshot>) -> FrameSnapshot {
        FrameSnapshot {
            t_secs: 1.0,
            frame_index: 30,
            viewport: (1080, 1920),
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
            has_own_transform: false,
            clips_overflow: clips,
            computed_opacity: 1.0,
            text: text.map(|s| s.to_string()),
            children: vec![],
            parent,
        }
    }

    #[test]
    fn flags_top_clip_as_warn() {
        let parent = n(
            1,
            "div",
            vec!["headline"],
            None,
            Rect { x: 100.0, y: 470.0, w: 420.0, h: 170.0 },
            None,
            true,
        );
        let child = n(
            2,
            "div",
            vec!["num"],
            Some("10"),
            Rect { x: 120.0, y: 466.0, w: 380.0, h: 160.0 },
            Some(1),
            false,
        );
        let fs = run(&snap(vec![parent, child]), Path::new("s.html"));
        assert_eq!(fs.len(), 1);
        assert_eq!(fs[0].severity, Severity::Warn);
        assert!(fs[0].message.contains("top 4 px"));
        assert!(fs[0].message.contains(".headline"));
    }

    #[test]
    fn error_when_miss_exceeds_8px() {
        let parent = n(
            1,
            "div",
            vec!["box"],
            None,
            Rect { x: 100.0, y: 500.0, w: 400.0, h: 200.0 },
            None,
            true,
        );
        let child = n(
            2,
            "div",
            vec!["headline"],
            Some("hello"),
            Rect { x: 120.0, y: 480.0, w: 400.0, h: 160.0 },
            Some(1),
            false,
        );
        let fs = run(&snap(vec![parent, child]), Path::new("s.html"));
        assert_eq!(fs.len(), 1);
        assert_eq!(fs[0].severity, Severity::Error);
        assert!(fs[0].message.contains("top 20 px"));
        assert!(fs[0].message.contains("right 20 px"));
    }

    #[test]
    fn ignores_when_child_fits() {
        let parent = n(
            1,
            "div",
            vec!["box"],
            None,
            Rect { x: 100.0, y: 470.0, w: 420.0, h: 200.0 },
            None,
            true,
        );
        let child = n(
            2,
            "div",
            vec!["headline"],
            Some("hi"),
            Rect { x: 120.0, y: 490.0, w: 380.0, h: 160.0 },
            Some(1),
            false,
        );
        assert!(run(&snap(vec![parent, child]), Path::new("s.html")).is_empty());
    }

    #[test]
    fn skips_non_clipping_ancestors() {
        let outer = n(
            1,
            "div",
            vec!["page"],
            None,
            Rect { x: 0.0, y: 0.0, w: 1080.0, h: 1920.0 },
            None,
            false,
        );
        let child = n(
            2,
            "div",
            vec!["headline"],
            Some("hi"),
            Rect { x: 120.0, y: 490.0, w: 380.0, h: 160.0 },
            Some(1),
            false,
        );
        assert!(run(&snap(vec![outer, child]), Path::new("s.html")).is_empty());
    }

    #[test]
    fn info_severity_for_subpixel_miss() {
        let parent = n(
            1,
            "div",
            vec!["box"],
            None,
            Rect { x: 100.0, y: 500.0, w: 400.0, h: 200.0 },
            None,
            true,
        );
        let child = n(
            2,
            "div",
            vec!["headline"],
            Some("hi"),
            Rect { x: 100.0, y: 499.0, w: 400.0, h: 200.0 },
            Some(1),
            false,
        );
        let fs = run(&snap(vec![parent, child]), Path::new("s.html"));
        assert_eq!(fs.len(), 1);
        assert_eq!(fs[0].severity, Severity::Info);
    }

    #[test]
    fn reports_inner_and_outer_clip_separately_when_edges_differ() {
        let outer = n(
            1,
            "section",
            vec!["frame"],
            None,
            Rect { x: 0.0, y: 0.0, w: 600.0, h: 1000.0 },
            None,
            true,
        );
        let inner = n(
            2,
            "div",
            vec!["card"],
            None,
            Rect { x: 100.0, y: 100.0, w: 700.0, h: 400.0 },
            Some(1),
            true,
        );
        let child = n(
            3,
            "div",
            vec!["headline"],
            Some("x"),
            Rect { x: 80.0, y: 80.0, w: 700.0, h: 500.0 },
            Some(2),
            false,
        );
        let fs = run(&snap(vec![outer, inner, child]), Path::new("s.html"));
        assert_eq!(fs.len(), 2);
    }

    #[test]
    fn collapses_redundant_outer_clips() {
        let outer = n(
            1,
            "section",
            vec!["frame"],
            None,
            Rect { x: 0.0, y: 0.0, w: 1080.0, h: 1920.0 },
            None,
            true,
        );
        let inner = n(
            2,
            "div",
            vec!["card"],
            None,
            Rect { x: 200.0, y: 200.0, w: 400.0, h: 400.0 },
            Some(1),
            true,
        );
        let child = n(
            3,
            "div",
            vec!["headline"],
            Some("x"),
            Rect { x: 100.0, y: 180.0, w: 600.0, h: 500.0 },
            Some(2),
            false,
        );
        let fs = run(&snap(vec![outer, inner, child]), Path::new("s.html"));
        assert_eq!(fs.len(), 1);
    }
}
