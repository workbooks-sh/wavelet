//! AST — the structural form of a WaveletFx program before lowering to IR.
//!
//! A program is one or more [`Output`]s. Each output terminates a [`Node`]
//! built from a [`Source`], zero or more [`Transform`]s, and zero or more
//! [`Combinator`] mixes with sibling chains.
//!
//! Every tweenable parameter is a [`Value`], which can be either a
//! constant or an Animato `Tween<f32>`. Discrete parameters (channel index,
//! polygon sides, posterize bin count) stay as `u32` — these are not sensibly
//! interpolated and would only confuse the IR walker.

use serde::{Deserialize, Serialize};

use crate::value::Value;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Node {
    Source(Source),
    Transform {
        input: Box<Node>,
        op: Transform,
    },
    Combine {
        lhs: Box<Node>,
        rhs: Box<Node>,
        op: Combinator,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Source {
    Osc {
        frequency: Value,
        sync: Value,
        offset: Value,
    },
    Noise {
        scale: Value,
        offset: Value,
    },
    Voronoi {
        scale: Value,
        speed: Value,
        blending: Value,
    },
    Solid {
        r: Value,
        g: Value,
        b: Value,
        a: Value,
    },
    Gradient {
        speed: Value,
    },
    Shape {
        sides: u32,
        radius: Value,
        smoothing: Value,
    },
    /// 2D circle SDF rendered as a soft-edged disc. `radius` in
    /// uv-units (0..1). `smoothing` widens the antialiased edge ramp.
    /// Math ported from sdfu / IQ's analytic SDF catalog — distance
    /// from center to perimeter, smoothstepped at the edge.
    Sphere {
        radius: Value,
        smoothing: Value,
    },
    /// 2D box SDF (axis-aligned rounded rect, corner radius 0).
    /// `width` and `height` are half-extents in uv-units.
    BoxSdf {
        width: Value,
        height: Value,
        smoothing: Value,
    },
    /// 2D torus / annulus — region between two concentric circles.
    /// `radius` is the centerline radius, `thickness` is half the band
    /// width. Both in uv-units.
    Torus {
        radius: Value,
        thickness: Value,
        smoothing: Value,
    },
    Src {
        channel: u32,
    },
    /// Sample from a named intermediate buffer written by another pass in the
    /// same composition. The referenced buffer must be declared (via
    /// `output_to(name)`) earlier in the composition than the pass using it.
    Buffer {
        name: String,
    },
    /// Previous-frame texture for the current pass's output buffer. Only
    /// meaningful inside a pass that writes to a named buffer — that buffer
    /// gets `feedback: true` in its `BufferSpec` and the consumer must
    /// allocate two textures and ping-pong between them each frame.
    Prev,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Transform {
    Rotate { angle: Value, speed: Value },
    Scale { amount: Value, x: Value, y: Value },
    Kaleid { sides: u32 },
    Pixelate { x: Value, y: Value },
    /// Multi-tap Gaussian blur. `radius` is in pixels (clamped 1..=32 at
    /// emit time). Implements true neighbor sampling — `mix()` of nearby
    /// pixels — not UV displacement. For a watery wobble or organic
    /// dissolve, prefer `Combinator::Modulate` with a low-frequency noise
    /// source and small amount.
    Blur { radius: Value },
    Repeat {
        x: Value,
        y: Value,
        offset_x: Value,
        offset_y: Value,
    },
    Scroll {
        x: Value,
        y: Value,
        speed_x: Value,
        speed_y: Value,
    },
    Color {
        r: Value,
        g: Value,
        b: Value,
        a: Value,
    },
    Brightness { amount: Value },
    Contrast { amount: Value },
    Invert { amount: Value },
    Posterize { bins: Value },
    Thresh { threshold: Value, tolerance: Value },
    Luma { threshold: Value, tolerance: Value },
    Saturate { amount: Value },
    Hue { amount: Value },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Combinator {
    Add { amount: Value },
    Mult { amount: Value },
    Blend { amount: Value },
    Diff,
    Mask,
    Modulate { amount: Value },
    ModulateScale { multiple: Value, offset: Value },
    ModulatePixelate { multiple: Value, offset: Value },
    ModulateRotate { multiple: Value, offset: Value },
    ModulateHue { amount: Value },
    /// IQ-style smooth-min union of two SDF-shaped sources. Bigger `k`
    /// = wider blend zone. The result is still a single distance
    /// field; chain further SDF combinators or finish with `.output`
    /// to render. Math ported from sdfu / IQ.
    SmoothUnion { k: Value },
    /// IQ-style smooth-max intersection. Bigger `k` = wider blend.
    SmoothIntersect { k: Value },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Output {
    pub chain: Node,
    pub buffer: Option<String>,
}

// ---- Traversal --------------------------------------------------------
//
// Per-enum `values()` accessors return every tweenable parameter in source
// order. `walk_values` drives a depth-first walk over a [`Node`] so the IR
// lowering pass can collect tweens without duplicating the tree shape.

impl Source {
    pub fn values(&self) -> Vec<&Value> {
        match self {
            Source::Osc { frequency, sync, offset } => vec![frequency, sync, offset],
            Source::Noise { scale, offset } => vec![scale, offset],
            Source::Voronoi { scale, speed, blending } => vec![scale, speed, blending],
            Source::Solid { r, g, b, a } => vec![r, g, b, a],
            Source::Gradient { speed } => vec![speed],
            Source::Shape { sides: _, radius, smoothing } => vec![radius, smoothing],
            Source::Sphere { radius, smoothing } => vec![radius, smoothing],
            Source::BoxSdf { width, height, smoothing } => vec![width, height, smoothing],
            Source::Torus { radius, thickness, smoothing } => vec![radius, thickness, smoothing],
            Source::Src { .. } | Source::Buffer { .. } | Source::Prev => vec![],
        }
    }
}

impl Transform {
    pub fn values(&self) -> Vec<&Value> {
        match self {
            Transform::Rotate { angle, speed } => vec![angle, speed],
            Transform::Scale { amount, x, y } => vec![amount, x, y],
            Transform::Kaleid { .. } => vec![],
            Transform::Pixelate { x, y } => vec![x, y],
            Transform::Blur { radius } => vec![radius],
            Transform::Repeat { x, y, offset_x, offset_y } => vec![x, y, offset_x, offset_y],
            Transform::Scroll { x, y, speed_x, speed_y } => vec![x, y, speed_x, speed_y],
            Transform::Color { r, g, b, a } => vec![r, g, b, a],
            Transform::Brightness { amount }
            | Transform::Contrast { amount }
            | Transform::Invert { amount }
            | Transform::Saturate { amount }
            | Transform::Hue { amount } => vec![amount],
            Transform::Posterize { bins } => vec![bins],
            Transform::Thresh { threshold, tolerance }
            | Transform::Luma { threshold, tolerance } => vec![threshold, tolerance],
        }
    }
}

impl Combinator {
    pub fn values(&self) -> Vec<&Value> {
        match self {
            Combinator::Add { amount }
            | Combinator::Mult { amount }
            | Combinator::Blend { amount }
            | Combinator::Modulate { amount }
            | Combinator::ModulateHue { amount } => vec![amount],
            Combinator::Diff | Combinator::Mask => vec![],
            Combinator::ModulateScale { multiple, offset }
            | Combinator::ModulatePixelate { multiple, offset }
            | Combinator::ModulateRotate { multiple, offset } => vec![multiple, offset],
            Combinator::SmoothUnion { k } | Combinator::SmoothIntersect { k } => vec![k],
        }
    }
}

/// Visit every [`Value`] reachable from `node` in depth-first, source order.
/// Used by the IR lowering pass to extract tweens into uniform slots without
/// rewriting the AST.
pub fn walk_values<F: FnMut(&Value)>(node: &Node, visit: &mut F) {
    match node {
        Node::Source(s) => {
            for v in s.values() {
                visit(v);
            }
        }
        Node::Transform { input, op } => {
            walk_values(input, visit);
            for v in op.values() {
                visit(v);
            }
        }
        Node::Combine { lhs, rhs, op } => {
            walk_values(lhs, visit);
            walk_values(rhs, visit);
            for v in op.values() {
                visit(v);
            }
        }
    }
}
