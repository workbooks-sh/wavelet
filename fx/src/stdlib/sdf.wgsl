// 2D SDF helpers — sphere / box / torus + smooth-min booleans.
//
// Attribution: ported from sdfu (https://github.com/termhn/sdfu,
// MIT/Apache-2.0), which itself credits Inigo Quilez's analytic SDF
// catalog (https://iquilezles.org/articles/distfunctions2d/). sdfu
// exposes these as Rust CPU code, not WGSL — per the gamut_fx integration
// brief we port the formulas with attribution.

// 2D circle SDF. Distance from `uv` to a centered disc of given
// radius. Negative inside, positive outside.
fn shady_sdf_sphere(uv: vec2<f32>, radius: f32) -> f32 {
  return length(uv - vec2<f32>(0.5)) - radius;
}

// 2D axis-aligned box SDF. `extent` is the half-size (width, height).
fn shady_sdf_box(uv: vec2<f32>, extent: vec2<f32>) -> f32 {
  let p = abs(uv - vec2<f32>(0.5)) - extent;
  return length(max(p, vec2<f32>(0.0))) + min(max(p.x, p.y), 0.0);
}

// 2D annulus / ring SDF. `radius` = centerline, `thickness` = half
// band width.
fn shady_sdf_torus(uv: vec2<f32>, radius: f32, thickness: f32) -> f32 {
  return abs(length(uv - vec2<f32>(0.5)) - radius) - thickness;
}

// Render a signed distance as a soft-edged grayscale vec4. Inside
// (d <= 0) is white; outside is black; `smoothing` antialiases the
// edge ramp.
fn shady_sdf_render(d: f32, smoothing: f32) -> vec4<f32> {
  let s = 1.0 - smoothstep(0.0, max(smoothing, 1e-5), d);
  return vec4<f32>(vec3<f32>(s), 1.0);
}

// Polynomial smooth-min of two color-shaped SDF masks. Operates on
// rgb (alpha = max of the two). Our convention: 1 inside, 0 outside,
// so the union is the brighter pixel — apply smooth-max in rgb.
fn shady_smooth_union(a: vec4<f32>, b: vec4<f32>, k: f32) -> vec4<f32> {
  let h = clamp(0.5 + 0.5 * (b.rgb - a.rgb) / max(vec3<f32>(k), vec3<f32>(1e-5)),
                vec3<f32>(0.0), vec3<f32>(1.0));
  let m = mix(b.rgb, a.rgb, h) + vec3<f32>(k) * h * (vec3<f32>(1.0) - h);
  return vec4<f32>(clamp(m, vec3<f32>(0.0), vec3<f32>(1.0)), max(a.a, b.a));
}

// Smooth-max intersect. Symmetric counterpart of smooth_union.
fn shady_smooth_intersect(a: vec4<f32>, b: vec4<f32>, k: f32) -> vec4<f32> {
  let h = clamp(0.5 - 0.5 * (b.rgb - a.rgb) / max(vec3<f32>(k), vec3<f32>(1e-5)),
                vec3<f32>(0.0), vec3<f32>(1.0));
  let m = mix(b.rgb, a.rgb, h) - vec3<f32>(k) * h * (vec3<f32>(1.0) - h);
  return vec4<f32>(clamp(m, vec3<f32>(0.0), vec3<f32>(1.0)), min(a.a, b.a));
}
