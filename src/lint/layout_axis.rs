//! Layout-axis-coherence rule — flag flex containers whose children
//! render along the WRONG axis vs the declared `flex-direction`.
//!
//! Motivation: in v8 the CTA scene's column-stack of headline + button
//! + footnote collapsed into a horizontal row (likely a stray
//! `flex-direction: row` from inherited styles or an element-default
//! reset). The visual is broken — text overlaps, the button sits next
//! to the headline — but every other rule passes because each element
//! is in-canvas, contrast-legible, and not glyph-clipped.
//!
//! Check: for each node with `flex_axis = Some(axis)`, gather the
//! visible element children. If declared `Column`, the children's Y
//! ranges should be predominantly non-overlapping (stacked vertically);
//! if declared `Row`, their X ranges should be predominantly non-
//! overlapping. We compare median overlap fractions against thresholds
//! tuned to be quiet for one-child containers and decisive for the
//! "all children at the same Y" failure mode.

use super::report::{LintFinding, Severity};
use crate::query::{FlexAxis, FrameSnapshot, NodeSnapshot, Rect};
use std::path::Path;

/// Identifier emitted in `LintFinding.rule`.
pub const RULE: &str = "layout-axis-coherence";

/// Overlap fraction (0.0 .. 1.0) above which we consider two sibling
/// bboxes "in the same band" along an axis. 0.5 = >50% of the smaller
/// child's extent overlaps with another sibling's extent on that axis.
const OVERLAP_BAND_THRESHOLD: f32 = 0.5;

/// Fraction of child pairs that must occupy the same band on the wrong
/// axis before we flag the container. With 3 children that's 2-of-3
/// pairs (66%); with 4 children that's 3-of-6 pairs (50%).
const WRONG_AXIS_PAIR_FRACTION: f32 = 0.5;

/// Skip containers smaller than this — flex layouts with one visible
/// child carry no axis signal, and we don't want to spam findings on
/// degenerate single-item flex wrappers.
const MIN_CHILDREN: usize = 2;

/// Run the rule against one scene snapshot.
pub fn run(snap: &FrameSnapshot, scene_path: &Path) -> Vec<LintFinding> {
    let nodes_by_id = index_by_id(&snap.nodes);
    let mut findings = Vec::new();

    for (idx, container) in snap.nodes.iter().enumerate() {
        let Some(axis) = container.flex_axis else {
            continue;
        };
        if !container.bbox.has_area() {
            continue;
        }
        if container.computed_opacity <= 0.0 {
            continue;
        }

        let children = visible_element_children(container, &snap.nodes, &nodes_by_id);
        if children.len() < MIN_CHILDREN {
            continue;
        }

        // Count sibling pairs whose extents overlap > threshold on the
        // axis perpendicular to the declared one. A "Column" container
        // expects children to span DIFFERENT Y bands — if many pairs
        // share a Y band (overlap > threshold) the layout collapsed
        // onto Row.
        let (total_pairs, wrong_axis_pairs) =
            count_wrong_axis_pairs(&children, axis);
        if total_pairs == 0 {
            continue;
        }
        let frac = wrong_axis_pairs as f32 / total_pairs as f32;
        if frac < WRONG_AXIS_PAIR_FRACTION {
            continue;
        }

        let elem_selector = best_selector(snap, container, idx);
        let declared = match axis {
            FlexAxis::Row => "row",
            FlexAxis::Column => "column",
        };
        let realized = match axis {
            FlexAxis::Row => "column (children stacked vertically)",
            FlexAxis::Column => "row (children side-by-side)",
        };
        let severity = severity_for(frac);
        findings.push(LintFinding {
            rule: RULE.to_string(),
            severity,
            scene_path: scene_path.to_path_buf(),
            t_secs: snap.t_secs,
            element_selector: elem_selector.clone(),
            element_bbox: container.bbox,
            message: format!(
                "flex-direction declared `{declared}` but {}/{} child pairs \
                 overlap on the {declared}-perpendicular axis — layout \
                 realized as {realized}",
                wrong_axis_pairs, total_pairs
            ),
            fix_hint: format!(
                "verify `flex-direction: {declared}` survives the cascade \
                 on {elem_selector} (no inline override, no shorthand \
                 reset, no inherited `flex-direction: {}` clobbering it)",
                opposite(axis),
            ),
            subkind: None,
        });
    }

    findings
}

/// Pairs counted as (total visible-child pairs, pairs that occupy the
/// same band on the axis perpendicular to `declared`).
fn count_wrong_axis_pairs(children: &[Rect], declared: FlexAxis) -> (usize, usize) {
    let mut total = 0usize;
    let mut wrong = 0usize;
    for i in 0..children.len() {
        for j in (i + 1)..children.len() {
            total += 1;
            let a = children[i];
            let b = children[j];
            let overlap_frac = match declared {
                // Column declared → check Y-band overlap (wrong axis).
                FlexAxis::Column => axis_overlap_fraction(a.y, a.h, b.y, b.h),
                // Row declared → check X-band overlap (wrong axis).
                FlexAxis::Row => axis_overlap_fraction(a.x, a.w, b.x, b.w),
            };
            if overlap_frac > OVERLAP_BAND_THRESHOLD {
                wrong += 1;
            }
        }
    }
    (total, wrong)
}

/// Fraction of the smaller range that overlaps with the other range
/// along one axis. 0.0 = disjoint, 1.0 = smaller is fully contained in
/// the larger. We divide by the smaller extent so a tall flex parent
/// containing a short child still scores correctly.
fn axis_overlap_fraction(a_start: f32, a_len: f32, b_start: f32, b_len: f32) -> f32 {
    if a_len <= 0.0 || b_len <= 0.0 {
        return 0.0;
    }
    let lo = a_start.max(b_start);
    let hi = (a_start + a_len).min(b_start + b_len);
    let overlap = (hi - lo).max(0.0);
    let smaller = a_len.min(b_len);
    if smaller <= 0.0 {
        0.0
    } else {
        overlap / smaller
    }
}

fn severity_for(frac: f32) -> Severity {
    // Strong signal — every pair on the wrong axis is almost certainly
    // a layout bug, not deliberate. Allow mid-range to land at Warn for
    // CTA-style three-element columns where one item is shorter.
    if frac >= 0.9 {
        Severity::Error
    } else if frac >= WRONG_AXIS_PAIR_FRACTION {
        Severity::Warn
    } else {
        Severity::Info
    }
}

fn opposite(axis: FlexAxis) -> &'static str {
    match axis {
        FlexAxis::Row => "column",
        FlexAxis::Column => "row",
    }
}

fn visible_element_children(
    container: &NodeSnapshot,
    nodes: &[NodeSnapshot],
    map: &[(usize, usize)],
) -> Vec<Rect> {
    let mut out = Vec::new();
    for &cid in &container.children {
        let Some(c_idx) = lookup(map, cid) else { continue };
        let c = &nodes[c_idx];
        if !c.bbox.has_area() {
            continue;
        }
        if c.computed_opacity <= 0.0 {
            continue;
        }
        out.push(c.bbox);
    }
    out
}

fn index_by_id(nodes: &[NodeSnapshot]) -> Vec<(usize, usize)> {
    let mut v: Vec<(usize, usize)> = nodes.iter().enumerate().map(|(i, n)| (n.id, i)).collect();
    v.sort_by_key(|p| p.0);
    v
}

fn lookup(map: &[(usize, usize)], node_id: usize) -> Option<usize> {
    map.binary_search_by_key(&node_id, |p| p.0).ok().map(|i| map[i].1)
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
    use crate::query::{FlexAxis, FrameSnapshot, NodeSnapshot, Rect};

    fn container(
        id: usize,
        children: Vec<usize>,
        bbox: Rect,
        axis: FlexAxis,
    ) -> NodeSnapshot {
        NodeSnapshot {
            id,
            semantic_id: format!("n{id}"),
            tag: "div".into(),
            element_id: Some(format!("cta-{id}")),
            classes: vec!["cta".into()],
            bbox,
            transform: None,
            clips_overflow: false,
            computed_opacity: 1.0,
            computed_font_size_px: 0.0,
            text: None,
            children,
            parent: None,
            glyph_run: None,
            flex_axis: Some(axis),
        }
    }

    fn child(id: usize, parent: usize, bbox: Rect) -> NodeSnapshot {
        NodeSnapshot {
            id,
            semantic_id: format!("n{id}"),
            tag: "span".into(),
            element_id: None,
            classes: vec![],
            bbox,
            transform: None,
            clips_overflow: false,
            computed_opacity: 1.0,
            computed_font_size_px: 36.0,
            text: Some("x".into()),
            children: vec![],
            parent: Some(parent),
            glyph_run: None,
            flex_axis: None,
        }
    }

    fn snap(nodes: Vec<NodeSnapshot>) -> FrameSnapshot {
        FrameSnapshot {
            t_secs: 1.0,
            frame_index: 30,
            viewport: (1080, 1920),
            active_scene: Some(0),
            nodes,
        }
    }

    #[test]
    fn column_with_actual_column_passes() {
        let c = container(
            1,
            vec![2, 3, 4],
            Rect { x: 100.0, y: 200.0, w: 800.0, h: 600.0 },
            FlexAxis::Column,
        );
        let a = child(2, 1, Rect { x: 100.0, y: 200.0, w: 800.0, h: 120.0 });
        let b = child(3, 1, Rect { x: 100.0, y: 360.0, w: 800.0, h: 120.0 });
        let d = child(4, 1, Rect { x: 100.0, y: 520.0, w: 800.0, h: 120.0 });
        let s = snap(vec![c, a, b, d]);
        let f = run(&s, Path::new("scene.html"));
        assert!(f.is_empty(), "well-stacked column should not flag, got {:?}", f);
    }

    #[test]
    fn column_collapsed_to_row_flags() {
        // Container declares column, but children sit side-by-side at
        // the same Y — the v8 CTA failure mode.
        let c = container(
            1,
            vec![2, 3, 4],
            Rect { x: 100.0, y: 800.0, w: 880.0, h: 200.0 },
            FlexAxis::Column,
        );
        let a = child(2, 1, Rect { x: 100.0, y: 800.0, w: 280.0, h: 200.0 });
        let b = child(3, 1, Rect { x: 400.0, y: 800.0, w: 280.0, h: 200.0 });
        let d = child(4, 1, Rect { x: 700.0, y: 800.0, w: 280.0, h: 200.0 });
        let s = snap(vec![c, a, b, d]);
        let f = run(&s, Path::new("scene.html"));
        assert_eq!(f.len(), 1, "expected one finding, got {:?}", f);
        assert_eq!(f[0].rule, RULE);
        assert!(matches!(f[0].severity, Severity::Error));
        assert!(f[0].message.contains("declared `column`"));
        assert!(f[0].message.contains("realized as row"));
    }

    #[test]
    fn row_collapsed_to_column_flags() {
        // Reciprocal: declared row, but children stacked vertically.
        let c = container(
            1,
            vec![2, 3, 4],
            Rect { x: 100.0, y: 200.0, w: 800.0, h: 600.0 },
            FlexAxis::Row,
        );
        let a = child(2, 1, Rect { x: 100.0, y: 200.0, w: 800.0, h: 120.0 });
        let b = child(3, 1, Rect { x: 100.0, y: 360.0, w: 800.0, h: 120.0 });
        let d = child(4, 1, Rect { x: 100.0, y: 520.0, w: 800.0, h: 120.0 });
        let s = snap(vec![c, a, b, d]);
        let f = run(&s, Path::new("scene.html"));
        assert_eq!(f.len(), 1);
        assert!(f[0].message.contains("declared `row`"));
    }

    #[test]
    fn single_child_does_not_flag() {
        let c = container(
            1,
            vec![2],
            Rect { x: 100.0, y: 200.0, w: 800.0, h: 200.0 },
            FlexAxis::Column,
        );
        let a = child(2, 1, Rect { x: 100.0, y: 200.0, w: 800.0, h: 200.0 });
        let s = snap(vec![c, a]);
        let f = run(&s, Path::new("scene.html"));
        assert!(f.is_empty());
    }

    #[test]
    fn non_flex_container_skipped() {
        // No flex_axis = not a flex container = nothing to check.
        let mut c = container(
            1,
            vec![2, 3],
            Rect { x: 100.0, y: 200.0, w: 800.0, h: 200.0 },
            FlexAxis::Column,
        );
        c.flex_axis = None;
        let a = child(2, 1, Rect { x: 100.0, y: 200.0, w: 380.0, h: 200.0 });
        let b = child(3, 1, Rect { x: 500.0, y: 200.0, w: 380.0, h: 200.0 });
        let s = snap(vec![c, a, b]);
        let f = run(&s, Path::new("scene.html"));
        assert!(f.is_empty());
    }
}
