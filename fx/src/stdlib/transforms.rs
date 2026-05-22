//! WGSL expression builders for transform primitives.
//!
//! Each `expr_*` takes a WGSL expression naming the input `vec4<f32>` color
//! plus the transform's parameters as WGSL expression strings, and returns a
//! new `vec4<f32>` expression. Transforms never re-evaluate the source — they
//! operate on the color produced upstream.

pub fn expr_color(color: &str, r: &str, g: &str, b: &str, a: &str) -> String {
    format!("({color} * vec4<f32>({r}, {g}, {b}, {a}))")
}

pub fn expr_brightness(color: &str, amount: &str) -> String {
    format!("vec4<f32>({color}.rgb + vec3<f32>({amount}), {color}.a)")
}

pub fn expr_contrast(color: &str, amount: &str) -> String {
    format!("vec4<f32>((({color}.rgb - vec3<f32>(0.5)) * {amount}) + vec3<f32>(0.5), {color}.a)")
}

pub fn expr_invert(color: &str, amount: &str) -> String {
    format!("vec4<f32>(mix({color}.rgb, vec3<f32>(1.0) - {color}.rgb, {amount}), {color}.a)")
}

/// Rotate `uv` around (0.5, 0.5) by `angle + iTime * speed` radians. Returns
/// the rotated uv expression — sources downstream sample at the new uv. Used
/// only when the IR threads uv (v1 work); kept here so the registry is
/// complete.
pub fn expr_rotate_uv(uv: &str, angle: &str, speed: &str, time: &str) -> String {
    format!("(rotate2d({uv} - vec2<f32>(0.5), {angle} + {time} * {speed}) + vec2<f32>(0.5))")
}

pub fn expr_scale_uv(uv: &str, amount: &str, x: &str, y: &str) -> String {
    format!("((({uv}) - vec2<f32>(0.5)) / vec2<f32>({amount} * {x}, {amount} * {y}) + vec2<f32>(0.5))")
}
