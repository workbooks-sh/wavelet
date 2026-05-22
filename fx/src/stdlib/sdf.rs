//! Expression builders for the 2D SDF primitives + smooth-min boolean
//! combinators. The actual WGSL function bodies live in [`sdf.wgsl`]
//! and get concatenated into the per-shader prelude by [`super::PRELUDE`].
//!
//! Math attribution: ported verbatim from sdfu
//! (<https://github.com/termhn/sdfu>, MIT/Apache-2.0), which itself
//! credits Inigo Quilez's canonical SDF article
//! (<https://iquilezles.org/articles/distfunctions2d/>). sdfu exposes
//! these as Rust CPU code, not WGSL strings — per the integration
//! brief we port the formulas verbatim with attribution.
//!
//! WaveletFx is fragment-only; "sphere" / "box" / "torus" here mean their
//! 2D analytic SDF analogues sampled at the fragment's uv. A 3D
//! raymarcher would call into the same distance functions on 3D rays;
//! we evaluate them on the (uv - center) vector instead. Authors who
//! want true 3D should reach for a dedicated raymarching shader —
//! outside wavelet_fx's "video post" scope per SHADY.md.
//!
//! Each `expr_*` returns a `vec4<f32>` whose rgb is the soft-edged
//! grayscale (smoothstepped from the SDF distance) and a is 1.0.
//! Outputs slot into wavelet_fx's color combinators (.blend, .add, .mult,
//! ...) the same way every other source does. The smooth_union /
//! smooth_intersect combinators operate on the resulting *colors*,
//! not raw distance fields — a v0 simplification that gives the
//! visually expected smooth-min behavior in the common case. Full-
//! fidelity SDF booleans (chained on pre-smoothstep distance scalars)
//! arrive in v1 when wavelet_fx gets a Value-shaped scalar pipeline.

pub fn expr_sphere(uv: &str, radius: &str, smoothing: &str) -> String {
    format!("shady_sdf_render(shady_sdf_sphere({uv}, {radius}), {smoothing})")
}

pub fn expr_box_sdf(uv: &str, width: &str, height: &str, smoothing: &str) -> String {
    format!(
        "shady_sdf_render(shady_sdf_box({uv}, vec2<f32>({width}, {height})), {smoothing})"
    )
}

pub fn expr_torus(uv: &str, radius: &str, thickness: &str, smoothing: &str) -> String {
    format!(
        "shady_sdf_render(shady_sdf_torus({uv}, {radius}, {thickness}), {smoothing})"
    )
}

pub fn expr_smooth_union(lhs: &str, rhs: &str, k: &str) -> String {
    format!("shady_smooth_union({lhs}, {rhs}, {k})")
}

pub fn expr_smooth_intersect(lhs: &str, rhs: &str, k: &str) -> String {
    format!("shady_smooth_intersect({lhs}, {rhs}, {k})")
}
