// Golden-RMSE assertion (wb-mxrk.5).
//
// Walks the host-pre-computed diff frame (max-channel absolute diff
// broadcast across RGB, normalized 0..1), computes global RMSE and the
// number of pixels with diff > max_diff_norm. Passes iff over_count <=
// max_pixels.
//
// Reason codes:
//   0 = pass
//   1 = fail
//   2 = empty frame
//
// Evidence:
//   [0] = global RMSE (0..1)
//   [1] = over_count (pixels exceeding tolerance)
//   [2] = max_diff_norm threshold
//   [3] = max_pixels budget

struct AssertionResult {
    passed: u32,
    reason_code: i32,
    evidence_count: u32,
    evidence: array<f32, 64>,
};

struct Params {
    frame_width: u32,
    frame_height: u32,
    max_diff_norm: f32,
    max_pixels: u32,
};

@group(0) @binding(0) var color_tex: texture_2d<f32>;
@group(0) @binding(3) var<uniform> params: Params;
@group(0) @binding(4) var<storage, read_write> result: AssertionResult;

@compute @workgroup_size(1, 1)
fn assert_main(@builtin(global_invocation_id) gid: vec3<u32>) {
    if (gid.x != 0u || gid.y != 0u) {
        return;
    }

    let w = params.frame_width;
    let h = params.frame_height;
    if (w == 0u || h == 0u) {
        result.passed = 0u;
        result.reason_code = 2;
        result.evidence_count = 0u;
        return;
    }

    var sos: f32 = 0.0;
    var over: u32 = 0u;
    for (var y: u32 = 0u; y < h; y = y + 1u) {
        for (var x: u32 = 0u; x < w; x = x + 1u) {
            let d = textureLoad(color_tex, vec2<i32>(i32(x), i32(y)), 0).r;
            sos = sos + d * d;
            if (d > params.max_diff_norm) {
                over = over + 1u;
            }
        }
    }
    let total = f32(w * h);
    let rmse = sqrt(sos / total);

    result.evidence[0] = rmse;
    result.evidence[1] = f32(over);
    result.evidence[2] = params.max_diff_norm;
    result.evidence[3] = f32(params.max_pixels);
    result.evidence_count = 4u;

    if (over <= params.max_pixels) {
        result.passed = 1u;
        result.reason_code = 0;
    } else {
        result.passed = 0u;
        result.reason_code = 1;
    }
}
