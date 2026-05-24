//! Glyph-clip rule — flag text whose actual painted glyph ink would
//! be clipped by an ancestor's `overflow: hidden|clip` / `clip-path` /
//! `mask-image`.
//!
//! Fact-checks from the visualization, not from layout. For every
//! text-bearing element with Parley `inline_layout_data`, we query
//! skrifa for each positioned glyph's ink bbox (post-shaping,
//! post-kerning, including italic side-bearings and ascender /
//! descender overrun), then ask whether that ink rect is fully
//! contained in the running clip rect built from the element's
//! clipping ancestors.
//!
//! This is strictly stronger than the previous layout-bbox check:
//! cases where the element's box fits but the italic lean / final
//! flourish / descender bowl paints past the clip edge get caught.

use super::report::{LintFinding, Severity};
use crate::query::{FrameSnapshot, GlyphInk, NodeSnapshot, Rect};
use kurbo::Affine;
use std::path::Path;

/// Identifier emitted in `LintFinding.rule`.
pub const RULE: &str = "glyph-clip";

const ERROR_PX: f32 = 8.0;
const WARN_PX: f32 = 2.0;

#[derive(Debug, Clone, Copy, Default)]
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
    fn fold(&mut self, other: &EdgeMiss) {
        self.top = self.top.max(other.top);
        self.bottom = self.bottom.max(other.bottom);
        self.left = self.left.max(other.left);
        self.right = self.right.max(other.right);
    }
}

/// Run the rule against one scene snapshot.
pub fn run(snap: &FrameSnapshot, scene_path: &Path) -> Vec<LintFinding> {
    let nodes_by_id = index_by_id(&snap.nodes);
    let mut findings = Vec::new();

    for (idx, node) in snap.nodes.iter().enumerate() {
        let Some(glyph_run) = node.glyph_run.as_ref() else {
            continue;
        };
        if glyph_run.glyphs.is_empty() {
            continue;
        }
        if !node.bbox.has_area() {
            continue;
        }
        if node.computed_opacity <= 0.0 {
            continue;
        }

        let clippers = collect_clipping_ancestors(node, &snap.nodes, &nodes_by_id);

        // Canvas-viewport check is a backstop for the case where NO
        // ancestor has `overflow: hidden` (the v8 CTA-overlay failure
        // mode). When some ancestor already covers the canvas with a
        // clip, the ancestor-clip path below reports the same defect
        // with a more specific selector — skip the synthetic finding
        // there to avoid double-reporting.
        let viewport_rect = Rect {
            x: 0.0,
            y: 0.0,
            w: snap.viewport.0 as f32,
            h: snap.viewport.1 as f32,
        };
        let canvas_covered = clippers.iter().any(|&ci| {
            let b = snap.nodes[ci].bbox;
            b.x <= 0.5
                && b.y <= 0.5
                && b.x + b.w + 0.5 >= viewport_rect.w
                && b.y + b.h + 0.5 >= viewport_rect.h
        });
        if !canvas_covered {
            let full_chain_xform =
                compose_chain(node, None, &snap.nodes, &nodes_by_id);
            let vp_bbox_miss = aggregate_bbox_miss_xformed(
                node.bbox,
                viewport_rect,
                &full_chain_xform,
            );
            let vp_glyph_miss = aggregate_glyph_miss_xformed(
                node,
                glyph_run.glyphs.as_slice(),
                viewport_rect,
                &full_chain_xform,
            );
            let vp_miss = combine(&vp_bbox_miss, &vp_glyph_miss);
            if vp_miss.any() {
                let elem_selector = best_selector(snap, node, idx);
                let detail = if vp_bbox_miss.any() && vp_glyph_miss.any() {
                    "element bbox + glyph ink"
                } else if vp_bbox_miss.any() {
                    "element bbox"
                } else {
                    "glyph ink"
                };
                findings.push(LintFinding {
                    rule: RULE.to_string(),
                    severity: severity_for(vp_miss.max()),
                    scene_path: scene_path.to_path_buf(),
                    t_secs: snap.t_secs,
                    element_selector: elem_selector.clone(),
                    element_bbox: node.bbox,
                    message: format!(
                        "{} painted outside canvas viewport ({}×{}); missing {}",
                        detail,
                        snap.viewport.0,
                        snap.viewport.1,
                        describe_miss(&vp_miss),
                    ),
                    fix_hint: format!(
                        "reposition {elem_selector} inside the canvas, shrink \
                         its font-size, or wrap it in a container sized to the \
                         canvas — text outside the viewport is never painted"
                    ),
                    subkind: Some("canvas".to_string()),
                });
            }
        }

        if clippers.is_empty() {
            continue;
        }

        // Compute one clip rect per ancestor clipper, intersected with
        // every outer clipper. We emit at most one finding per
        // (text-element, ancestor-clipper) pair that introduces a NEW
        // edge miss. Multiple clippers on the same edge collapse into
        // the innermost one.
        let mut clip_chain: Vec<(usize, Rect)> = Vec::new();
        let mut accum = unbounded_rect();
        for &anc_idx in &clippers {
            accum = intersect(accum, snap.nodes[anc_idx].bbox);
            clip_chain.push((anc_idx, accum));
        }

        let mut prev_miss = EdgeMiss::default();
        for &(anc_idx, clip_rect) in &clip_chain {
            // Compose the cumulative 2D affine from this clipping
            // ancestor's nearest descendant down to (and including) the
            // text element. CSS clipping happens in the ancestor's local
            // coordinate frame, so the ancestor's own transform does NOT
            // apply to the clip-vs-glyph test, but every transform on
            // the chain below it does. Identity when nothing on that
            // chain is transformed.
            let chain_xform =
                compose_chain(node, Some(anc_idx), &snap.nodes, &nodes_by_id);

            // Two-stage check. First: does the element's own bbox bust
            // the clip after its own + descendant-chain transforms? An
            // oversized text element with letter-spacing / animation-
            // start sizing larger than its container is itself a primary
            // clip vector even when the glyph ink rects all land inside.
            // Second: per-glyph ink check for the italic / ascender /
            // descender cases the bbox check misses.
            let bbox_miss = aggregate_bbox_miss_xformed(node.bbox, clip_rect, &chain_xform);
            let glyph_miss = aggregate_glyph_miss_xformed(
                node,
                glyph_run.glyphs.as_slice(),
                clip_rect,
                &chain_xform,
            );
            let miss = combine(&bbox_miss, &glyph_miss);
            if !miss.any() {
                continue;
            }
            if !adds_new_edge(&miss, &prev_miss) {
                continue;
            }

            let anc = &snap.nodes[anc_idx];
            let severity = severity_for(miss.max());
            let anc_selector = best_selector(snap, anc, anc_idx);
            let elem_selector = best_selector(snap, node, idx);
            let detail = if bbox_miss.any() && glyph_miss.any() {
                "element bbox + glyph ink"
            } else if bbox_miss.any() {
                "element bbox"
            } else {
                "glyph ink"
            };
            findings.push(LintFinding {
                rule: RULE.to_string(),
                severity,
                scene_path: scene_path.to_path_buf(),
                t_secs: snap.t_secs,
                element_selector: elem_selector,
                element_bbox: node.bbox,
                message: format!(
                    "{} clipped by {} (clips its descendants, bbox {}); missing {}",
                    detail,
                    anc_selector,
                    fmt_rect(anc.bbox),
                    describe_miss(&miss)
                ),
                fix_hint: build_fix_hint(&miss, &anc_selector),
                subkind: None,
            });

            prev_miss.fold(&miss);
        }
    }

    findings
}

/// Measure how far the element's own bbox — after applying the
/// cumulative descendant-chain transform — exceeds `clip_rect` on each
/// edge. Captures the primary clip vector when an over-letter-spaced,
/// animation-start-sized, OR animation-scaled text element ends up
/// painting larger than its container.
///
/// `chain_xform` operates around the bbox origin (element-local frame
/// translated to (0, 0)); pass `Affine::IDENTITY` for the no-transform
/// case. We bbox-transform all four corners and take the axis-aligned
/// bounding rect, which is the conservative answer for any 2D affine
/// (rotations included).
fn aggregate_bbox_miss_xformed(bbox: Rect, clip_rect: Rect, chain_xform: &Affine) -> EdgeMiss {
    if !bbox.has_area() {
        return EdgeMiss::default();
    }
    let xformed = transform_rect(bbox, chain_xform);
    let clip_x1 = clip_rect.x + clip_rect.w;
    let clip_y1 = clip_rect.y + clip_rect.h;
    let bx1 = xformed.x + xformed.w;
    let by1 = xformed.y + xformed.h;
    EdgeMiss {
        top: (clip_rect.y - xformed.y).max(0.0),
        bottom: (by1 - clip_y1).max(0.0),
        left: (clip_rect.x - xformed.x).max(0.0),
        right: (bx1 - clip_x1).max(0.0),
    }
}

/// Per-edge maximum of two EdgeMisses. Used to merge the bbox-vs-clip
/// and glyph-ink-vs-clip results into one finding when both fire.
fn combine(a: &EdgeMiss, b: &EdgeMiss) -> EdgeMiss {
    EdgeMiss {
        top: a.top.max(b.top),
        bottom: a.bottom.max(b.bottom),
        left: a.left.max(b.left),
        right: a.right.max(b.right),
    }
}

/// For every glyph, lift it from element-local to scene-absolute,
/// apply the cumulative descendant-chain transform, and measure how
/// far its ink rect exceeds `clip_rect` on each edge. Aggregated
/// per-element: worst overshoot on each edge across all glyphs.
///
/// Transform application: ink rects are in element-local space; we
/// add the bbox origin to get document-space, then `chain_xform`
/// (which operates around bbox origin with transform-origin already
/// baked in by `resolve_2d_transform`) gives us the painted position.
/// For non-axis-aligned chain transforms (rotation, skew), we take
/// the AABB of the four transformed ink corners — the conservative
/// envelope.
fn aggregate_glyph_miss_xformed(
    node: &NodeSnapshot,
    glyphs: &[GlyphInk],
    clip_rect: Rect,
    chain_xform: &Affine,
) -> EdgeMiss {
    let mut acc = EdgeMiss::default();
    let ox = node.bbox.x;
    let oy = node.bbox.y;
    let clip_x0 = clip_rect.x;
    let clip_y0 = clip_rect.y;
    let clip_x1 = clip_rect.x + clip_rect.w;
    let clip_y1 = clip_rect.y + clip_rect.h;
    for g in glyphs {
        let ink_x0 = ox + g.ink_min_x;
        let ink_y0 = oy + g.ink_min_y;
        let ink_x1 = ox + g.ink_max_x;
        let ink_y1 = oy + g.ink_max_y;
        // Skip degenerate glyphs (zero-area ink — typically whitespace
        // or unmappable codepoints with .notdef stripped).
        if !(ink_x1 > ink_x0 && ink_y1 > ink_y0) {
            continue;
        }
        let ink_rect = Rect {
            x: ink_x0,
            y: ink_y0,
            w: ink_x1 - ink_x0,
            h: ink_y1 - ink_y0,
        };
        let xformed = transform_rect(ink_rect, chain_xform);
        let top = (clip_y0 - xformed.y).max(0.0);
        let bottom = (xformed.y + xformed.h - clip_y1).max(0.0);
        let left = (clip_x0 - xformed.x).max(0.0);
        let right = (xformed.x + xformed.w - clip_x1).max(0.0);
        if top > acc.top {
            acc.top = top;
        }
        if bottom > acc.bottom {
            acc.bottom = bottom;
        }
        if left > acc.left {
            acc.left = left;
        }
        if right > acc.right {
            acc.right = right;
        }
    }
    acc
}

/// Compose the cumulative 2D affine from `elem` upward through its
/// ancestor chain. When `stop_at` is `Some(anc_idx)`, the walk stops
/// at (exclusive of) that ancestor — used for the clipping-ancestor
/// case, where the ancestor's own transform cancels with the clip
/// rect (both live in the ancestor's local frame). When `None`, the
/// walk goes all the way to the document root — used for the canvas-
/// viewport check, where the viewport is a fixed rect in document
/// space affected by every ancestor's transform.
///
/// Each element's transform operates around its own bbox origin in
/// element-local space (`transform-origin` already baked in by
/// `blitz_dom::resolve_2d_transform`). We wrap each with translations
/// to/from its bbox origin so the composed matrix operates on points
/// in document coordinates. Returns `Affine::IDENTITY` when no element
/// on the chain has a transform set.
fn compose_chain(
    elem: &NodeSnapshot,
    stop_at: Option<usize>,
    nodes: &[NodeSnapshot],
    map: &[(usize, usize)],
) -> Affine {
    let stop_id = stop_at.map(|i| nodes[i].id);
    let mut chain: Vec<&NodeSnapshot> = Vec::new();
    chain.push(elem);
    let mut cursor = elem.parent;
    while let Some(pid) = cursor {
        if Some(pid) == stop_id {
            break;
        }
        let Some(p_idx) = lookup(map, pid) else { break };
        let p = &nodes[p_idx];
        chain.push(p);
        cursor = p.parent;
    }

    // Walking leaf → ancestor: each ancestor's wrapped affine
    // pre-multiplies the accumulator so far, matching CSS composition
    // order (leaf transform applies first, ancestor transforms wrap).
    let mut acc = Affine::IDENTITY;
    for n in chain {
        if let Some(coeffs) = n.transform {
            let local = Affine::new([
                coeffs[0] as f64,
                coeffs[1] as f64,
                coeffs[2] as f64,
                coeffs[3] as f64,
                coeffs[4] as f64,
                coeffs[5] as f64,
            ]);
            let to_local = Affine::translate(kurbo::Vec2::new(
                -n.bbox.x as f64,
                -n.bbox.y as f64,
            ));
            let to_doc = Affine::translate(kurbo::Vec2::new(
                n.bbox.x as f64,
                n.bbox.y as f64,
            ));
            acc = (to_doc * local * to_local) * acc;
        }
    }
    acc
}

/// Apply `xform` to the four corners of `rect` and return their
/// axis-aligned bounding box. For pure translation / scale this is
/// the exact transformed rect; for rotation / skew it's the conserv-
/// ative envelope, which is what we want for a clipping check.
fn transform_rect(rect: Rect, xform: &Affine) -> Rect {
    let x0 = rect.x as f64;
    let y0 = rect.y as f64;
    let x1 = (rect.x + rect.w) as f64;
    let y1 = (rect.y + rect.h) as f64;
    let corners = [
        kurbo::Point::new(x0, y0),
        kurbo::Point::new(x1, y0),
        kurbo::Point::new(x0, y1),
        kurbo::Point::new(x1, y1),
    ];
    let mut min_x = f64::INFINITY;
    let mut min_y = f64::INFINITY;
    let mut max_x = f64::NEG_INFINITY;
    let mut max_y = f64::NEG_INFINITY;
    for c in corners {
        let p = *xform * c;
        if p.x < min_x {
            min_x = p.x;
        }
        if p.y < min_y {
            min_y = p.y;
        }
        if p.x > max_x {
            max_x = p.x;
        }
        if p.y > max_y {
            max_y = p.y;
        }
    }
    Rect {
        x: min_x as f32,
        y: min_y as f32,
        w: (max_x - min_x) as f32,
        h: (max_y - min_y) as f32,
    }
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

/// True if `next` reports a miss on at least one edge that `prev` did
/// not. Used to suppress reporting a chain of outer ancestors that all
/// produce the same clipping behaviour — the innermost one already
/// describes the problem.
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
         OR shrink the text (font-size / line-height) to fit"
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
#[path = "glyph_clip_tests.rs"]
mod tests;
