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

use crate::query::glyph_run::GlyphRunData;
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

/// Resolved flex main axis. `RowReverse` collapses to `Row` and
/// `ColumnReverse` to `Column` — for layout-axis coherence lints we
/// only care which axis the author chose, not the visual ordering.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FlexAxis {
    /// `flex-direction: row | row-reverse` — children laid out along X.
    Row,
    /// `flex-direction: column | column-reverse` — children laid out along Y.
    Column,
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
    /// Resolved 2D affine for this element's own CSS transform, in
    /// kurbo-style column-major coefficient order [a, b, c, d, tx, ty]
    /// (augmented matrix [[a,c,tx],[b,d,ty],[0,0,1]]). None = identity.
    /// `transform-origin` is already baked in by Blitz/Stylo's resolver,
    /// matching what the painter applies — to apply at a query site,
    /// translate the point to be relative to the element's bbox origin,
    /// multiply, then translate back.
    pub transform: Option<[f32; 6]>,
    /// True when this node's computed style clips its descendants —
    /// `overflow-x|y: hidden|clip`, OR `clip-path` set to anything other
    /// than `none`, OR a non-`none` `mask-image`. Consumers (e.g. the
    /// `glyph-clip` lint rule) treat this as "this node's bbox is a
    /// clip rectangle for everything beneath it."
    pub clips_overflow: bool,
    /// Declared layout axis for this node, if it is a flex container.
    /// `Some(Column)` for `display: flex; flex-direction: column` (and
    /// `column-reverse`); `Some(Row)` for `row` / `row-reverse`. `None`
    /// when the element is not a flex container, so layout-axis lints
    /// can skip non-flex nodes cheaply. Captured here (rather than via
    /// computed-style queries inside the rule) because Stylo state
    /// isn't carried in `FrameSnapshot` — only this resolved summary.
    #[serde(default)]
    pub flex_axis: Option<FlexAxis>,
    /// Computed `opacity` rolled up through ancestors.
    pub computed_opacity: f32,
    /// Computed font-size in CSS px (= device px at 1.0 DPR, which our
    /// render path uses). 0.0 when the element has no font-size cascade
    /// (root inheritor, etc.); skip in lint.
    pub computed_font_size_px: f32,
    /// Concatenated text content of any direct text children, trimmed.
    pub text: Option<String>,
    /// Child node ids — see `FrameSnapshot.nodes_by_id` to resolve.
    pub children: Vec<usize>,
    /// Parent node id, if any (root has none).
    pub parent: Option<usize>,
    /// Per-element shaped-glyph ink data, captured from Parley's
    /// inline-layout output. `None` when the element has no
    /// `inline_layout_data` (non-inline-root, or text routed through a
    /// non-Parley path). Coordinates are element-local (relative to
    /// the element's content-box origin, which the lint rule treats as
    /// the bbox origin). Skipped from serde because the data is bulky,
    /// embedded-rendered, and only used by in-process consumers
    /// (`glyph-clip` lint).
    #[serde(skip)]
    pub glyph_run: Option<GlyphRunData>,
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

/// Depth-first walk; captures the layout-space bbox, cumulative opacity,
/// per-node CSS transform matrix, and stable `semantic_id` for every
/// element. The transform is the element's own resolved 2D affine
/// (matching `blitz_dom::resolve_2d_transform` — the function the
/// painter uses); callers that need cumulative ancestor-chain
/// composition do it at the query site.
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

    let transform = node_transform(node);
    let clips_overflow = node_clips_overflow(node);
    let computed_font_size_px = node_computed_font_size_px(node);
    let flex_axis = node_flex_axis(node);

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

    let glyph_run = element_data
        .inline_layout_data
        .as_ref()
        .and_then(|tl| GlyphRunData::from_layout(&tl.layout));

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
        transform,
        clips_overflow,
        computed_opacity,
        computed_font_size_px,
        text,
        children: child_ids.clone(),
        parent,
        glyph_run,
        flex_axis,
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

/// Resolved 2D affine for this node's own CSS transform, in kurbo-style
/// column-major coefficient order `[a, b, c, d, tx, ty]`. Returns `None`
/// when the node has no styles or the resolved transform is identity.
/// Delegates to `blitz_dom::resolve_2d_transform` — the exact API the
/// Blitz painter calls (vendor/blitz-paint/src/render.rs:578) — so the
/// matrix we expose matches what the renderer paints with. The reference
/// box uses the element's `final_layout.size` at scale 1.0 (offline
/// render path is DPR 1.0); `transform-origin` is baked into the returned
/// affine by the resolver.
fn node_transform(node: &Node) -> Option<[f32; 6]> {
    use style::values::computed::CSSPixelLength;
    let styles = node.primary_styles()?;
    let size = node.final_layout.size;
    let reference_box = euclid::default::Rect::new(
        euclid::default::Point2D::new(CSSPixelLength::new(0.0), CSSPixelLength::new(0.0)),
        euclid::default::Size2D::new(
            CSSPixelLength::new(size.width),
            CSSPixelLength::new(size.height),
        ),
    );
    let affine = blitz_dom::resolve_2d_transform(styles.get_box(), reference_box, 1.0)?;
    let c = affine.as_coeffs();
    Some([
        c[0] as f32,
        c[1] as f32,
        c[2] as f32,
        c[3] as f32,
        c[4] as f32,
        c[5] as f32,
    ])
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
    // Stylo prints unset mask-image as `OwnedList([none])` (lowercase)
    // — earlier match-on-"None" logic was inverted. Be explicit: a mask
    // clips when the Debug repr contains any concrete image variant
    // (Url, Gradient, ImageSet, CrossFade, LightDark).
    let mask_debug = format!("{:?}", styles.get_svg().mask_image);
    if mask_debug.contains("Url")
        || mask_debug.contains("Gradient")
        || mask_debug.contains("ImageSet")
        || mask_debug.contains("CrossFade")
        || mask_debug.contains("LightDark")
    {
        return true;
    }
    false
}

/// Resolved flex axis for `node`. `Some(Row | Column)` when the node's
/// computed `display` is a flex container (`flex` / `inline-flex`),
/// reflecting `flex-direction` (with reverse collapsed onto the base
/// axis). `None` for any non-flex display (including grid, block, etc.)
/// — those don't carry a single dominant child-flow axis we can
/// validate against in the `layout-axis-coherence` lint.
fn node_flex_axis(node: &Node) -> Option<FlexAxis> {
    use style::properties::longhands::flex_direction::computed_value::T as FlexDirection;
    use style::values::specified::box_::DisplayInside;
    let styles = node.primary_styles()?;
    let display = styles.get_box().clone_display();
    if !matches!(display.inside(), DisplayInside::Flex) {
        return None;
    }
    let dir = styles.clone_flex_direction();
    Some(match dir {
        FlexDirection::Row | FlexDirection::RowReverse => FlexAxis::Row,
        FlexDirection::Column | FlexDirection::ColumnReverse => FlexAxis::Column,
    })
}

/// Computed font-size in CSS px. Returns 0.0 when the element has no
/// primary styles (no cascade ran). At 1.0 DPR — which the offline
/// render path uses — CSS px equals device px.
fn node_computed_font_size_px(node: &Node) -> f32 {
    let Some(styles) = node.primary_styles() else {
        return 0.0;
    };
    styles.get_font().clone_font_size().computed_size().px()
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
