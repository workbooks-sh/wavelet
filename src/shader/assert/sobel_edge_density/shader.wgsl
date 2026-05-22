// Sobel edge-density assertion (wb-mxrk.5).
//
// Single-thread reducer: scans the region, computes the 3x3 Sobel
// magnitude at each interior pixel, counts pixels with magnitude >
// threshold, passes iff (count / region_area) >= min_density.
//
// Sobel kernel applied to luminance (Rec.709). Border pixels (outside
// the 3x3 footprint) are skipped — region_area for density is the
// interior pixel count.
//
// Reason codes:
//   0 = pass
//   1 = fail (density below floor)
//   2 = empty region
//
// Evidence:
//   [0] = edge density (edge_count / interior_pixels)
//   [1] = edge_count
//   [2] = interior_pixels
//   [3] = threshold (magnitude floor)
//   [4] = min_density (parameter)

struct AssertionResult {
    passed: u32,
    reason_code: i32,
    evidence_count: u32,
    evidence: array<f32, 64>,
};

struct Params {
    frame_width: u32,
    frame_height: u32,
    region_x: f32,
    region_y: f32,
    region_w: f32,
    region_h: f32,
    threshold: f32,
    min_density: f32,
};

@group(0) @binding(0) var color_tex: texture_2d<f32>;
@group(0) @binding(3) var<uniform> params: Params;
@group(0) @binding(4) var<storage, read_write> result: AssertionResult;

fn luma(p: vec3<f32>) -> f32 {
    return 0.2126 * p.r + 0.7152 * p.g + 0.0722 * p.b;
}

fn sample_luma(x: i32, y: i32) -> f32 {
    let p = textureLoad(color_tex, vec2<i32>(x, y), 0).rgb;
    return luma(p);
}

@compute @workgroup_size(1, 1)
fn assert_main(@builtin(global_invocation_id) gid: vec3<u32>) {
    if (gid.x != 0u || gid.y != 0u) {
        return;
    }

    let fw = f32(params.frame_width);
    let fh = f32(params.frame_height);
    let x0 = i32(clamp(params.region_x * fw, 0.0, fw));
    let y0 = i32(clamp(params.region_y * fh, 0.0, fh));
    let x1 = i32(clamp((params.region_x + params.region_w) * fw, 0.0, fw));
    let y1 = i32(clamp((params.region_y + params.region_h) * fh, 0.0, fh));

    let ix0 = max(x0, 1);
    let iy0 = max(y0, 1);
    let ix1 = min(x1, i32(params.frame_width) - 1);
    let iy1 = min(y1, i32(params.frame_height) - 1);

    if (ix1 <= ix0 || iy1 <= iy0) {
        result.passed = 0u;
        result.reason_code = 2;
        result.evidence_count = 0u;
        return;
    }

    var edge_count: u32 = 0u;
    var interior: u32 = 0u;
    for (var y: i32 = iy0; y < iy1; y = y + 1) {
        for (var x: i32 = ix0; x < ix1; x = x + 1) {
            let tl = sample_luma(x - 1, y - 1);
            let tm = sample_luma(x,     y - 1);
            let tr = sample_luma(x + 1, y - 1);
            let ml = sample_luma(x - 1, y);
            let mr = sample_luma(x + 1, y);
            let bl = sample_luma(x - 1, y + 1);
            let bm = sample_luma(x,     y + 1);
            let br = sample_luma(x + 1, y + 1);

            let gx = -tl - 2.0 * ml - bl + tr + 2.0 * mr + br;
            let gy = -tl - 2.0 * tm - tr + bl + 2.0 * bm + br;
            let mag = sqrt(gx * gx + gy * gy);

            interior = interior + 1u;
            if (mag > params.threshold) {
                edge_count = edge_count + 1u;
            }
        }
    }

    let density = f32(edge_count) / f32(interior);
    result.evidence[0] = density;
    result.evidence[1] = f32(edge_count);
    result.evidence[2] = f32(interior);
    result.evidence[3] = params.threshold;
    result.evidence[4] = params.min_density;
    result.evidence_count = 5u;

    if (density >= params.min_density) {
        result.passed = 1u;
        result.reason_code = 0;
    } else {
        result.passed = 0u;
        result.reason_code = 1;
    }
}
