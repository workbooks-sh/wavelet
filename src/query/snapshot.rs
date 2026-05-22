//! `FrameSnapshot` — the resolved scene-graph at one moment in time.
//!
//! Captures every element's world-space layout bbox, computed opacity, text
//! content, CSS transform, and a stable cross-run `semantic_id`. This is the
//! data structure every scene-graph query reads from — by walking the Blitz
//! tree exactly once per `(comp, t_secs)` pair, we amortize the cost of
//! repeated queries at the same time.
//!
//! The shape borrows from an earlier RVST scene-snapshot design but adds:
//!   - `t_secs` + `frame_index` for time awareness
//!   - `world_transform` capturing accumulated CSS transforms from ancestors
//!     (Blitz's painter doesn't propagate parent transforms to descendants
//!     today — wb-b53k — so we compute the "spec-correct" cumulative
//!     transform here for queries that want to compare against painter output)

use crate::render::load_html_with_base;
use crate::render_offline::{Composition, SceneSpec};
use blitz_dom::{BaseDocument, Node};
use serde::{Deserialize, Serialize};
use std::hash::{Hash, Hasher};
use std::path::Path;
use twox_hash::XxHash64;

/// Axis-aligned rectangle in document coordinates, pre-CSS-transform.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Rect {
    /// Left edge in document pixels.
    pub x: f32,
    /// Top edge in document pixels.
    pub y: f32,
    /// Width in pixels.
    pub w: f32,
    /// Height in pixels.
    pub h: f32,
}

impl Rect {
    /// True if the rectangle has any area at all.
    pub fn has_area(&self) -> bool {
        self.w > 0.0 && self.h > 0.0
    }

    /// True if this rectangle is fully contained within `viewport`.
    pub fn within(&self, viewport: Rect) -> bool {
        self.x >= viewport.x
            && self.y >= viewport.y
            && self.x + self.w <= viewport.x + viewport.w
            && self.y + self.h <= viewport.y + viewport.h
    }

    /// True if this rectangle intersects `other`.
    pub fn intersects(&self, other: Rect) -> bool {
        self.x < other.x + other.w
            && other.x < self.x + self.w
            && self.y < other.y + other.h
            && other.y < self.y + self.h
    }
}

/// Why an element is or isn't visible at the queried time. Structured so an
/// agent can branch on the variant without parsing a string. Mirrors RVST's
/// `InvisibilityReason` (snapshot.rs:648) with two added time-aware variants
/// for motion-graphics use.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum VisibilityVerdict {
    /// Element is mounted, laid out, has area, opacity > 0, and is on-screen.
    Visible,
    /// No node in the document matches the selector at all.
    NotMounted,
    /// `display: none` on the element or an ancestor.
    DisplayNone,
    /// Layout box has zero width or height.
    ZeroSize {
        /// The (zero-area) layout box.
        bbox: Rect,
    },
    /// Computed opacity rolls up to zero.
    Transparent {
        /// Final computed opacity (parent_opacity * own_opacity).
        opacity: f32,
    },
    /// Bounding box falls outside the viewport.
    Offscreen {
        /// Element's layout bbox.
        bbox: Rect,
        /// Viewport rect for reference.
        viewport: Rect,
    },
    /// An ancestor's overflow / clip-path clips the element entirely out.
    ClippedByAncestor {
        /// Ancestor that introduced the clip.
        ancestor_id: usize,
        /// The clip rect (in document coordinates).
        ancestor_clip: Rect,
    },
    /// Element exists but its scene's start_frame is later than the queried
    /// time. (Motion-graphics-aware variant — not in RVST.)
    NotYetStarted {
        /// Time in seconds when the element first becomes mounted.
        starts_at_secs: f32,
    },
    /// Element's scene has already ended at the queried time.
    AlreadyEnded {
        /// Time in seconds when the element was last mounted.
        ended_at_secs: f32,
    },
}

/// One element's state at a single moment.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeSnapshot {
    /// Blitz internal node id. Not stable across runs.
    pub id: usize,
    /// Stable cross-run hash of (tag + element_id + classes + parent semantic_id).
    /// Survives small content edits in adjacent siblings.
    pub semantic_id: String,
    /// HTML tag name, lowercased.
    pub tag: String,
    /// CSS `id` attribute, if any.
    pub element_id: Option<String>,
    /// CSS class list.
    pub classes: Vec<String>,
    /// Layout-space bbox in document coordinates. Pre-CSS-transform.
    pub bbox: Rect,
    /// True when this element has a non-identity CSS transform set
    /// (via `transform`, `translate`, `rotate`, or `scale`). Used by
    /// `transform_inherits` to detect the wb-b53k bug pattern without
    /// needing access to Servo's `style` crate (Stylo isn't on crates.io).
    pub has_own_transform: bool,
    /// True when this node's computed style clips its descendants —
    /// `overflow-x|y: hidden|clip`, OR `clip-path` set to anything other
    /// than `none`, OR a non-`none` `mask-image`. Consumers (e.g. the
    /// `glyph-clip` lint rule) treat this as "this node's bbox is a
    /// clip rectangle for everything beneath it."
    pub clips_overflow: bool,
    /// Computed `opacity` rolled up through ancestors.
    pub computed_opacity: f32,
    /// Concatenated text content of any direct text children, trimmed.
    pub text: Option<String>,
    /// Child node ids — see `FrameSnapshot.nodes_by_id` to resolve.
    pub children: Vec<usize>,
    /// Parent node id, if any (root has none).
    pub parent: Option<usize>,
}

/// The whole scene graph captured at one frame of one composition.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FrameSnapshot {
    /// Time in seconds from the composition's start.
    pub t_secs: f32,
    /// 0-indexed frame number = `t_secs * fps`, rounded down.
    pub frame_index: u32,
    /// Output viewport (width, height) in pixels.
    pub viewport: (u32, u32),
    /// Active scene index at this time, if any. None when the time falls
    /// between scenes or after the composition ends.
    pub active_scene: Option<usize>,
    /// Flat node list, captured in document order (root first).
    pub nodes: Vec<NodeSnapshot>,
}

impl FrameSnapshot {
    /// Build a snapshot for `comp` at the given time. Loads the active scene's
    /// HTML, applies its motion bindings, ticks the timeline, then walks the
    /// resolved Blitz document tree.
    ///
    /// `root_dir` is the directory the composition's relative paths resolve
    /// against — typically the parent directory of the comp.json file.
    pub fn at(comp: &Composition, root_dir: &Path, t_secs: f32) -> Self {
        let frame_index = (t_secs * comp.fps as f32) as u32;
        let frame_for_active = frame_index.min(comp.duration_frames.saturating_sub(1));

        let active_idx = comp.scenes.iter().position(|s| {
            frame_for_active >= s.start_frame
                && frame_for_active < s.start_frame + s.duration_frames
        });

        let Some(scene_idx) = active_idx else {
            return Self {
                t_secs,
                frame_index,
                viewport: (comp.width, comp.height),
                active_scene: None,
                nodes: Vec::new(),
            };
        };

        let scene = &comp.scenes[scene_idx];
        let local_frame = frame_index.saturating_sub(scene.start_frame);
        let local_t_secs = local_frame as f32 / comp.fps as f32;

        let nodes = capture_scene(scene, root_dir, comp.width, comp.height, local_t_secs);

        Self {
            t_secs,
            frame_index,
            viewport: (comp.width, comp.height),
            active_scene: Some(scene_idx),
            nodes,
        }
    }

    /// Snapshot a single HTML file at time `t_secs`. The caller supplies
    /// the canvas dimensions; resolution attrs / CSS vars on `<html>` are
    /// not consulted here. Used by `wavelet lint`, which walks scene
    /// files directly rather than going through a Composition.
    pub fn from_html(html_path: &Path, width: u32, height: u32, t_secs: f32) -> Self {
        let frame_index = (t_secs * 30.0) as u32;
        let html = match std::fs::read_to_string(html_path) {
            Ok(s) => s,
            Err(_) => {
                return Self {
                    t_secs,
                    frame_index,
                    viewport: (width, height),
                    active_scene: None,
                    nodes: Vec::new(),
                };
            }
        };
        let absolute = std::fs::canonicalize(html_path).unwrap_or_else(|_| html_path.to_path_buf());
        let base_url = url::Url::from_file_path(&absolute).ok().map(|u| u.to_string());
        let mut doc = load_html_with_base(&html, width, height, base_url);
        doc.as_mut().resolve(t_secs as f64);

        let base = doc.as_ref();
        let root = base.root_element();
        let mut nodes = Vec::new();
        walk_node(base, root.id, None, 1.0, "", &mut nodes);

        Self {
            t_secs,
            frame_index,
            viewport: (width, height),
            active_scene: Some(0),
            nodes,
        }
    }

    /// Look up a node by its CSS `id` attribute. Returns the first match in
    /// document order — duplicate ids should be a lint error, not a query
    /// failure.
    pub fn by_element_id(&self, element_id: &str) -> Option<&NodeSnapshot> {
        self.nodes
            .iter()
            .find(|n| n.element_id.as_deref() == Some(element_id))
    }

    /// Resolve a selector to all matching nodes in document order. Today
    /// only `#id` selectors are supported.
    pub fn select(&self, selector: &str) -> Vec<&NodeSnapshot> {
        let trimmed = selector.trim();
        if !trimmed.starts_with('#') {
            return Vec::new();
        }
        let want = &trimmed[1..];
        self.nodes
            .iter()
            .filter(|n| n.element_id.as_deref() == Some(want))
            .collect()
    }
}

/// Walk one scene's resolved document and capture every element.
fn capture_scene(
    scene: &SceneSpec,
    root_dir: &Path,
    width: u32,
    height: u32,
    local_t_secs: f32,
) -> Vec<NodeSnapshot> {
    let resolved = root_dir.join(&scene.html_path);
    let Ok(html) = std::fs::read_to_string(&resolved) else {
        return Vec::new();
    };

    let absolute = std::fs::canonicalize(&resolved).unwrap_or(resolved);
    let base_url = url::Url::from_file_path(&absolute)
        .ok()
        .map(|u| u.to_string());
    let mut doc = load_html_with_base(&html, width, height, base_url);

    doc.as_mut().resolve(local_t_secs as f64);

    let base = doc.as_ref();
    let root = base.root_element();
    let mut out = Vec::new();
    walk_node(base, root.id, None, 1.0, "", &mut out);
    out
}

/// Depth-first walk; captures the layout-space bbox, cumulative opacity, and
/// stable `semantic_id` for every element. CSS transform composition is
/// represented by `has_own_transform` (a boolean per node) rather than a
/// cumulative matrix — Servo's `style` crate isn't on crates.io so we can't
/// call `resolve_2d_transform` directly. The boolean is enough for Phase 1's
/// transform-propagation check.
fn walk_node(
    doc: &BaseDocument,
    node_id: usize,
    parent: Option<usize>,
    parent_opacity: f32,
    parent_semantic: &str,
    out: &mut Vec<NodeSnapshot>,
) {
    let Some(node) = doc.get_node(node_id) else {
        return;
    };

    let Some(element_data) = node.data.downcast_element() else {
        for &c in &node.children {
            walk_node(doc, c, parent, parent_opacity, parent_semantic, out);
        }
        return;
    };

    let layout = node.final_layout;
    let local_pos = absolute_position(doc, node_id);
    let bbox = Rect {
        x: local_pos.0,
        y: local_pos.1,
        w: layout.size.width,
        h: layout.size.height,
    };

    let has_own_transform = node_has_transform(node);
    let clips_overflow = node_clips_overflow(node);

    let own_opacity = node
        .primary_styles()
        .map(|s| s.get_effects().opacity)
        .unwrap_or(1.0);
    let computed_opacity = parent_opacity * own_opacity;

    let tag = element_data.name.local.to_string().to_ascii_lowercase();
    let element_id = element_data.id.as_ref().map(|a| a.to_string());
    let classes = element_data
        .attr(blitz_dom::local_name!("class"))
        .map(|s| s.split_ascii_whitespace().map(|c| c.to_string()).collect::<Vec<_>>())
        .unwrap_or_default();

    let text = collect_direct_text(doc, node);

    let semantic_id = compute_semantic_id(parent_semantic, &tag, element_id.as_deref(), &classes);

    let mut child_ids: Vec<usize> = Vec::new();
    for &c in &node.children {
        if let Some(cn) = doc.get_node(c) {
            if cn.data.downcast_element().is_some() {
                child_ids.push(c);
            }
        }
    }

    out.push(NodeSnapshot {
        id: node_id,
        semantic_id: semantic_id.clone(),
        tag,
        element_id,
        classes,
        bbox,
        has_own_transform,
        clips_overflow,
        computed_opacity,
        text,
        children: child_ids.clone(),
        parent,
    });

    for c in child_ids {
        walk_node(doc, c, Some(node_id), computed_opacity, &semantic_id, out);
    }
}

/// Cumulative document-space top-left of a node by walking `layout_parent`.
fn absolute_position(doc: &BaseDocument, node_id: usize) -> (f32, f32) {
    let mut x = 0.0f32;
    let mut y = 0.0f32;
    let mut cur = Some(node_id);
    while let Some(id) = cur {
        let Some(node) = doc.get_node(id) else { break };
        x += node.final_layout.location.x;
        y += node.final_layout.location.y;
        cur = node.layout_parent.get();
    }
    (x, y)
}

/// True when this node has a non-default `transform`, `translate`, `rotate`,
/// or `scale` set. We probe the computed style without computing the actual
/// affine — that would require Servo's `style` crate which isn't on
/// crates.io. The boolean is sufficient for Phase 1's wb-b53k detection.
fn node_has_transform(node: &Node) -> bool {
    let Some(styles) = node.primary_styles() else {
        return false;
    };
    let box_ = styles.get_box();
    // The shorthand `transform` is a `Transform(Vec<TransformOperation>)`.
    if !box_.transform.0.is_empty() {
        return true;
    }
    // Individual properties via stringly-typed Debug — opaque to us without
    // the `style` crate, but the `None` variant has a stable Debug repr.
    let translate_str = format!("{:?}", box_.translate);
    if !translate_str.contains("None") {
        return true;
    }
    let rotate_str = format!("{:?}", box_.rotate);
    if !rotate_str.contains("None") {
        return true;
    }
    let scale_str = format!("{:?}", box_.scale);
    if !scale_str.contains("None") {
        return true;
    }
    false
}

/// True when this node clips its descendants — `overflow-x|y: hidden`
/// or `clip`, OR a non-`none` `clip-path`, OR any non-`none`
/// `mask-image` layer. The `mask_image` branch uses Debug-string
/// matching because the Servo computed value is a `Vec<Image>` and we
/// only need a boolean.
fn node_clips_overflow(node: &Node) -> bool {
    use style::values::computed::Overflow;
    use style::values::generics::basic_shape::ClipPath;
    let Some(styles) = node.primary_styles() else {
        return false;
    };
    let ox = styles.clone_overflow_x();
    let oy = styles.clone_overflow_y();
    if matches!(ox, Overflow::Hidden | Overflow::Clip)
        || matches!(oy, Overflow::Hidden | Overflow::Clip)
    {
        return true;
    }
    if styles.get_svg().clip_path != ClipPath::None {
        return true;
    }
    let mask_debug = format!("{:?}", styles.get_svg().mask_image);
    let only_none = mask_debug.matches("None").count() > 0
        && !mask_debug.contains("Url")
        && !mask_debug.contains("Gradient")
        && !mask_debug.contains("ImageSet")
        && !mask_debug.contains("CrossFade")
        && !mask_debug.contains("LightDark");
    if !only_none {
        return true;
    }
    false
}

/// Collect the trimmed concatenation of any direct text-node children.
fn collect_direct_text(doc: &BaseDocument, node: &Node) -> Option<String> {
    let mut buf = String::new();
    for &c in &node.children {
        let Some(cn) = doc.get_node(c) else { continue };
        if let blitz_dom::NodeData::Text(t) = &cn.data {
            buf.push_str(&t.content);
        }
    }
    let trimmed = buf.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

/// xxhash64 of (parent_semantic + tag + #id + .classes). Stable across runs.
fn compute_semantic_id(
    parent_semantic: &str,
    tag: &str,
    element_id: Option<&str>,
    classes: &[String],
) -> String {
    let mut hasher = XxHash64::with_seed(0);
    parent_semantic.hash(&mut hasher);
    tag.hash(&mut hasher);
    if let Some(eid) = element_id {
        eid.hash(&mut hasher);
    }
    for c in classes {
        c.hash(&mut hasher);
    }
    format!("xxh:{:x}", hasher.finish())
}
