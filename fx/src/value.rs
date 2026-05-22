//! `Value` — a parameter that is either a constant or an Animato tween.
//!
//! Every tweenable numeric parameter in the WaveletFx AST is a `Value`. This is
//! the seam where WaveletFx inherits Animato's timeline model: instead of
//! inventing a "shader time" type, we accept Animato's `Tween<f32>` directly.
//! Consumers sample tweens by calling `tween.seek(frame_secs)` followed by
//! `tween.value()` — the exact call pattern wavelet already uses for DOM/CSS
//! animation, so the timeline/timecode model is shared end to end.
//!
//! Constants are inlined at emit time as WGSL literals. Tweens become entries
//! in the uniform table; the consumer writes their per-frame value into the
//! corresponding uniform buffer slot.
//!
//! Specialized to `f32` for v0. Color and vector tweens (e.g. `Tween<[f32;
//! 4]>` for animated `.color(r, g, b, a)`) come in v1 as `ValueColor` /
//! `ValueVec2`; doing it now would force generic trait bounds through the
//! entire AST for no v0 benefit.

use animato::Tween;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Value {
    Const(f32),
    Tween(Tween<f32>),
    /// A reference to a per-frame uniform the consumer fills (audio level,
    /// beat phase, CSS custom property, ...). Multiple `Uniform` Values
    /// referring to the same [`UniformRef`] share one slot in the emitted
    /// Uniforms struct — repeated references are deduped at lowering time.
    Uniform(UniformRef),
}

/// Per-frame uniform sources the shader can reference. The consumer
/// (wavelet) writes the per-frame value into the slot each frame; WaveletFx just
/// allocates the slot and names it.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum UniformRef {
    /// RMS of the audio mixer's running window. `u_audio_rms: f32`.
    AudioRms,
    /// Energy of a specific FFT bin. One slot per distinct bin index used
    /// across the composition (`u_audio_fft_<n>: f32`).
    AudioFftBin(u32),
    /// Beat phase in `[0, 1)`, or -1 when no beat track is bound. `u_beat: f32`.
    Beat,
    /// Deterministic per-frame seed (`comp_hash ^ frame_index`, cast to
    /// f32). `u_seed: f32`.
    Seed,
    /// Current value of a CSS custom property on the host element
    /// (Animato-driven). One slot per distinct property name
    /// (`u_prop_<sanitized>: f32`).
    CssProp(String),
}

impl From<f32> for Value {
    fn from(v: f32) -> Self {
        Value::Const(v)
    }
}

impl From<Tween<f32>> for Value {
    fn from(t: Tween<f32>) -> Self {
        Value::Tween(t)
    }
}

impl Value {
    /// `true` when this parameter needs a uniform slot at emit time. Both
    /// tweens and per-frame uniform references count; constants inline as
    /// WGSL literals.
    pub fn is_dynamic(&self) -> bool {
        !matches!(self, Value::Const(_))
    }
}

/// Stable WGSL field name for the slot a [`UniformRef`] resolves to. The
/// Uniforms struct in every emitted shader contains exactly one field per
/// distinct slot name across the composition.
pub fn uniform_slot_name(u: &UniformRef) -> String {
    match u {
        UniformRef::AudioRms => "u_audio_rms".to_string(),
        UniformRef::AudioFftBin(n) => format!("u_audio_fft_{}", n),
        UniformRef::Beat => "u_beat".to_string(),
        UniformRef::Seed => "u_seed".to_string(),
        UniformRef::CssProp(name) => format!("u_prop_{}", sanitize_css_prop(name)),
    }
}

/// CSS custom-property names are `--kebab-case`; WGSL identifiers are
/// `snake_case`. Strip the leading dashes, replace any non-alphanumeric
/// remainder with `_`. `--brand-energy` becomes `brand_energy`.
fn sanitize_css_prop(name: &str) -> String {
    let trimmed = name.trim_start_matches('-');
    let mut out = String::with_capacity(trimmed.len());
    for ch in trimmed.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch);
        } else {
            out.push('_');
        }
    }
    if out.is_empty() {
        "_".to_string()
    } else {
        out
    }
}
