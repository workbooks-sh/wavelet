// Fused 3x3 Sobel edge magnitude.
//
// Reads luminance from the input color texture, computes Gx/Gy via the
// standard Sobel kernel, writes sqrt(Gx^2 + Gy^2) normalized to [0,1] into
// the R channel of the output storage texture. G/B mirror R for visual
// inspection; A is 1.0. Out-of-bounds reads clamp to the edge.

@group(0) @binding(0) var src: texture_2d<f32>;
@group(0) @binding(1) var dst: texture_storage_2d<rgba8unorm, write>;

fn luma(p: vec4<f32>) -> f32 {
    return dot(p.rgb, vec3<f32>(0.2126, 0.7152, 0.0722));
}

fn sample_clamped(coord: vec2<i32>, dims: vec2<i32>) -> f32 {
    let c = clamp(coord, vec2<i32>(0, 0), dims - vec2<i32>(1, 1));
    return luma(textureLoad(src, c, 0));
}

@compute @workgroup_size(8, 8)
fn sobel_main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let dims = vec2<i32>(textureDimensions(src));
    let xy = vec2<i32>(i32(gid.x), i32(gid.y));
    if (xy.x >= dims.x || xy.y >= dims.y) {
        return;
    }

    let tl = sample_clamped(xy + vec2<i32>(-1, -1), dims);
    let t  = sample_clamped(xy + vec2<i32>( 0, -1), dims);
    let tr = sample_clamped(xy + vec2<i32>( 1, -1), dims);
    let l  = sample_clamped(xy + vec2<i32>(-1,  0), dims);
    let r  = sample_clamped(xy + vec2<i32>( 1,  0), dims);
    let bl = sample_clamped(xy + vec2<i32>(-1,  1), dims);
    let b  = sample_clamped(xy + vec2<i32>( 0,  1), dims);
    let br = sample_clamped(xy + vec2<i32>( 1,  1), dims);

    let gx = -tl - 2.0 * l - bl + tr + 2.0 * r + br;
    let gy = -tl - 2.0 * t - tr + bl + 2.0 * b + br;

    // Sobel magnitude max for an 8-bit luma input is 4*sqrt(2) ≈ 5.657.
    // Divide by that to land in [0,1] for the rgba8unorm sink.
    let mag = sqrt(gx * gx + gy * gy) / 5.65685424949;
    let m = clamp(mag, 0.0, 1.0);
    textureStore(dst, xy, vec4<f32>(m, m, m, 1.0));
}
