// WCAG 2.2 luminance contrast assertion (wb-mxrk.5).
//
// Scans a normalized region of the input color frame for the max + min
// relative luminance, computes the contrast ratio per the W3C formula,
// and writes pass/fail iff CR >= min_contrast.
//
// Single-thread reducer (@workgroup_size(1,1) dispatched once); for the
// frame sizes the validator runs on (full HD region scans) this stays
// well inside a sub-millisecond dispatch and avoids the workgroup-shared
// partials path. If we need to scale, switch to the reduce primitive's
// two-pass min/max — for now, simplicity wins.
//
// Reason codes:
//   0 = pass (CR >= min_contrast)
//   1 = fail, contrast below floor
//   2 = empty region
//
// Evidence:
//   [0] = contrast ratio
//   [1] = L_max (lighter)
//   [2] = L_min (darker)
//   [3] = min_contrast threshold

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
    min_contrast: f32,
};

@group(0) @binding(0) var color_tex: texture_2d<f32>;
@group(0) @binding(3) var<uniform> params: Params;
@group(0) @binding(4) var<storage, read_write> result: AssertionResult;

fn srgb_to_linear(c: f32) -> f32 {
    if (c <= 0.04045) {
        return c / 12.92;
    }
    return pow((c + 0.055) / 1.055, 2.4);
}

fn relative_luminance(rgb: vec3<f32>) -> f32 {
    let r = srgb_to_linear(rgb.r);
    let g = srgb_to_linear(rgb.g);
    let b = srgb_to_linear(rgb.b);
    return 0.2126 * r + 0.7152 * g + 0.0722 * b;
}

@compute @workgroup_size(1, 1)
fn assert_main(@builtin(global_invocation_id) gid: vec3<u32>) {
    if (gid.x != 0u || gid.y != 0u) {
        return;
    }

    let fw = f32(params.frame_width);
    let fh = f32(params.frame_height);
    let x0 = u32(clamp(params.region_x * fw, 0.0, fw));
    let y0 = u32(clamp(params.region_y * fh, 0.0, fh));
    let x1 = u32(clamp((params.region_x + params.region_w) * fw, 0.0, fw));
    let y1 = u32(clamp((params.region_y + params.region_h) * fh, 0.0, fh));

    if (x1 <= x0 || y1 <= y0) {
        result.passed = 0u;
        result.reason_code = 2;
        result.evidence_count = 0u;
        return;
    }

    var lmax: f32 = -1.0;
    var lmin: f32 = 2.0;
    for (var y: u32 = y0; y < y1; y = y + 1u) {
        for (var x: u32 = x0; x < x1; x = x + 1u) {
            let px = textureLoad(color_tex, vec2<i32>(i32(x), i32(y)), 0).rgb;
            let l = relative_luminance(px);
            if (l > lmax) { lmax = l; }
            if (l < lmin) { lmin = l; }
        }
    }

    let cr = (lmax + 0.05) / (lmin + 0.05);
    let passed = cr >= params.min_contrast;
    if (passed) {
        result.passed = 1u;
        result.reason_code = 0;
    } else {
        result.passed = 0u;
        result.reason_code = 1;
    }
    result.evidence_count = 4u;
    result.evidence[0] = cr;
    result.evidence[1] = lmax;
    result.evidence[2] = lmin;
    result.evidence[3] = params.min_contrast;
}
