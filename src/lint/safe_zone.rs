//! Safe-zone rule — flag text-bearing / hero elements whose layout
//! bboxes intersect a platform's chrome danger zones.

use super::report::{LintFinding, Severity};
use super::safe_zones::{danger_zones, SafeZone, SafeZoneTable, DANGER_LABELS};
use crate::query::{FrameSnapshot, NodeSnapshot, Rect};
use std::path::Path;

/// Identifier emitted in `LintFinding.rule`.
pub const RULE: &str = "safe-zone";

const TEXT_BEARING_TAGS: &[&str] = &[
    "h1", "h2", "h3", "h4", "h5", "h6", "p", "span", "div", "button", "a", "li",
];

const HERO_CLASS_HINTS: &[&str] = &["text", "headline", "cta", "caption", "hero", "title"];

/// Run the rule against one scene snapshot. Returns a (possibly empty)
/// vec of findings. The caller scales `zone` from the reference canvas
/// first via `SafeZoneTable::scaled`.
pub fn run(
    snap: &FrameSnapshot,
    scene_path: &Path,
    zone: &SafeZone,
    platform_label: &str,
) -> Vec<LintFinding> {
    let canvas_w = snap.viewport.0 as f32;
    let canvas_h = snap.viewport.1 as f32;
    let zones = danger_zones(zone, canvas_w, canvas_h);

    let mut findings = Vec::new();
    let mut seen: Vec<(usize, usize)> = Vec::new();

    for (idx, node) in snap.nodes.iter().enumerate() {
        if !is_candidate(node) {
            continue;
        }
        if !node.bbox.has_area() {
            continue;
        }
        if node.computed_opacity <= 0.0 {
            continue;
        }
        for (zi, dz) in zones.iter().enumerate() {
            let overlap = intersect(node.bbox, *dz);
            if overlap.has_area() {
                if seen.contains(&(node.id, zi)) {
                    continue;
                }
                seen.push((node.id, zi));
                let severity = severity_for_zone(zi, &overlap);
                let label = DANGER_LABELS[zi];
                findings.push(LintFinding {
                    rule: RULE.to_string(),
                    severity,
                    scene_path: scene_path.to_path_buf(),
                    t_secs: snap.t_secs,
                    element_selector: best_selector(snap, node, idx),
                    element_bbox: node.bbox,
                    message: format!(
                        "overlaps {label} ({} safe-zone, {})",
                        platform_label,
                        edge_describe(zi, zone)
                    ),
                    fix_hint: fix_hint_for_zone(zi, zone),
                });
            }
        }
    }

    findings
}

fn is_candidate(node: &NodeSnapshot) -> bool {
    if !TEXT_BEARING_TAGS.contains(&node.tag.as_str()) {
        return false;
    }
    let has_text = node.text.as_deref().map(|t| !t.is_empty()).unwrap_or(false);
    if has_text {
        return true;
    }
    node.classes
        .iter()
        .any(|c| HERO_CLASS_HINTS.iter().any(|h| c.to_ascii_lowercase().contains(h)))
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

fn severity_for_zone(zone_idx: usize, overlap: &Rect) -> Severity {
    let area = overlap.w * overlap.h;
    if area >= 200.0 {
        Severity::Error
    } else {
        let _ = zone_idx;
        Severity::Warn
    }
}

fn edge_describe(zone_idx: usize, zone: &SafeZone) -> String {
    match zone_idx {
        0 => format!("y < {}", zone.top_px as i32),
        1 => format!("y > canvas_h - {}", zone.bottom_px as i32),
        2 => format!("x < {}", zone.left_px as i32),
        3 => format!("x > canvas_w - {}", zone.right_px as i32),
        _ => String::new(),
    }
}

fn fix_hint_for_zone(zone_idx: usize, zone: &SafeZone) -> String {
    match zone_idx {
        0 => format!(
            "drop the element below y={} px to clear the top chrome strip",
            zone.top_px as i32
        ),
        1 => format!(
            "lift the element above the bottom {} px of the canvas",
            zone.bottom_px as i32
        ),
        2 => format!(
            "push the element right of x={} px to clear the left chrome",
            zone.left_px as i32
        ),
        3 => format!(
            "pull the element left of (canvas_w - {} px) to clear the right chrome",
            zone.right_px as i32
        ),
        _ => String::new(),
    }
}

/// Build a best-effort selector for one node. Order of preference:
/// `#id` → `.first-class` → `tag[index]`.
pub fn best_selector(snap: &FrameSnapshot, node: &NodeSnapshot, idx: usize) -> String {
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

    fn snap_with(nodes: Vec<NodeSnapshot>) -> FrameSnapshot {
        FrameSnapshot {
            t_secs: 1.0,
            frame_index: 30,
            viewport: (1080, 1920),
            active_scene: Some(0),
            nodes,
        }
    }

    fn node(id: usize, tag: &str, classes: Vec<&str>, text: Option<&str>, bbox: Rect) -> NodeSnapshot {
        NodeSnapshot {
            id,
            semantic_id: format!("n{id}"),
            tag: tag.to_string(),
            element_id: None,
            classes: classes.iter().map(|s| s.to_string()).collect(),
            bbox,
            has_own_transform: false,
            clips_overflow: false,
            computed_opacity: 1.0,
            text: text.map(|s| s.to_string()),
            children: vec![],
            parent: None,
        }
    }

    #[test]
    fn flags_bottom_collision() {
        let bad = node(
            1,
            "div",
            vec!["cta-button"],
            Some("Shop now"),
            Rect { x: 440.0, y: 1680.0, w: 200.0, h: 80.0 },
        );
        let snap = snap_with(vec![bad]);
        let zone = SafeZone {
            top_px: 108.0,
            bottom_px: 320.0,
            left_px: 60.0,
            right_px: 120.0,
        };
        let fs = run(&snap, Path::new("scene.html"), &zone, "tiktok");
        assert_eq!(fs.len(), 1);
        assert_eq!(fs[0].severity, Severity::Error);
        assert!(fs[0].message.contains("bottom-chrome"));
    }

    #[test]
    fn ignores_clean_layout() {
        let ok = node(
            1,
            "h1",
            vec!["headline"],
            Some("hi"),
            Rect { x: 200.0, y: 600.0, w: 600.0, h: 120.0 },
        );
        let snap = snap_with(vec![ok]);
        let zone = SafeZone {
            top_px: 108.0,
            bottom_px: 320.0,
            left_px: 60.0,
            right_px: 120.0,
        };
        let fs = run(&snap, Path::new("scene.html"), &zone, "tiktok");
        assert!(fs.is_empty());
    }

    #[test]
    fn skips_non_text_tags() {
        let n = node(
            1,
            "svg",
            vec![],
            Some("blocked"),
            Rect { x: 0.0, y: 0.0, w: 1080.0, h: 100.0 },
        );
        let snap = snap_with(vec![n]);
        let zone = SafeZone {
            top_px: 108.0,
            bottom_px: 320.0,
            left_px: 60.0,
            right_px: 120.0,
        };
        assert!(run(&snap, Path::new("s.html"), &zone, "tiktok").is_empty());
    }
}
