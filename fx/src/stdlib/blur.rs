//! Separable Gaussian blur — WGSL fragment-shader builders for the two
//! halves of a 2-pass blur. wavelet_fx's `.blur()` lowers to a horizontal pass
//! reading the source, writing into a half-res-ish intermediate; then a
//! vertical pass reading that intermediate, writing into the
//! "blurred-source" buffer that the rest of the chain samples from.
//!
//! Math reference: Bevy's `bevy_core_pipeline::bloom` (MIT-licensed).
//! The kernel here is a 17-tap normalized Gaussian over a configurable
//! σ; Bevy uses dual-Kawase for performance and we deliberately picked
//! the simpler separable Gaussian first because:
//!
//! 1. wavelet_fx authors specify σ directly (`.blur(radius)`); dual-Kawase
//!    approximates Gaussians but doesn't expose σ as a tunable.
//! 2. Visual parity with the CPU `gaussian_rgba` fallback is the
//!    Phase 2 acceptance test — separable Gaussian on GPU matches the
//!    CPU output pixel-for-pixel within rounding tolerance.
//!
//! The actual taps are computed on-the-fly inside the WGSL function from
//! the σ uniform — keeping the kernel data-driven means a single shader
//! body works across all blur radii authors might pick at runtime.
//!
//! Attribution: dual-blur / bloom pipeline structure adapted from Bevy
//! Engine, `crates/bevy_core_pipeline/src/bloom/`. License: MIT.
//! <https://github.com/bevyengine/bevy/blob/main/crates/bevy_core_pipeline/src/bloom/>

/// Horizontal pass of a separable 1D Gaussian. The author of the
/// consumer pipeline:
/// 1. Binds the source RGBA texture at `@binding(1)` (texture) +
///    `@binding(2)` (sampler).
/// 2. Sets `u.u_blur_sigma` (the σ this pass uses) and `u.u_resolution`
///    (so we can step in 1-texel units) in the uniform buffer.
/// 3. Renders a fullscreen triangle; output goes to the intermediate
///    "blur_h_<channel>" texture.
///
/// Tap count: ceil(3σ) on each side, clamped to a max of 16 to stay
/// inside reasonable shader-instruction budgets. At σ=8 that's 25 taps;
/// at σ=16 it's the cap-clamped 33 taps. The cap mirrors Bevy's bloom
/// up-sample budget on integrated GPUs.
pub const SEPARABLE_GAUSSIAN_H: &str = r#"
// Separable Gaussian — horizontal pass.
// Reference: Bevy bloom (MIT) — bevyengine/bevy/crates/bevy_core_pipeline/src/bloom/
fn fx_blur_h(
  src: texture_2d<f32>,
  smp: sampler,
  uv: vec2<f32>,
  sigma: f32,
  resolution: vec2<f32>,
) -> vec4<f32> {
  let radius_f = ceil(sigma * 3.0);
  let radius = min(i32(radius_f), 16);
  let two_sigma_sq = 2.0 * sigma * sigma;
  let texel = vec2<f32>(1.0 / resolution.x, 0.0);
  var acc = vec4<f32>(0.0);
  var wsum: f32 = 0.0;
  for (var i: i32 = -16; i <= 16; i = i + 1) {
    if (i < -radius || i > radius) { continue; }
    let x = f32(i);
    let w = exp(-x * x / two_sigma_sq);
    let s = textureSample(src, smp, uv + texel * x);
    acc = acc + s * w;
    wsum = wsum + w;
  }
  return acc / max(wsum, 1e-5);
}
"#;

/// Vertical pass. Same shape as horizontal but steps along Y.
pub const SEPARABLE_GAUSSIAN_V: &str = r#"
// Separable Gaussian — vertical pass.
// Reference: Bevy bloom (MIT) — bevyengine/bevy/crates/bevy_core_pipeline/src/bloom/
fn fx_blur_v(
  src: texture_2d<f32>,
  smp: sampler,
  uv: vec2<f32>,
  sigma: f32,
  resolution: vec2<f32>,
) -> vec4<f32> {
  let radius_f = ceil(sigma * 3.0);
  let radius = min(i32(radius_f), 16);
  let two_sigma_sq = 2.0 * sigma * sigma;
  let texel = vec2<f32>(0.0, 1.0 / resolution.y);
  var acc = vec4<f32>(0.0);
  var wsum: f32 = 0.0;
  for (var i: i32 = -16; i <= 16; i = i + 1) {
    if (i < -radius || i > radius) { continue; }
    let y = f32(i);
    let w = exp(-y * y / two_sigma_sq);
    let s = textureSample(src, smp, uv + texel * y);
    acc = acc + s * w;
    wsum = wsum + w;
  }
  return acc / max(wsum, 1e-5);
}
"#;
