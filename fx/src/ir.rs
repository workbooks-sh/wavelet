//! IR — the lowered form of a WaveletFx program. AST nodes are flattened into a
//! linear list of passes, each with input/output texture refs and a uniform
//! table. The IR is the thing [`crate::emit`] walks to produce WGSL.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use crate::builder::Composition;
use crate::diagnostics::Diagnostic;
use crate::value::{uniform_slot_name, UniformRef, Value};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RenderGraph {
    pub passes: Vec<Pass>,
    pub uniforms: Vec<UniformBinding>,
    pub buffers: Vec<BufferSpec>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Pass {
    pub name: String,
    pub inputs: Vec<TextureRef>,
    pub output: TextureRef,
    /// The body kept in AST form for v0 — `emit` walks this directly. When
    /// separable-kernel detection arrives the IR will gain a flattened op
    /// sequence with explicit buffer reads; today the AST is enough.
    pub body: crate::ast::Node,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UniformBinding {
    pub name: String,
    pub kind: UniformKind,
}

impl UniformBinding {
    /// Sample this uniform at absolute frame time `secs`. Returns `Some(v)`
    /// for scalar uniforms (tweens, constants) and `None` for uniforms the
    /// consumer fills itself (time, resolution, seed, audio, beat, css-prop).
    ///
    /// This is the canonical consumer pattern: per frame, walk
    /// [`EmitOutput::uniforms`], call `sample_at(frame_secs)` for each, and
    /// write the result into the corresponding uniform buffer slot.
    ///
    /// `Tween::seek` is normalized to `[0, 1]` of the tween's own duration —
    /// `secs` is converted to that progress before sampling. Delay, loops,
    /// and ping-pong reversal are not yet honored; for those, use the tween
    /// directly via `UniformKind::Tween(_)`.
    pub fn sample_at(&self, secs: f32) -> Option<f32> {
        match &self.kind {
            UniformKind::Tween(tween) => {
                if tween.duration <= 0.0 {
                    return Some(tween.value());
                }
                let mut tw = tween.clone();
                tw.seek((secs / tween.duration).clamp(0.0, 1.0));
                Some(tw.value())
            }
            UniformKind::Constant(c) => Some(*c),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum UniformKind {
    /// Frame time in seconds. Bound by wavelet from `frame_index / fps`. This is
    /// the same `t` consumers pass to `Tween::seek` / `Timeline::seek_abs`, so
    /// the timeline model is shared end-to-end with Animato.
    Time,
    /// `vec2<f32>` viewport size in pixels.
    Resolution,
    /// Deterministic seed = `comp_hash ^ frame_index`.
    Seed,
    /// RMS of the audio mixer's running window.
    AudioRms,
    /// Specific FFT bin index.
    AudioFftBin(u32),
    /// Beat phase 0..1, or -1 if no beat track.
    Beat,
    /// Value of a CSS custom property on the host element (Animato-driven).
    CssProp(String),
    /// An Animato tween. The consumer calls `tween.seek(frame_secs)` then
    /// `tween.value()` each frame and writes the result to this slot. Carries
    /// the tween itself so the render-graph spec is fully self-describing.
    Tween(animato::Tween<f32>),
    /// Inline numeric constant — does not occupy a uniform slot.
    Constant(f32),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BufferSpec {
    pub name: String,
    pub width: BufferDim,
    pub height: BufferDim,
    pub format: TextureFormat,
    /// `true` for the ping-pong buffer backing `prev`. The consumer must
    /// allocate two textures with the same descriptor and alternate which
    /// one is the read target each frame.
    pub feedback: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum BufferDim {
    /// Same as the host viewport.
    Viewport,
    /// Viewport scaled by `factor` (e.g. 0.5 for half-res bloom).
    ViewportScaled(f32),
    /// Fixed pixel dimensions.
    Fixed(u32),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TextureFormat {
    Rgba8UnormSrgb,
    Rgba16Float,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TextureRef {
    /// One of the consumer's input textures (e.g. Vello's rasterized frame).
    InputChannel(u32),
    /// A named intermediate buffer in this graph.
    Buffer(String),
    /// The final destination — what the consumer encodes / displays.
    SwapchainOrFinal,
    /// Previous-frame texture for the named feedback buffer.
    PrevFrame(String),
}

/// Lower a [`Composition`] to a [`RenderGraph`]. Produces one [`Pass`] per
/// output in declaration order; buffer references must be backwards-only.
///
/// The lowering walks every [`Value`](crate::value::Value) in every output's
/// chain in source order. Constants stay inline (they become WGSL literals at
/// emit time); Animato tweens are extracted into [`UniformBinding`]s with
/// stable names `u_tween_0`, `u_tween_1`, ... in that same depth-first order
/// `crate::emit` uses, so the two passes agree on slot numbering without
/// sharing state.
pub fn lower(composition: &Composition) -> Result<RenderGraph, Diagnostic> {
    if composition.outputs.is_empty() {
        return Err(Diagnostic::InvalidComposition(
            "composition has no outputs".into(),
        ));
    }

    let mut uniforms = vec![
        UniformBinding {
            name: "u_time".into(),
            kind: UniformKind::Time,
        },
        UniformBinding {
            name: "u_resolution".into(),
            kind: UniformKind::Resolution,
        },
    ];

    // Dynamic Values (tweens + per-frame uniform refs) are collected across
    // ALL outputs so every pass shares one Uniforms layout. Tweens get
    // unique indexed slots in walk order; uniform refs are deduped by
    // content (multiple `audio_rms()` references share a single slot).
    let mut tween_idx = 0u32;
    let mut seen_uniforms: HashSet<UniformRef> = HashSet::new();
    for output in &composition.outputs {
        crate::ast::walk_values(&output.chain, &mut |value| match value {
            Value::Tween(t) => {
                uniforms.push(UniformBinding {
                    name: format!("u_tween_{}", tween_idx),
                    kind: UniformKind::Tween(t.clone()),
                });
                tween_idx += 1;
            }
            Value::Uniform(uref) => {
                if seen_uniforms.insert(uref.clone()) {
                    uniforms.push(UniformBinding {
                        name: uniform_slot_name(uref),
                        kind: uniform_ref_to_kind(uref),
                    });
                }
            }
            Value::Const(_) => {}
        });
    }

    let mut passes = Vec::with_capacity(composition.outputs.len());
    let mut buffers = Vec::new();
    let mut declared_buffers: HashSet<String> = HashSet::new();
    let mut seen_final = false;

    for output in &composition.outputs {
        // Backwards-only buffer references: a pass can only sample buffers
        // that were declared by an earlier output.
        validate_buffer_refs(&output.chain, &declared_buffers)?;

        let (name, output_ref) = match &output.buffer {
            Some(b) => {
                if !declared_buffers.insert(b.clone()) {
                    return Err(Diagnostic::InvalidComposition(format!(
                        "duplicate buffer output '{}'",
                        b
                    )));
                }
                (b.clone(), TextureRef::Buffer(b.clone()))
            }
            None => {
                if seen_final {
                    return Err(Diagnostic::InvalidComposition(
                        "composition has more than one final output — call output() at most once"
                            .into(),
                    ));
                }
                seen_final = true;
                ("main".to_string(), TextureRef::SwapchainOrFinal)
            }
        };

        let uses_prev = node_uses_prev(&output.chain);
        if uses_prev && output.buffer.is_none() {
            return Err(Diagnostic::InvalidComposition(
                "prev() is only valid inside a pass that writes to a named buffer (use output_to())".into(),
            ));
        }

        let inputs = collect_inputs(&output.chain, output.buffer.as_deref());

        if let Some(bname) = &output.buffer {
            buffers.push(BufferSpec {
                name: bname.clone(),
                width: BufferDim::Viewport,
                height: BufferDim::Viewport,
                format: TextureFormat::Rgba8UnormSrgb,
                feedback: uses_prev,
            });
        }

        passes.push(Pass {
            name,
            inputs,
            output: output_ref,
            body: output.chain.clone(),
        });
    }

    Ok(RenderGraph {
        passes,
        uniforms,
        buffers,
    })
}

fn uniform_ref_to_kind(u: &UniformRef) -> UniformKind {
    match u {
        UniformRef::AudioRms => UniformKind::AudioRms,
        UniformRef::AudioFftBin(n) => UniformKind::AudioFftBin(*n),
        UniformRef::Beat => UniformKind::Beat,
        UniformRef::Seed => UniformKind::Seed,
        UniformRef::CssProp(name) => UniformKind::CssProp(name.clone()),
    }
}

fn validate_buffer_refs(
    node: &crate::ast::Node,
    declared: &HashSet<String>,
) -> Result<(), Diagnostic> {
    use crate::ast::{Node, Source};
    match node {
        Node::Source(Source::Buffer { name }) => {
            if !declared.contains(name) {
                Err(Diagnostic::InvalidComposition(format!(
                    "buffer '{}' is not declared before this pass — declare it earlier with output_to('{}')",
                    name, name
                )))
            } else {
                Ok(())
            }
        }
        Node::Source(_) => Ok(()),
        Node::Transform { input, .. } => validate_buffer_refs(input, declared),
        Node::Combine { lhs, rhs, .. } => {
            validate_buffer_refs(lhs, declared)?;
            validate_buffer_refs(rhs, declared)
        }
    }
}

fn node_uses_prev(node: &crate::ast::Node) -> bool {
    use crate::ast::{Node, Source};
    match node {
        Node::Source(Source::Prev) => true,
        Node::Source(_) => false,
        Node::Transform { input, .. } => node_uses_prev(input),
        Node::Combine { lhs, rhs, .. } => node_uses_prev(lhs) || node_uses_prev(rhs),
    }
}

fn collect_inputs(node: &crate::ast::Node, current_buffer: Option<&str>) -> Vec<TextureRef> {
    let mut inputs = Vec::new();
    let mut seen = HashSet::new();
    collect_inputs_inner(node, current_buffer, &mut inputs, &mut seen);
    inputs.sort_by_key(|t| match t {
        TextureRef::InputChannel(n) => (0, *n as i32, String::new()),
        TextureRef::Buffer(name) => (1, 0, name.clone()),
        TextureRef::PrevFrame(name) => (2, 0, name.clone()),
        _ => (3, 0, String::new()),
    });
    inputs
}

fn collect_inputs_inner(
    node: &crate::ast::Node,
    current_buffer: Option<&str>,
    out: &mut Vec<TextureRef>,
    seen: &mut HashSet<TextureRef>,
) {
    use crate::ast::{Node, Source};
    let push = |t: TextureRef, out: &mut Vec<TextureRef>, seen: &mut HashSet<TextureRef>| {
        if seen.insert(t.clone()) {
            out.push(t);
        }
    };
    match node {
        Node::Source(Source::Src { channel }) => push(TextureRef::InputChannel(*channel), out, seen),
        Node::Source(Source::Buffer { name }) => push(TextureRef::Buffer(name.clone()), out, seen),
        Node::Source(Source::Prev) => {
            if let Some(b) = current_buffer {
                push(TextureRef::PrevFrame(b.to_string()), out, seen);
            }
        }
        Node::Source(_) => {}
        Node::Transform { input, .. } => collect_inputs_inner(input, current_buffer, out, seen),
        Node::Combine { lhs, rhs, .. } => {
            collect_inputs_inner(lhs, current_buffer, out, seen);
            collect_inputs_inner(rhs, current_buffer, out, seen);
        }
    }
}
