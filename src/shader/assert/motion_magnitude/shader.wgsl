// Motion-magnitude assertion (wb-mxrk.5).
//
// Operates on a pre-computed diff frame (host pre-computes per-pixel L2
// RGB delta and broadcasts to all channels). Walks every pixel once,
// builds an 8-bucket histogram of motion magnitude + mean motion, and
// passes iff mean >= params.min_mean.
//
// Reason codes:
//   0 = pass (mean >= min_mean)
//   1 = fail
//   2 = empty frame
//
// Evidence:
//   [0..8] = 8-bucket histogram (bin edges 0, 1/8, 2/8, ..., 1.0)
//   [8]    = mean motion magnitude
//   [9]    = min_mean threshold

struct AssertionResult {
    passed: u32,
    reason_code: i32,
    evidence_count: u32,
    evidence: array<f32, 64>,
};

struct Params {
    frame_width: u32,
    frame_height: u32,
    min_mean: f32,
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

    var hist: array<u32, 8>;
    for (var i: u32 = 0u; i < 8u; i = i + 1u) { hist[i] = 0u; }

    var sum: f32 = 0.0;
    for (var y: u32 = 0u; y < h; y = y + 1u) {
        for (var x: u32 = 0u; x < w; x = x + 1u) {
            let m = textureLoad(color_tex, vec2<i32>(i32(x), i32(y)), 0).r;
            sum = sum + m;
            var bin = u32(floor(m * 8.0));
            if (bin > 7u) { bin = 7u; }
            hist[bin] = hist[bin] + 1u;
        }
    }

    let total = f32(w * h);
    let mean = sum / total;

    for (var i: u32 = 0u; i < 8u; i = i + 1u) {
        result.evidence[i] = f32(hist[i]);
    }
    result.evidence[8] = mean;
    result.evidence[9] = params.min_mean;
    result.evidence_count = 10u;

    if (mean >= params.min_mean) {
        result.passed = 1u;
        result.reason_code = 0;
    } else {
        result.passed = 0u;
        result.reason_code = 1;
    }
}
