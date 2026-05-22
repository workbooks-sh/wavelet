//! WGSL expression builders for source primitives.
//!
//! Each `expr_*` returns a WGSL expression that produces a `vec4<f32>`.
//! Inputs are WGSL expression strings — the caller (the emit walker) is
//! responsible for substituting either inline literals (for constants) or
//! uniform references like `u.u_tween_0` (for animated values).

pub fn expr_solid(r: &str, g: &str, b: &str, a: &str) -> String {
    format!("vec4<f32>({r}, {g}, {b}, {a})")
}

pub fn expr_src(channel: u32, uv: &str) -> String {
    format!("textureSample(iChannel{0}, iChannel{0}_sampler, {1})", channel, uv)
}

/// 1-octave value noise via `hash21`. Same uv-domain space Hydra uses
/// (scale = repeats-per-tile, offset advances along the hash).
pub fn expr_noise(uv: &str, scale: &str, offset: &str) -> String {
    format!(
        "vec4<f32>(vec3<f32>(hash21({uv} * {scale} + vec2<f32>({offset}))), 1.0)"
    )
}

/// Sine oscillator across uv.x — `frequency` cycles per tile, `sync` shifts
/// the phase along y (Hydra's "sync" param), `offset` translates time.
pub fn expr_osc(uv: &str, frequency: &str, sync: &str, offset: &str) -> String {
    format!(
        "vec4<f32>(vec3<f32>(0.5 + 0.5 * sin(({uv}.x * {frequency} + {uv}.y * {sync} + {offset}) * 6.28318)), 1.0)"
    )
}

/// Voronoi cell noise. The full algorithm lives in `prelude.wgsl` as
/// `shady_voronoi`; this just emits the call expression with the
/// composition clock as `t`.
pub fn expr_voronoi(uv: &str, scale: &str, speed: &str, blending: &str) -> String {
    format!("shady_voronoi({uv}, {scale}, {speed}, {blending}, u.u_time)")
}

/// Hydra-style debug gradient: rg = uv, b = phase. Speed drives the phase.
pub fn expr_gradient(uv: &str, speed: &str) -> String {
    format!("shady_gradient({uv}, {speed}, u.u_time)")
}

/// Regular polygon SDF. `sides` is u32 in the AST; passed as an f32 here
/// because WGSL doesn't auto-coerce.
pub fn expr_shape(uv: &str, sides: u32, radius: &str, smoothing: &str) -> String {
    format!("shady_shape({uv}, {sides}.0, {radius}, {smoothing})")
}
