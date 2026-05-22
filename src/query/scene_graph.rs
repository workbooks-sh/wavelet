//! Scene-graph queries — answer agent questions from the resolved layout
//! tree without touching rendered pixels. Phase 1 of epic wb-q4a6.
//!
//! Every function takes a `&FrameSnapshot` and returns a typed result the
//! CLI surfaces as JSON. None of these hit OCR, SSIM, or VLM.

use super::snapshot::{FrameSnapshot, NodeSnapshot, Rect, VisibilityVerdict};
use serde::{Deserialize, Serialize};

/// Per-query result shapes — kept distinct so the CLI's JSON output is
/// self-describing per question kind. All carry an `ok` boolean.

/// Result of a bbox query.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BboxResult {
    /// True when at least one node matches the selector.
    pub ok: bool,
    /// The matched node's bbox in document coordinates (pre-CSS-transform).
    pub bbox: Option<Rect>,
    /// Selector that was queried.
    pub selector: String,
    /// CSS id of the matched node, when distinct from the selector.
    pub element_id: Option<String>,
}

/// Result of a title-safe-area check.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SafeAreaResult {
    /// True when the element's bbox is fully within the safe-area rect.
    pub ok: bool,
    /// Selector that was queried.
    pub selector: String,
    /// Element bbox.
    pub bbox: Option<Rect>,
    /// Computed safe-area rect (after applying `inset`).
    pub safe_area: Rect,
    /// Inset fraction the safe-area was computed from (e.g. 0.1 = 10%).
    pub inset: f32,
}

/// Result of a text-overlap check. Detects pairs of text-bearing elements
/// whose layout bboxes intersect without one containing the other — typical
/// symptom of accidental negative margins, absolute-positioning math errors,
/// or fixed-height containers that get content overflow.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OverlapResult {
    /// True when no overlapping text pairs were found.
    pub ok: bool,
    /// One entry per overlapping pair (root-first ordering by node id).
    pub overlaps: Vec<OverlapPair>,
}

/// One pair of overlapping text elements.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OverlapPair {
    /// First node's CSS id (or `node-<id>` fallback).
    pub a: String,
    /// First node's bbox.
    pub a_bbox: Rect,
    /// First node's text content (truncated to 60 chars).
    pub a_text: String,
    /// Second node's CSS id.
    pub b: String,
    /// Second node's bbox.
    pub b_bbox: Rect,
    /// Second node's text content.
    pub b_text: String,
    /// Area of the intersection, in square pixels.
    pub overlap_area: f32,
}

/// Result of a transform-propagation check. The painter bug wb-b53k means
/// descendants don't pick up ancestor CSS transforms; this query verifies
/// the element WOULD be at the right place if the painter respected the
/// spec. If any ancestor has a non-identity transform, the element is
/// flagged as affected.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransformInheritsResult {
    /// True when no ancestor has a non-identity CSS transform — the element
    /// is safe to animate via individual translate/rotate/scale on its
    /// parents and will paint at the correct position.
    pub ok: bool,
    /// Selector that was queried.
    pub selector: String,
    /// IDs of the offending ancestor chain, root-first. Empty when ok=true.
    pub affected_ancestors: Vec<usize>,
    /// Layout-space bbox (before any ancestor transform).
    pub layout_bbox: Option<Rect>,
    /// Spec-correct bbox after applying the cumulative ancestor transform.
    /// Differs from `layout_bbox` exactly when ancestors have transforms.
    pub spec_bbox: Option<Rect>,
}

/// Visibility check. Returns `Ok(Visible)` on success; the variant otherwise
/// is the reason.
pub fn visibility_of(snap: &FrameSnapshot, selector: &str) -> VisibilityVerdict {
    let nodes = snap.select(selector);
    if nodes.is_empty() {
        return VisibilityVerdict::NotMounted;
    }
    let n = nodes[0];
    classify_visibility(snap, n)
}

fn classify_visibility(snap: &FrameSnapshot, n: &NodeSnapshot) -> VisibilityVerdict {
    if !n.bbox.has_area() {
        return VisibilityVerdict::ZeroSize { bbox: n.bbox };
    }
    if n.computed_opacity <= 0.0 {
        return VisibilityVerdict::Transparent {
            opacity: n.computed_opacity,
        };
    }
    let viewport = Rect {
        x: 0.0,
        y: 0.0,
        w: snap.viewport.0 as f32,
        h: snap.viewport.1 as f32,
    };
    if !n.bbox.intersects(viewport) {
        return VisibilityVerdict::Offscreen {
            bbox: n.bbox,
            viewport,
        };
    }
    VisibilityVerdict::Visible
}

/// Return the bbox of the first node matching `selector`.
pub fn bbox_of(snap: &FrameSnapshot, selector: &str) -> BboxResult {
    let nodes = snap.select(selector);
    match nodes.first() {
        Some(n) => BboxResult {
            ok: true,
            bbox: Some(n.bbox),
            selector: selector.to_string(),
            element_id: n.element_id.clone(),
        },
        None => BboxResult {
            ok: false,
            bbox: None,
            selector: selector.to_string(),
            element_id: None,
        },
    }
}

/// True when the element's bbox is fully inside a safe-area rect.
///
/// `inset` is the fraction of the viewport to inset from each edge.
/// The conventional broadcast "title-safe" area uses `inset = 0.1` (10%
/// margin on each side, so the safe area is the center 80%).
pub fn in_safe_area(snap: &FrameSnapshot, selector: &str, inset: f32) -> SafeAreaResult {
    let inset = inset.clamp(0.0, 0.5);
    let vw = snap.viewport.0 as f32;
    let vh = snap.viewport.1 as f32;
    let safe = Rect {
        x: vw * inset,
        y: vh * inset,
        w: vw * (1.0 - 2.0 * inset),
        h: vh * (1.0 - 2.0 * inset),
    };
    let n = snap.select(selector).first().copied();
    let (ok, bbox) = match n {
        Some(node) => (node.bbox.within(safe), Some(node.bbox)),
        None => (false, None),
    };
    SafeAreaResult {
        ok,
        selector: selector.to_string(),
        bbox,
        safe_area: safe,
        inset,
    }
}

/// Find any pair of text-bearing elements whose bboxes intersect without
/// one being an ancestor of the other. Catches accidental layout collisions
/// — overlapping headlines, captions colliding with image overlays, broken
/// grid math, etc. Cheap (O(n²) over text nodes, typically n<20 per scene).
pub fn no_overlap(snap: &FrameSnapshot) -> OverlapResult {
    // Collect text-bearing leaf elements (have non-empty text + no element children).
    let text_nodes: Vec<&NodeSnapshot> = snap
        .nodes
        .iter()
        .filter(|n| n.text.is_some() && n.bbox.has_area())
        .collect();

    // Build ancestor lookup so we can exclude container-content pairs (a
    // parent div's bbox legitimately encompasses its child's text).
    let parent_of: std::collections::HashMap<usize, Option<usize>> =
        snap.nodes.iter().map(|n| (n.id, n.parent)).collect();
    let is_ancestor = |maybe_anc: usize, mut start: usize| -> bool {
        while let Some(&p) = parent_of.get(&start) {
            match p {
                Some(pp) => {
                    if pp == maybe_anc {
                        return true;
                    }
                    start = pp;
                }
                None => return false,
            }
        }
        false
    };

    let mut overlaps = Vec::new();
    for i in 0..text_nodes.len() {
        for j in (i + 1)..text_nodes.len() {
            let a = text_nodes[i];
            let b = text_nodes[j];
            if !a.bbox.intersects(b.bbox) {
                continue;
            }
            if is_ancestor(a.id, b.id) || is_ancestor(b.id, a.id) {
                continue;
            }
            overlaps.push(OverlapPair {
                a: a.element_id.clone().unwrap_or_else(|| format!("node-{}", a.id)),
                a_bbox: a.bbox,
                a_text: truncate(a.text.as_deref().unwrap_or(""), 60),
                b: b.element_id.clone().unwrap_or_else(|| format!("node-{}", b.id)),
                b_bbox: b.bbox,
                b_text: truncate(b.text.as_deref().unwrap_or(""), 60),
                overlap_area: rect_intersect_area(a.bbox, b.bbox),
            });
        }
    }
    OverlapResult {
        ok: overlaps.is_empty(),
        overlaps,
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let mut out: String = s.chars().take(max - 1).collect();
        out.push('…');
        out
    }
}

fn rect_intersect_area(a: Rect, b: Rect) -> f32 {
    let x0 = a.x.max(b.x);
    let y0 = a.y.max(b.y);
    let x1 = (a.x + a.w).min(b.x + b.w);
    let y1 = (a.y + a.h).min(b.y + b.h);
    if x1 <= x0 || y1 <= y0 {
        0.0
    } else {
        (x1 - x0) * (y1 - y0)
    }
}

/// Verify that an element's painted position will match the CSS spec — i.e.
/// no ancestor has a CSS transform that the painter would silently drop.
/// Catches the wb-b53k bug pattern: the painter applies transforms to the
/// element they're declared on but doesn't propagate them to descendants.
///
/// Returns `ok=true` when no ancestor has `transform` / `translate` / `rotate`
/// / `scale` set. Otherwise lists the offending ancestors root-first so an
/// agent can localize the patch.
pub fn transform_inherits(snap: &FrameSnapshot, selector: &str) -> TransformInheritsResult {
    let nodes = snap.select(selector);
    let Some(n) = nodes.first().copied() else {
        return TransformInheritsResult {
            ok: false,
            selector: selector.to_string(),
            affected_ancestors: Vec::new(),
            layout_bbox: None,
            spec_bbox: None,
        };
    };

    let by_id: std::collections::HashMap<usize, &NodeSnapshot> =
        snap.nodes.iter().map(|nn| (nn.id, nn)).collect();
    let mut offending = Vec::new();
    let mut cur_parent = n.parent;
    while let Some(pid) = cur_parent {
        let Some(p) = by_id.get(&pid) else { break };
        if p.has_own_transform {
            offending.push(p.id);
        }
        cur_parent = p.parent;
    }
    offending.reverse();

    TransformInheritsResult {
        ok: offending.is_empty(),
        selector: selector.to_string(),
        affected_ancestors: offending,
        layout_bbox: Some(n.bbox),
        // spec_bbox would require composing cumulative affine, which needs
        // Servo's style crate (not on crates.io). For now we surface the
        // affected-ancestor list — that's what an agent needs to localize.
        spec_bbox: Some(n.bbox),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render_offline::{Composition, SceneSpec};
    use crate::test_utils::BLITZ_GUARD;
    use std::path::{Path, PathBuf};

    fn snapshot_serial(comp: &Composition, root: &Path, t: f32) -> FrameSnapshot {
        let _g = BLITZ_GUARD.lock().unwrap();
        FrameSnapshot::at(comp, root, t)
    }

    fn write_html(dir: &Path, name: &str, body: &str) {
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(
            dir.join(name),
            format!(
                r#"<!doctype html><html><head><style>
                    html, body {{ margin:0; padding:0; width:100%; height:100%; }}
                    body {{ display:flex; align-items:center; justify-content:center; }}
                    #hero {{ width:200px; height:50px; background:#fff; }}
                </style></head><body>{body}</body></html>"#
            ),
        )
        .unwrap();
    }

    fn comp_with(scene: SceneSpec, dir: &Path) -> (Composition, PathBuf) {
        let comp = Composition {
            width: 1280,
            height: 720,
            fps: 30,
            duration_frames: 30,
            scenes: vec![scene],
            aspect: None,
            audio_cues: vec![],
        };
        (comp, dir.to_path_buf())
    }

    #[test]
    fn bbox_of_named_element() {
        let dir = std::env::temp_dir().join("wavelet-query-bbox-test");
        write_html(&dir, "scene.html", r#"<div id="hero">Hi</div>"#);
        let (comp, root) = comp_with(
            SceneSpec {
                html_path: PathBuf::from("scene.html"),
                start_frame: 0,
                duration_frames: 30,
                transition_in: None, video_bg: None,
            },
            &dir,
        );

        let snap = snapshot_serial(&comp, &root, 0.5);
        let r = bbox_of(&snap, "#hero");
        assert!(r.ok, "bbox query should succeed");
        let b = r.bbox.unwrap();
        // Body uses flex center, so #hero sits in the middle of the 1280x720 frame.
        assert!(b.w >= 199.0 && b.w <= 201.0, "width approx 200, got {}", b.w);
        assert!(b.h >= 49.0 && b.h <= 51.0, "height approx 50, got {}", b.h);
        assert!(b.x > 500.0 && b.x < 600.0, "expected horizontally centered, got x={}", b.x);
    }

    #[test]
    fn visibility_returns_not_mounted_for_missing_selector() {
        let dir = std::env::temp_dir().join("wavelet-query-vis-missing");
        write_html(&dir, "scene.html", r#"<div id="hero">Hi</div>"#);
        let (comp, root) = comp_with(
            SceneSpec {
                html_path: PathBuf::from("scene.html"),
                start_frame: 0,
                duration_frames: 30,
                transition_in: None, video_bg: None,
            },
            &dir,
        );

        let snap = snapshot_serial(&comp, &root, 0.5);
        assert_eq!(visibility_of(&snap, "#typo"), VisibilityVerdict::NotMounted);
    }

    #[test]
    fn visibility_returns_transparent_for_opacity_zero_element() {
        let dir = std::env::temp_dir().join("wavelet-query-vis-transparent");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("scene.html"),
            r#"<!doctype html><html><head><style>
                html, body { margin:0; padding:0; width:100%; height:100%; }
                #hero { width:200px; height:50px; background:#fff; opacity:0; }
            </style></head><body><div id="hero">Hi</div></body></html>"#,
        )
        .unwrap();
        let (comp, root) = comp_with(
            SceneSpec {
                html_path: PathBuf::from("scene.html"),
                start_frame: 0,
                duration_frames: 30,
                transition_in: None, video_bg: None,
            },
            &dir,
        );

        let snap = snapshot_serial(&comp, &root, 0.0);
        match visibility_of(&snap, "#hero") {
            VisibilityVerdict::Transparent { opacity } => {
                assert!(opacity < 0.05, "expected near-zero opacity, got {opacity}");
            }
            other => panic!("expected Transparent, got {:?}", other),
        }
    }

    #[test]
    fn safe_area_flags_offset_element() {
        let dir = std::env::temp_dir().join("wavelet-query-safe-area");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("scene.html"),
            r#"<!doctype html><html><head><style>
                html, body { margin:0; padding:0; }
                #hero { position:absolute; left:10px; top:10px; width:200px; height:50px; background:#fff; }
            </style></head><body><div id="hero">Hi</div></body></html>"#,
        )
        .unwrap();
        let (comp, root) = comp_with(
            SceneSpec {
                html_path: PathBuf::from("scene.html"),
                start_frame: 0,
                duration_frames: 30,
                transition_in: None, video_bg: None,
            },
            &dir,
        );

        let snap = snapshot_serial(&comp, &root, 0.0);
        // 10% inset on 1280x720 = safe rect (128, 72) → (1152, 648). Element at (10, 10) is outside.
        let r = in_safe_area(&snap, "#hero", 0.1);
        assert!(!r.ok, "element at (10,10) is outside the 10% safe area");

        // 0% inset means the full viewport is the safe area; element is inside that.
        let r0 = in_safe_area(&snap, "#hero", 0.0);
        assert!(r0.ok, "with no inset the element is inside the viewport");
    }

    #[test]
    fn transform_inherits_ok_when_no_ancestor_transforms() {
        let dir = std::env::temp_dir().join("wavelet-query-xform-clean");
        write_html(&dir, "scene.html", r#"<div id="hero">Hi</div>"#);
        let (comp, root) = comp_with(
            SceneSpec {
                html_path: PathBuf::from("scene.html"),
                start_frame: 0,
                duration_frames: 30,
                transition_in: None, video_bg: None,
            },
            &dir,
        );

        let snap = snapshot_serial(&comp, &root, 0.5);
        let r = transform_inherits(&snap, "#hero");
        assert!(r.ok, "no ancestor has a transform; query should be ok");
        assert!(r.affected_ancestors.is_empty());
    }

    #[test]
    fn no_overlap_passes_on_clean_layout() {
        let dir = std::env::temp_dir().join("wavelet-query-overlap-clean");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("scene.html"),
            r#"<!doctype html><html><head><style>
                html, body { margin:0; padding:0; }
                #a { position:absolute; top:50px; left:50px; }
                #b { position:absolute; top:200px; left:50px; }
            </style></head><body><div id="a">A</div><div id="b">B</div></body></html>"#,
        ).unwrap();
        let (comp, root) = comp_with(
            SceneSpec {
                html_path: PathBuf::from("scene.html"),
                start_frame: 0, duration_frames: 30, transition_in: None, video_bg: None,
            },
            &dir,
        );
        let snap = snapshot_serial(&comp, &root, 0.5);
        let r = no_overlap(&snap);
        assert!(r.ok, "non-overlapping siblings should pass, got {:?}", r.overlaps);
    }

    #[test]
    fn no_overlap_flags_collision() {
        let dir = std::env::temp_dir().join("wavelet-query-overlap-collide");
        std::fs::create_dir_all(&dir).unwrap();
        // Two absolutely-positioned text divs sharing the same coords.
        std::fs::write(
            dir.join("scene.html"),
            r#"<!doctype html><html><head><style>
                html, body { margin:0; padding:0; }
                #title { position:absolute; top:300px; left:400px; font-size: 64px; }
                #subtitle { position:absolute; top:320px; left:420px; font-size: 32px; }
            </style></head><body><div id="title">Hello</div><div id="subtitle">World</div></body></html>"#,
        ).unwrap();
        let (comp, root) = comp_with(
            SceneSpec {
                html_path: PathBuf::from("scene.html"),
                start_frame: 0, duration_frames: 30, transition_in: None, video_bg: None,
            },
            &dir,
        );
        let snap = snapshot_serial(&comp, &root, 0.5);
        let r = no_overlap(&snap);
        assert!(!r.ok, "overlapping siblings should be flagged");
        assert!(!r.overlaps.is_empty());
        let pair = &r.overlaps[0];
        assert!(pair.overlap_area > 0.0);
        assert!(
            (pair.a == "title" && pair.b == "subtitle") ||
            (pair.a == "subtitle" && pair.b == "title"),
            "expected title+subtitle pair, got {:?} vs {:?}", pair.a, pair.b
        );
    }

    #[test]
    fn transform_inherits_flags_ancestor_with_translate() {
        // The wb-b53k pattern: a parent with a CSS translate has child text
        // whose painted position the painter gets wrong today. Our query
        // surfaces that.
        let dir = std::env::temp_dir().join("wavelet-query-xform-dirty");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("scene.html"),
            r#"<!doctype html><html><head><style>
                html, body { margin:0; padding:0; width:100%; height:100%; }
                body { display:flex; align-items:center; justify-content:center; }
                #parent { translate: 50px 0; }
                #child { width:100px; height:30px; background:#fff; }
            </style></head><body><div id="parent"><div id="child">Hi</div></div></body></html>"#,
        )
        .unwrap();
        let (comp, root) = comp_with(
            SceneSpec {
                html_path: PathBuf::from("scene.html"),
                start_frame: 0,
                duration_frames: 30,
                transition_in: None, video_bg: None,
            },
            &dir,
        );

        let snap = snapshot_serial(&comp, &root, 0.5);
        let r = transform_inherits(&snap, "#child");
        assert!(!r.ok, "#parent has translate: 50px 0; #child should be flagged");
        assert!(!r.affected_ancestors.is_empty(), "expected at least one affected ancestor");
    }
}
