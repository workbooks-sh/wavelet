// Windowed mean + variance via separable Gaussian convolution (wb-mxrk.3).
//
// Two compute passes, three entry points:
//   - `pass_h_mean`     : horizontal blur of luma into `tmp_mean`
//   - `pass_v_mean_var` : vertical blur of `tmp_mean` into `mean`, plus
//                         vertical blur of squared input into `m2`, then
//                         `variance = m2 - mean^2`.
//
// Coefficients are uploaded as a `vec4<f32>` array (16 vec4s = 64 floats),
// indexed by `[0, window_size)`. window_size is bounded to 64 in Rust.

struct Params {
    frame_width: u32,
    frame_height: u32,
    window_size: u32,
    _pad: u32,
};

@group(0) @binding(0) var src_tex: texture_2d<f32>;
@group(0) @binding(1) var<uniform> params: Params;
@group(0) @binding(2) var<uniform> kernel: array<vec4<f32>, 16>;
@group(0) @binding(3) var<storage, read_write> tmp_mean: array<f32>;
@group(0) @binding(4) var<storage, read_write> tmp_m2: array<f32>;
@group(0) @binding(5) var<storage, read_write> out_mean: array<f32>;
@group(0) @binding(6) var<storage, read_write> out_var: array<f32>;

fn luma(px: vec4<f32>) -> f32 {
    return 0.2126 * px.r + 0.7152 * px.g + 0.0722 * px.b;
}

fn kernel_weight(i: u32) -> f32 {
    let v = kernel[i / 4u];
    let lane = i % 4u;
    if (lane == 0u) { return v.x; }
    if (lane == 1u) { return v.y; }
    if (lane == 2u) { return v.z; }
    return v.w;
}

fn idx(x: u32, y: u32) -> u32 {
    return y * params.frame_width + x;
}

@compute @workgroup_size(8, 8)
fn pass_h(@builtin(global_invocation_id) gid: vec3<u32>) {
    let x = gid.x;
    let y = gid.y;
    if (x >= params.frame_width || y >= params.frame_height) {
        return;
    }
    let win = params.window_size;
    let half_w = i32(win / 2u);

    var acc_mean: f32 = 0.0;
    var acc_m2: f32 = 0.0;
    for (var i: u32 = 0u; i < win; i = i + 1u) {
        let xi = clamp(i32(x) + i32(i) - half_w, 0, i32(params.frame_width) - 1);
        let px = textureLoad(src_tex, vec2<i32>(xi, i32(y)), 0);
        let l = luma(px);
        let w = kernel_weight(i);
        acc_mean = acc_mean + w * l;
        acc_m2 = acc_m2 + w * l * l;
    }
    let o = idx(x, y);
    tmp_mean[o] = acc_mean;
    tmp_m2[o] = acc_m2;
}

@compute @workgroup_size(8, 8)
fn pass_v(@builtin(global_invocation_id) gid: vec3<u32>) {
    let x = gid.x;
    let y = gid.y;
    if (x >= params.frame_width || y >= params.frame_height) {
        return;
    }
    let win = params.window_size;
    let half_w = i32(win / 2u);

    var acc_mean: f32 = 0.0;
    var acc_m2: f32 = 0.0;
    for (var i: u32 = 0u; i < win; i = i + 1u) {
        let yi = clamp(i32(y) + i32(i) - half_w, 0, i32(params.frame_height) - 1);
        let src_i = idx(x, u32(yi));
        let w = kernel_weight(i);
        acc_mean = acc_mean + w * tmp_mean[src_i];
        acc_m2 = acc_m2 + w * tmp_m2[src_i];
    }
    let o = idx(x, y);
    out_mean[o] = acc_mean;
    out_var[o] = max(acc_m2 - acc_mean * acc_mean, 0.0);
}
