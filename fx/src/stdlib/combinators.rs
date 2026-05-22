//! WGSL expression builders for combinator primitives — mix two in-flight
//! colors into one. Each `expr_*` takes WGSL expressions naming the two input
//! colors plus the combinator's parameters; returns a new `vec4<f32>` expr.
//!
//! `modulate` in Hydra is a uv-displacement combinator (lhs is resampled at
//! uv shifted by rhs.rg). At the v0 expression layer we don't re-evaluate
//! lhs, so the closest expression-level analog is colour-space displacement:
//! shift lhs by `(rhs - 0.5) * amount`. v1 will thread uv through the IR and
//! emit true Hydra-style modulation.

pub fn expr_add(lhs: &str, rhs: &str, amount: &str) -> String {
    format!("({lhs} + {rhs} * {amount})")
}

pub fn expr_mult(lhs: &str, rhs: &str, amount: &str) -> String {
    format!("({lhs} * mix(vec4<f32>(1.0), {rhs}, {amount}))")
}

pub fn expr_blend(lhs: &str, rhs: &str, amount: &str) -> String {
    format!("mix({lhs}, {rhs}, {amount})")
}

pub fn expr_modulate(lhs: &str, rhs: &str, amount: &str) -> String {
    format!("({lhs} + ({rhs} - vec4<f32>(0.5)) * {amount})")
}

pub fn expr_diff(lhs: &str, rhs: &str) -> String {
    format!("abs({lhs} - {rhs})")
}

pub fn expr_mask(lhs: &str, rhs: &str) -> String {
    format!("({lhs} * vec4<f32>(step(vec3<f32>(0.5), {rhs}.rgb), 1.0))")
}
