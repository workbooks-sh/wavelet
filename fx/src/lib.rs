//! WaveletFx — Hydra-shaped DSL that compiles to WGSL fragment shaders + a
//! render-graph spec.
//!
//! Renderer-agnostic by design: this crate never imports `wgpu`. Consumers
//! (wavelet) receive an [`EmitOutput`] containing WGSL strings and a JSON spec
//! describing pass order, intermediate buffers, and uniform bindings.
//!
//! See `SHADY.md` at the crate root for the full design and v0 scope.

pub mod ast;
pub mod builder;
pub mod diagnostics;
pub mod emit;
pub mod ir;
pub mod parse;
pub mod stdlib;
pub mod value;

pub use builder::{
    audio_fft, audio_rms, box_sdf, from_buffer, gradient, noise, osc, prev, prop, seed, shape,
    solid, sphere, src, time_beat, torus, voronoi, Chain, Composition,
};
pub use value::UniformRef;
pub use parse::parse;
pub use diagnostics::Diagnostic;
pub use emit::{BindingSlot, EmitOutput, EmittedPass, PassBindings, PreEffect, TextureBinding};
pub use ir::{BufferSpec, Pass, RenderGraph, TextureRef, UniformBinding, UniformKind};
pub use value::Value;

// Re-export the slice of Animato that WaveletFx users interact with directly, so
// they import `use wavelet_fx::{osc, Tween, Easing};` from one place instead of
// pulling in a second crate. The same `Tween` / `Easing` / `Timeline` types
// wavelet already uses for DOM animation — one timeline/timecode model end to
// end.
pub use animato::{Easing, Timeline, Tween};

/// Compile a [`Composition`] (the output of the builder API or the parser)
/// down to an [`EmitOutput`]: WGSL strings + a render-graph spec the consumer
/// uses to build wgpu pipelines.
///
/// Every emitted pass is run through naga's WGSL frontend before this
/// function returns. A parse / type error becomes
/// [`Diagnostic::InvalidEmittedWgsl`] tagged with the offending pass name
/// — far easier to debug than the equivalent error surfacing inside
/// `wgpu::Device::create_shader_module` at render time. Use
/// [`compile_unvalidated`] if you need the raw output (e.g. test
/// harnesses that intentionally emit broken WGSL to exercise error
/// paths).
pub fn compile(composition: &Composition) -> Result<EmitOutput, Diagnostic> {
    let out = compile_unvalidated(composition)?;
    emit::validate_with_naga(&out)?;
    Ok(out)
}

/// Skip the naga validation step. Public for test harnesses only —
/// production callers should use [`compile`] so they don't ship invalid
/// WGSL.
pub fn compile_unvalidated(composition: &Composition) -> Result<EmitOutput, Diagnostic> {
    let graph = ir::lower(composition)?;
    Ok(emit::emit(&graph))
}
