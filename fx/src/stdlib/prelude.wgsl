// GamutFx stdlib prelude — helpers reused across every emitted shader.
//
// Kept lightweight: hash, rotate2d, and a handful of source primitives the
// expression-style emitters call directly. WGSL constant-folds the unused
// helpers so paying the bytes for the whole prelude per shader is fine.

fn hash21(p: vec2<f32>) -> f32 {
  var p3 = fract(vec3<f32>(p.xyx) * 0.1031);
  p3 = p3 + dot(p3, p3.yzx + 33.33);
  return fract((p3.x + p3.y) * p3.z);
}

fn hash22(p: vec2<f32>) -> vec2<f32> {
  let q = vec2<f32>(dot(p, vec2<f32>(127.1, 311.7)), dot(p, vec2<f32>(269.5, 183.3)));
  return fract(sin(q) * 43758.5453);
}

fn rotate2d(uv: vec2<f32>, angle: f32) -> vec2<f32> {
  let c = cos(angle);
  let s = sin(angle);
  return mat2x2<f32>(c, -s, s, c) * uv;
}

// Hydra-style single-cell Voronoi. `scale` controls cell density; `speed`
// drifts the cell anchors over time; `blending` softens the cell edges
// (0 = hard, 1 = fully blended). Returns a colour-shaped vec4 — black at
// cell centres, brighter toward edges.
fn shady_voronoi(uv: vec2<f32>, scale: f32, speed: f32, blending: f32, t: f32) -> vec4<f32> {
  let g = floor(uv * scale);
  let f = fract(uv * scale);
  var min_dist: f32 = 8.0;
  for (var j: i32 = -1; j <= 1; j = j + 1) {
    for (var i: i32 = -1; i <= 1; i = i + 1) {
      let nb = vec2<f32>(f32(i), f32(j));
      let anchor = 0.5 + 0.5 * sin(t * speed + 6.2831 * hash22(g + nb));
      let d = length(nb + anchor - f);
      min_dist = min(min_dist, d);
    }
  }
  let v = mix(min_dist, smoothstep(0.0, 1.0, min_dist), blending);
  return vec4<f32>(vec3<f32>(v), 1.0);
}

// Hydra's gradient defaults: rg = uv, b = phase, a = 1. Useful as a debug
// texture or as a modulation source.
fn shady_gradient(uv: vec2<f32>, speed: f32, t: f32) -> vec4<f32> {
  let b = 0.5 + 0.5 * sin(t * speed);
  return vec4<f32>(uv.x, uv.y, b, 1.0);
}

// Regular n-sided polygon SDF centred at (0.5, 0.5). `radius` is the
// inscribed-circle radius; `smoothing` widens the edge ramp. Sides clamped
// to >= 3 — fewer makes no geometric sense.
fn shady_shape(uv: vec2<f32>, sides: f32, radius: f32, smoothing: f32) -> vec4<f32> {
  let p = uv - vec2<f32>(0.5);
  let n = max(sides, 3.0);
  let a = atan2(p.y, p.x) + 3.14159265;
  let r = 6.28318530 / n;
  let d = cos(floor(0.5 + a / r) * r - a) * length(p);
  let s = 1.0 - smoothstep(radius, radius + smoothing, d);
  return vec4<f32>(vec3<f32>(s), 1.0);
}
