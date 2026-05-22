//! Emit — walk a [`RenderGraph`] and produce WGSL fragment shaders + the JSON
//! render-graph spec consumers need to build pipelines.
//!
//! The emit walker threads two things through the AST:
//!
//! - **color** — the in-flight `vec4<f32>` produced by upstream nodes
//! - **uv** — the texture-coordinate expression at which the current node
//!   should be evaluated. Sources sample at `uv`; uv-rewriting transforms
//!   (rotate, scale, scroll, ...) prepend a new uv binding and pass it down
//!   to their input; Hydra-style `modulate` evaluates `rhs` first to compute
//!   a displacement, then resamples `lhs` at the displaced uv.
//!
//! Slot numbering for tween uniforms is determined by a **pre-pass** that
//! walks the AST in canonical [`crate::ast::walk_values`] order and assigns
//! each tween a stable index via its memory address. This decouples slot
//! assignment from emit order — WGSL statements can be emitted in any order
//! (necessary for modulate-resampling), and the same uniform name will
//! resolve regardless. The pre-pass is also what [`crate::ir::lower`] uses
//! to size the uniform table, so the two stay in lockstep by construction.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::ast::{Combinator, Node, Source, Transform};
use crate::ir::{BufferSpec, Pass, RenderGraph, TextureRef, UniformBinding, UniformKind};
use crate::stdlib;
use crate::value::{uniform_slot_name, UniformRef, Value};

/// The complete compile output. Consumers (wavelet) take this and build wgpu
/// pipelines from it; WaveletFx itself never touches the GPU.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmitOutput {
    pub passes: Vec<EmittedPass>,
    pub uniforms: Vec<UniformBinding>,
    pub buffers: Vec<BufferSpec>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmittedPass {
    pub name: String,
    /// A complete WGSL fragment shader. Vertex shader is the consumer's
    /// responsibility (every pass uses a fullscreen-quad vertex shader, so
    /// that stays out of the per-pass body).
    pub wgsl: String,
    pub inputs: Vec<TextureRef>,
    pub output: TextureRef,
    /// The bind-group layout this shader expects. Consumers iterate
    /// `bindings.textures`, resolve each `source` to their own
    /// `wgpu::Texture`, and build a `BindGroup` mechanically — no parsing
    /// the WGSL string, no duplicating the binding convention from
    /// `emit.rs`. The numbers here are exactly the `@group(g) @binding(b)`
    /// attributes in `wgsl`.
    pub bindings: PassBindings,
    /// Pre-effects applied to input textures BEFORE the WGSL shader runs.
    /// wavelet_fx itself doesn't implement these — they're declarative
    /// instructions the consumer (wavelet) executes via Rust crates like
    /// `imageproc`. This keeps non-shader math out of the WGSL emit:
    /// wavelet_fx composes, crates compute. See `PreEffect` for the catalog
    /// of supported pre-pass operations.
    #[serde(default)]
    pub pre_effects: Vec<PreEffect>,
}

/// One declarative pre-pass operation the consumer (wavelet) applies
/// before binding the input to the fragment shader. wavelet_fx's `.blur()`
/// lowers to one of these rather than hand-rolling WGSL inside the
/// per-pass shader — the math lives once, in a place we can swap
/// implementations without touching the DSL surface.
///
/// Two blur variants exist on purpose:
///
/// - `GpuBlur` (preferred) — consumer runs a 2-pass separable Gaussian
///   on the GPU using wavelet_fx's lifted WGSL (`stdlib::blur::SEPARABLE_GAUSSIAN_H`
///   / `..._V`, attributed to Bevy's bloom). Fast, parallel, no
///   CPU→GPU round-trip. Emitted whenever `.blur()` is applied directly
///   to a `Source::Src(N)` so the consumer knows exactly which input
///   texture to operate on.
/// - `CpuBlur` — fallback when GPU isn't available, or when wavelet_fx can't
///   prove the blur applies cleanly to a named input texture (e.g.
///   `.blur()` on a derived chain like `src(0).rotate(...).blur(8)` —
///   that'd require materializing the intermediate, which is the
///   wavelet_fx-IR multi-pass roadmap, not a v0 feature).
///
/// Consumers should attempt `GpuBlur` first and fall back to `CpuBlur`
/// if their renderer doesn't support arbitrary GPU pre-passes. The
/// wavelet_fx emit picks the variant; consumers don't choose.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PreEffect {
    /// Apply a Gaussian blur to the input channel's source texture on
    /// the GPU using the lifted Bevy-bloom separable WGSL. Consumer
    /// runs two fullscreen passes (horizontal then vertical), each
    /// using `stdlib::blur::SEPARABLE_GAUSSIAN_H` / `..._V`, with the
    /// final result replacing the input channel's texture before the
    /// main wavelet_fx fragment shader binds it.
    GpuBlur {
        /// Which input channel this blur applies to. Matches the
        /// `TextureRef::InputChannel(n)` of one of `EmittedPass.inputs`.
        input_channel: u32,
        /// Gaussian σ in pixels. Clamped to a sensible range by the
        /// consumer.
        radius: f32,
    },
    /// Apply a Gaussian blur on the CPU. Same parameters as `GpuBlur`,
    /// different execution. Kept as a fallback for consumers without
    /// general GPU pre-pass support.
    CpuBlur {
        input_channel: u32,
        radius: f32,
    },
}

/// Bind-group layout for a single emitted pass. All bindings live in
/// `@group(0)` for v0; the structure leaves room for multi-group layouts
/// without an API break.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PassBindings {
    pub uniforms: BindingSlot,
    pub textures: Vec<TextureBinding>,
}

/// A single `@group @binding` slot.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct BindingSlot {
    pub group: u32,
    pub binding: u32,
}

/// One texture exposed to the shader, paired with the sampler bound
/// alongside it. `source` tells the consumer which logical texture to
/// supply: a consumer input channel, an intermediate buffer this graph
/// allocates, or the ping-pong "previous frame" of a feedback buffer.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TextureBinding {
    pub source: TextureRef,
    pub texture: BindingSlot,
    pub sampler: BindingSlot,
}

pub fn emit(graph: &RenderGraph) -> EmitOutput {
    let passes = graph
        .passes
        .iter()
        .map(|pass| {
            let (wgsl, bindings, pre_effects) = emit_pass_wgsl(pass, &graph.uniforms);
            EmittedPass {
                name: pass.name.clone(),
                wgsl,
                inputs: pass.inputs.clone(),
                output: pass.output.clone(),
                bindings,
                pre_effects,
            }
        })
        .collect();

    EmitOutput {
        passes,
        uniforms: graph.uniforms.clone(),
        buffers: graph.buffers.clone(),
    }
}

/// Run naga's WGSL frontend over every emitted pass. Returns the first
/// failure as [`crate::diagnostics::Diagnostic::InvalidEmittedWgsl`] with
/// the pass name and naga's own formatted error. Used by
/// [`crate::compile`] to guarantee every shipped WGSL string is at
/// least parseable + type-coherent before the consumer's wgpu submission
/// — wgpu's own error path is far from the cause and harder to debug.
///
/// Cheap: naga is an in-process Rust crate, single-threaded parse runs
/// in microseconds on the small per-pass strings wavelet_fx produces.
pub fn validate_with_naga(out: &EmitOutput) -> Result<(), crate::diagnostics::Diagnostic> {
    for pass in &out.passes {
        if let Err(err) = naga::front::wgsl::parse_str(&pass.wgsl) {
            return Err(crate::diagnostics::Diagnostic::InvalidEmittedWgsl {
                pass_name: pass.name.clone(),
                message: err.emit_to_string(&pass.wgsl),
            });
        }
    }
    Ok(())
}

fn emit_pass_wgsl(
    pass: &Pass,
    uniforms: &[UniformBinding],
) -> (String, PassBindings, Vec<PreEffect>) {
    let mut out = String::new();
    out.push_str(stdlib::PRELUDE);
    out.push('\n');

    out.push_str("struct Uniforms {\n");
    out.push_str("  u_time: f32,\n");
    out.push_str("  u_resolution: vec2<f32>,\n");
    // Tweens, audio, beat, seed, css-prop slots — all f32 scalars driven
    // by the consumer each frame. Time and Resolution are already
    // hardcoded above; Constant doesn't occupy a slot.
    for u in uniforms {
        match &u.kind {
            UniformKind::Time | UniformKind::Resolution | UniformKind::Constant(_) => {}
            UniformKind::Tween(_)
            | UniformKind::AudioRms
            | UniformKind::AudioFftBin(_)
            | UniformKind::Beat
            | UniformKind::Seed
            | UniformKind::CssProp(_) => {
                out.push_str(&format!("  {}: f32,\n", u.name));
            }
        }
    }
    out.push_str("};\n\n");
    out.push_str("@group(0) @binding(0) var<uniform> u: Uniforms;\n");

    let uniforms_slot = BindingSlot { group: 0, binding: 0 };
    let mut texture_bindings = Vec::with_capacity(pass.inputs.len());
    let mut binding = 1u32;
    for t in &pass.inputs {
        let (var, sampler) = match t {
            TextureRef::InputChannel(ch) => (format!("iChannel{}", ch), format!("iChannel{}_sampler", ch)),
            TextureRef::Buffer(name) => (format!("buffer_{}", name), format!("buffer_{}_sampler", name)),
            TextureRef::PrevFrame(name) => (format!("prev_{}", name), format!("prev_{}_sampler", name)),
            TextureRef::SwapchainOrFinal => continue,
        };
        let tex_slot = BindingSlot { group: 0, binding };
        out.push_str(&format!(
            "@group(0) @binding({}) var {}: texture_2d<f32>;\n",
            binding, var
        ));
        binding += 1;
        let smp_slot = BindingSlot { group: 0, binding };
        out.push_str(&format!(
            "@group(0) @binding({}) var {}: sampler;\n",
            binding, sampler
        ));
        binding += 1;
        texture_bindings.push(TextureBinding {
            source: t.clone(),
            texture: tex_slot,
            sampler: smp_slot,
        });
    }
    out.push('\n');

    out.push_str("@fragment\nfn fs_main(@location(0) uv: vec2<f32>) -> @location(0) vec4<f32> {\n");

    let value_names = assign_value_names(&pass.body);
    let current_buffer = match &pass.output {
        TextureRef::Buffer(name) => Some(name.clone()),
        _ => None,
    };
    let mut state = EmitState::new(value_names, current_buffer);
    let mut body = String::new();
    let final_var = emit_node(&pass.body, "uv", &mut state, &mut body);
    out.push_str(&body);
    out.push_str(&format!("  return {};\n", final_var));
    out.push_str("}\n");

    let bindings = PassBindings {
        uniforms: uniforms_slot,
        textures: texture_bindings,
    };
    let pre_effects = std::mem::take(&mut state.pre_effects);
    (out, bindings, pre_effects)
}

/// Walk the AST in canonical [`crate::ast::walk_values`] order and produce a
/// map from each dynamic [`Value`]'s memory address to its WGSL uniform
/// reference. This is the seam that decouples slot assignment from emit
/// order: the emit walker can later visit nodes in any sequence and still
/// resolve every dynamic value to the same name [`crate::ir::lower`] put
/// in the uniform table.
///
/// Tweens get unique indexed slots (`u.u_tween_<n>`) in walk order. Uniform
/// references dedupe by content — multiple `audio_rms()` references share
/// `u.u_audio_rms`. Constants don't get an entry; they inline as literals.
fn assign_value_names(node: &Node) -> HashMap<*const Value, String> {
    let mut names = HashMap::new();
    let mut tween_idx = 0u32;
    let mut uniform_slots: HashMap<UniformRef, String> = HashMap::new();
    crate::ast::walk_values(node, &mut |v| match v {
        Value::Tween(_) => {
            names.insert(v as *const Value, format!("u.u_tween_{}", tween_idx));
            tween_idx += 1;
        }
        Value::Uniform(uref) => {
            let slot = uniform_slots
                .entry(uref.clone())
                .or_insert_with(|| format!("u.{}", uniform_slot_name(uref)))
                .clone();
            names.insert(v as *const Value, slot);
        }
        Value::Const(_) => {}
    });
    names
}

struct EmitState {
    next_color: u32,
    next_uv: u32,
    value_names: HashMap<*const Value, String>,
    /// The name of the buffer this pass writes to, if any. Resolves
    /// `Source::Prev` to `prev_<name>` — only meaningful in passes that
    /// `lower()` has already verified write to a named buffer.
    current_buffer: Option<String>,
    /// Pre-effects accumulated during this pass's emit. wavelet_fx's `.blur()`
    /// and any future "this isn't really a shader" operations push their
    /// instruction here; the consumer (wavelet) reads `EmittedPass.pre_effects`
    /// and dispatches each via the appropriate Rust crate (`imageproc`
    /// for blur, etc.). Cleared per-pass.
    pre_effects: Vec<PreEffect>,
}

impl EmitState {
    fn new(value_names: HashMap<*const Value, String>, current_buffer: Option<String>) -> Self {
        Self {
            next_color: 0,
            next_uv: 0,
            value_names,
            current_buffer,
            pre_effects: Vec::new(),
        }
    }

    fn fresh_color(&mut self) -> String {
        let id = self.next_color;
        self.next_color += 1;
        format!("c{}", id)
    }

    fn fresh_uv(&mut self) -> String {
        let id = self.next_uv;
        self.next_uv += 1;
        format!("uv{}", id)
    }

    /// Resolve a [`Value`] to a WGSL expression. Constants inline as float
    /// literals; tweens and uniform refs look up the slot name assigned
    /// during the pre-pass (see `assign_value_names`).
    fn value(&self, v: &Value) -> String {
        match v {
            Value::Const(c) => format_f32(*c),
            Value::Tween(_) | Value::Uniform(_) => self
                .value_names
                .get(&(v as *const Value))
                .cloned()
                .expect(
                    "every dynamic value should have an assigned slot — pre-pass and emit walks differ",
                ),
        }
    }
}

fn format_f32(c: f32) -> String {
    if !c.is_finite() {
        return "0.0".to_string();
    }
    if c.fract() == 0.0 {
        format!("{:.1}", c)
    } else {
        format!("{}", c)
    }
}

/// Emit WGSL for `node` evaluated at `uv`. Returns the WGSL variable name
/// holding the resulting `vec4<f32>`.
fn emit_node(node: &Node, uv: &str, st: &mut EmitState, body: &mut String) -> String {
    match node {
        Node::Source(s) => emit_source(s, uv, st, body),
        Node::Transform { input, op } => emit_transform(input, op, uv, st, body),
        Node::Combine { lhs, rhs, op } => emit_combine(lhs, rhs, op, uv, st, body),
    }
}

fn emit_source(s: &Source, uv: &str, st: &mut EmitState, body: &mut String) -> String {
    let expr = match s {
        Source::Solid { r, g, b, a } => {
            let r = st.value(r);
            let g = st.value(g);
            let b = st.value(b);
            let a = st.value(a);
            stdlib::sources::expr_solid(&r, &g, &b, &a)
        }
        Source::Noise { scale, offset } => {
            let scale = st.value(scale);
            let offset = st.value(offset);
            stdlib::sources::expr_noise(uv, &scale, &offset)
        }
        Source::Osc { frequency, sync, offset } => {
            let f = st.value(frequency);
            let sy = st.value(sync);
            let o = st.value(offset);
            stdlib::sources::expr_osc(uv, &f, &sy, &o)
        }
        Source::Src { channel } => stdlib::sources::expr_src(*channel, uv),
        Source::Buffer { name } => {
            format!("textureSample(buffer_{0}, buffer_{0}_sampler, {1})", name, uv)
        }
        Source::Prev => {
            // `lower()` rejects prev() in passes without a named output, so
            // this Option is always Some here. Defensive fallback emits a
            // valid expression to keep the shader compilable if invariants
            // ever drift.
            match &st.current_buffer {
                Some(b) => format!("textureSample(prev_{0}, prev_{0}_sampler, {1})", b, uv),
                None => "vec4<f32>(0.0, 0.0, 0.0, 1.0)".to_string(),
            }
        }
        Source::Voronoi { scale, speed, blending } => {
            let scale = st.value(scale);
            let speed = st.value(speed);
            let blending = st.value(blending);
            stdlib::sources::expr_voronoi(uv, &scale, &speed, &blending)
        }
        Source::Gradient { speed } => {
            let speed = st.value(speed);
            stdlib::sources::expr_gradient(uv, &speed)
        }
        Source::Shape { sides, radius, smoothing } => {
            let radius = st.value(radius);
            let smoothing = st.value(smoothing);
            stdlib::sources::expr_shape(uv, *sides, &radius, &smoothing)
        }
        Source::Sphere { radius, smoothing } => {
            let radius = st.value(radius);
            let smoothing = st.value(smoothing);
            stdlib::sdf::expr_sphere(uv, &radius, &smoothing)
        }
        Source::BoxSdf { width, height, smoothing } => {
            let width = st.value(width);
            let height = st.value(height);
            let smoothing = st.value(smoothing);
            stdlib::sdf::expr_box_sdf(uv, &width, &height, &smoothing)
        }
        Source::Torus { radius, thickness, smoothing } => {
            let radius = st.value(radius);
            let thickness = st.value(thickness);
            let smoothing = st.value(smoothing);
            stdlib::sdf::expr_torus(uv, &radius, &thickness, &smoothing)
        }
    };
    let var = st.fresh_color();
    body.push_str(&format!("  let {}: vec4<f32> = {};\n", var, expr));
    var
}

fn emit_transform(
    input: &Node,
    op: &Transform,
    uv: &str,
    st: &mut EmitState,
    body: &mut String,
) -> String {
    // Two kinds of transforms:
    //   1. uv-rewriting: compute a new uv expression from op params, then
    //      recurse into `input` with that uv.
    //   2. color-only: recurse into `input` at the current uv, then apply
    //      the color expression to the returned vec4.
    //
    // Time is read from `u.u_time` for animated rewrites (rotate's speed,
    // scroll's speed_x/y). Consumers bind that uniform; WaveletFx doesn't care
    // how.
    match op {
        Transform::Rotate { angle, speed } => {
            let angle = st.value(angle);
            let speed = st.value(speed);
            let new_uv = st.fresh_uv();
            body.push_str(&format!(
                "  let {}: vec2<f32> = {};\n",
                new_uv,
                stdlib::transforms::expr_rotate_uv(uv, &angle, &speed, "u.u_time")
            ));
            emit_node(input, &new_uv, st, body)
        }
        Transform::Scale { amount, x, y } => {
            let amount = st.value(amount);
            let x = st.value(x);
            let y = st.value(y);
            let new_uv = st.fresh_uv();
            body.push_str(&format!(
                "  let {}: vec2<f32> = {};\n",
                new_uv,
                stdlib::transforms::expr_scale_uv(uv, &amount, &x, &y)
            ));
            emit_node(input, &new_uv, st, body)
        }
        Transform::Scroll { x, y, speed_x, speed_y } => {
            let x = st.value(x);
            let y = st.value(y);
            let sx = st.value(speed_x);
            let sy = st.value(speed_y);
            let new_uv = st.fresh_uv();
            body.push_str(&format!(
                "  let {}: vec2<f32> = ({} + vec2<f32>({} + u.u_time * {}, {} + u.u_time * {}));\n",
                new_uv, uv, x, sx, y, sy
            ));
            emit_node(input, &new_uv, st, body)
        }
        Transform::Pixelate { x, y } => {
            let x = st.value(x);
            let y = st.value(y);
            let new_uv = st.fresh_uv();
            body.push_str(&format!(
                "  let {0}: vec2<f32> = (floor({1} * vec2<f32>({2}, {3})) + vec2<f32>(0.5)) / vec2<f32>({2}, {3});\n",
                new_uv, uv, x, y
            ));
            emit_node(input, &new_uv, st, body)
        }
        Transform::Blur { radius } => {
            // wavelet_fx doesn't compute blur inside the per-pass fragment
            // shader. It records a pre-pass instruction ("blur input
            // channel N by radius R") and passes through the chain. The
            // consumer (wavelet) runs the real Gaussian — preferably as a
            // 2-pass separable Gaussian on the GPU using the lifted
            // Bevy-bloom WGSL in `stdlib::blur`, falling back to the
            // CPU `gaussian_rgba` path when GPU pre-passes aren't
            // wired. WaveletFx composes shaders + DSL; non-shader math
            // comes from crates / lifted code.
            //
            // We honor `.blur()` only when its direct input is `src(N)`.
            // Blur on a derived chain (`src(0).rotate(...).blur(8)`) is
            // a no-op + warning today — proper support requires wavelet_fx
            // IR multi-pass synthesis, which is roadmapped but not in
            // this phase.
            match input {
                Node::Source(Source::Src { channel }) => {
                    let r_v = st.value(radius);
                    // Constants emit as float literals; tweens / uniform
                    // refs stay as uniform names and can't drive a
                    // pre-pass parameter (which is fixed per render).
                    // For non-constant radii we fall back to a sensible
                    // default and document the limitation.
                    let radius_f32 = r_v.parse::<f32>().unwrap_or_else(|_| {
                        eprintln!(
                            "wavelet_fx warning: .blur(<dynamic value>) — animated radius \
                             not yet supported; using 8.0px"
                        );
                        8.0
                    });
                    // GpuBlur is the preferred path. Consumers without
                    // GPU pre-pass support are expected to translate it
                    // into a CpuBlur themselves at dispatch time.
                    st.pre_effects.push(PreEffect::GpuBlur {
                        input_channel: *channel,
                        radius: radius_f32,
                    });
                    return emit_node(input, uv, st, body);
                }
                _ => {
                    eprintln!(
                        "wavelet_fx warning: .blur() only works directly on src(N) sources \
                         today (got a derived chain). Treated as no-op; see RECIPES.md."
                    );
                    return emit_node(input, uv, st, body);
                }
            }
        }
        Transform::Repeat { x, y, offset_x, offset_y } => {
            let x = st.value(x);
            let y = st.value(y);
            let ox = st.value(offset_x);
            let oy = st.value(offset_y);
            let new_uv = st.fresh_uv();
            body.push_str(&format!(
                "  let {}: vec2<f32> = fract({} * vec2<f32>({}, {}) + vec2<f32>({}, {}));\n",
                new_uv, uv, x, y, ox, oy
            ));
            emit_node(input, &new_uv, st, body)
        }
        // Color-only transforms.
        Transform::Color { r, g, b, a } => {
            let cin = emit_node(input, uv, st, body);
            let r = st.value(r);
            let g = st.value(g);
            let b = st.value(b);
            let a = st.value(a);
            let var = st.fresh_color();
            body.push_str(&format!(
                "  let {}: vec4<f32> = {};\n",
                var,
                stdlib::transforms::expr_color(&cin, &r, &g, &b, &a)
            ));
            var
        }
        Transform::Brightness { amount } => simple_color(input, uv, st, body, amount, |c, a| {
            stdlib::transforms::expr_brightness(c, a)
        }),
        Transform::Contrast { amount } => simple_color(input, uv, st, body, amount, |c, a| {
            stdlib::transforms::expr_contrast(c, a)
        }),
        Transform::Invert { amount } => simple_color(input, uv, st, body, amount, |c, a| {
            stdlib::transforms::expr_invert(c, a)
        }),
        // Not yet implemented as proper transforms. Pass color through; uv is
        // unchanged. Kaleid + the rest land in their own follow-up.
        Transform::Kaleid { .. }
        | Transform::Posterize { .. }
        | Transform::Thresh { .. }
        | Transform::Luma { .. }
        | Transform::Saturate { .. }
        | Transform::Hue { .. } => emit_node(input, uv, st, body),
    }
}

fn simple_color<F>(
    input: &Node,
    uv: &str,
    st: &mut EmitState,
    body: &mut String,
    amount: &Value,
    expr: F,
) -> String
where
    F: FnOnce(&str, &str) -> String,
{
    let cin = emit_node(input, uv, st, body);
    let amount = st.value(amount);
    let var = st.fresh_color();
    body.push_str(&format!(
        "  let {}: vec4<f32> = {};\n",
        var,
        expr(&cin, &amount)
    ));
    var
}

fn emit_combine(
    lhs: &Node,
    rhs: &Node,
    op: &Combinator,
    uv: &str,
    st: &mut EmitState,
    body: &mut String,
) -> String {
    // Hydra-style modulate-family combinators are uv-displacement: evaluate
    // rhs at the current uv, derive a new uv from rhs.rg, then resample lhs
    // at the displaced uv. Color-mixing combinators (add, mult, blend, diff,
    // mask) evaluate both at the same uv and combine the resulting colors.
    match op {
        Combinator::Modulate { amount } => {
            let crhs = emit_node(rhs, uv, st, body);
            let amount = st.value(amount);
            let new_uv = st.fresh_uv();
            body.push_str(&format!(
                "  let {}: vec2<f32> = ({} + ({}.rg - vec2<f32>(0.5)) * {});\n",
                new_uv, uv, crhs, amount
            ));
            emit_node(lhs, &new_uv, st, body)
        }
        Combinator::ModulateScale { multiple, offset } => {
            let crhs = emit_node(rhs, uv, st, body);
            let multiple = st.value(multiple);
            let offset = st.value(offset);
            let new_uv = st.fresh_uv();
            body.push_str(&format!(
                "  let {0}: vec2<f32> = ((({1}) - vec2<f32>(0.5)) / vec2<f32>(({2}.r - 0.5) * {3} + {4}) + vec2<f32>(0.5));\n",
                new_uv, uv, crhs, multiple, offset
            ));
            emit_node(lhs, &new_uv, st, body)
        }
        Combinator::ModulateRotate { multiple, offset } => {
            let crhs = emit_node(rhs, uv, st, body);
            let multiple = st.value(multiple);
            let offset = st.value(offset);
            let new_uv = st.fresh_uv();
            body.push_str(&format!(
                "  let {}: vec2<f32> = (rotate2d({} - vec2<f32>(0.5), ({}.r - 0.5) * {} + {}) + vec2<f32>(0.5));\n",
                new_uv, uv, crhs, multiple, offset
            ));
            emit_node(lhs, &new_uv, st, body)
        }
        Combinator::ModulatePixelate { multiple, offset } => {
            let crhs = emit_node(rhs, uv, st, body);
            let multiple = st.value(multiple);
            let offset = st.value(offset);
            let new_uv = st.fresh_uv();
            body.push_str(&format!(
                "  let {0}: vec2<f32> = (floor({1} * ({2}.r * {3} + {4})) + vec2<f32>(0.5)) / ({2}.r * {3} + {4});\n",
                new_uv, uv, crhs, multiple, offset
            ));
            emit_node(lhs, &new_uv, st, body)
        }
        // ModulateHue is a colour-space combinator, not uv-displacement —
        // evaluate both sides at the same uv, then mix.
        Combinator::ModulateHue { amount } => {
            color_combine(lhs, rhs, uv, st, body, Some(amount), |l, r, a| {
                stdlib::combinators::expr_modulate(l, r, a.unwrap())
            })
        }
        Combinator::Add { amount } => {
            color_combine(lhs, rhs, uv, st, body, Some(amount), |l, r, a| {
                stdlib::combinators::expr_add(l, r, a.unwrap())
            })
        }
        Combinator::Mult { amount } => {
            color_combine(lhs, rhs, uv, st, body, Some(amount), |l, r, a| {
                stdlib::combinators::expr_mult(l, r, a.unwrap())
            })
        }
        Combinator::Blend { amount } => {
            color_combine(lhs, rhs, uv, st, body, Some(amount), |l, r, a| {
                stdlib::combinators::expr_blend(l, r, a.unwrap())
            })
        }
        Combinator::Diff => color_combine(lhs, rhs, uv, st, body, None, |l, r, _| {
            stdlib::combinators::expr_diff(l, r)
        }),
        Combinator::Mask => color_combine(lhs, rhs, uv, st, body, None, |l, r, _| {
            stdlib::combinators::expr_mask(l, r)
        }),
        Combinator::SmoothUnion { k } => {
            color_combine(lhs, rhs, uv, st, body, Some(k), |l, r, k| {
                stdlib::sdf::expr_smooth_union(l, r, k.unwrap())
            })
        }
        Combinator::SmoothIntersect { k } => {
            color_combine(lhs, rhs, uv, st, body, Some(k), |l, r, k| {
                stdlib::sdf::expr_smooth_intersect(l, r, k.unwrap())
            })
        }
    }
}

fn color_combine<F>(
    lhs: &Node,
    rhs: &Node,
    uv: &str,
    st: &mut EmitState,
    body: &mut String,
    amount: Option<&Value>,
    expr: F,
) -> String
where
    F: FnOnce(&str, &str, Option<&str>) -> String,
{
    let clhs = emit_node(lhs, uv, st, body);
    let crhs = emit_node(rhs, uv, st, body);
    let amount = amount.map(|v| st.value(v));
    let var = st.fresh_color();
    body.push_str(&format!(
        "  let {}: vec4<f32> = {};\n",
        var,
        expr(&clhs, &crhs, amount.as_deref())
    ));
    var
}
