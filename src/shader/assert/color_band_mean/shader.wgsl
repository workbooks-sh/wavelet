// Color-band mean assertion (wb-mxrk.5).
//
// Single-thread reducer: scans a region, converts each pixel to HSL,
// accumulates mean H (via circular mean on the hue wheel), mean S, mean
// L, and hue standard deviation (circular). Passes iff all stat means
// fall within ±tolerance of the supplied targets.
//
// Hue is treated as a circular variable: mean computed as
//   atan2(mean(sin(2πH)), mean(cos(2πH))) / (2π)
// where H is in [0, 1).
//
// The LMS color-blindness simulation variant called out in the ticket is
// DEFERRED — the data path here is the HSL pipeline only. Once the
// validator catalog exposes a `colorblind_mode` enum we'll pre-multiply
// the matrix before HSL conversion. See host.rs.
//
// Reason codes:
//   0 = pass
//   1 = fail (any stat outside tolerance)
//   2 = empty region
//
// Evidence:
//   [0] = mean hue (0..1)
//   [1] = mean saturation (0..1)
//   [2] = mean lightness (0..1)
//   [3] = hue circular stdev (0..0.5; 0 = perfectly aligned)
//   [4] = target hue
//   [5] = target saturation
//   [6] = target lightness
//   [7] = tolerance

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
    target_h: f32,
    target_s: f32,
    target_l: f32,
    tolerance: f32,
};

@group(0) @binding(0) var color_tex: texture_2d<f32>;
@group(0) @binding(3) var<uniform> params: Params;
@group(0) @binding(4) var<storage, read_write> result: AssertionResult;

const TWO_PI: f32 = 6.283185307179586;

fn rgb_to_hsl(rgb: vec3<f32>) -> vec3<f32> {
    let cmax = max(rgb.r, max(rgb.g, rgb.b));
    let cmin = min(rgb.r, min(rgb.g, rgb.b));
    let d = cmax - cmin;
    let l = (cmax + cmin) * 0.5;

    var h: f32 = 0.0;
    var s: f32 = 0.0;
    if (d > 0.00001) {
        if (l < 0.5) {
            s = d / (cmax + cmin);
        } else {
            s = d / (2.0 - cmax - cmin);
        }
        if (cmax == rgb.r) {
            h = (rgb.g - rgb.b) / d;
            if (rgb.g < rgb.b) { h = h + 6.0; }
        } else if (cmax == rgb.g) {
            h = (rgb.b - rgb.r) / d + 2.0;
        } else {
            h = (rgb.r - rgb.g) / d + 4.0;
        }
        h = h / 6.0;
    }
    return vec3<f32>(h, s, l);
}

fn hue_diff(a: f32, b: f32) -> f32 {
    let d = abs(a - b);
    return min(d, 1.0 - d);
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

    var sum_s: f32 = 0.0;
    var sum_l: f32 = 0.0;
    var sum_sin: f32 = 0.0;
    var sum_cos: f32 = 0.0;
    var n: u32 = 0u;
    for (var y: u32 = y0; y < y1; y = y + 1u) {
        for (var x: u32 = x0; x < x1; x = x + 1u) {
            let px = textureLoad(color_tex, vec2<i32>(i32(x), i32(y)), 0).rgb;
            let hsl = rgb_to_hsl(px);
            sum_s = sum_s + hsl.y;
            sum_l = sum_l + hsl.z;
            let theta = hsl.x * TWO_PI;
            sum_sin = sum_sin + sin(theta);
            sum_cos = sum_cos + cos(theta);
            n = n + 1u;
        }
    }

    let nf = f32(n);
    let mean_s = sum_s / nf;
    let mean_l = sum_l / nf;
    let mean_sin = sum_sin / nf;
    let mean_cos = sum_cos / nf;
    var mean_h = atan2(mean_sin, mean_cos) / TWO_PI;
    if (mean_h < 0.0) { mean_h = mean_h + 1.0; }
    let r_len = sqrt(mean_sin * mean_sin + mean_cos * mean_cos);
    let stdev_h = sqrt(max(0.0, -2.0 * log(max(r_len, 0.00001)))) / TWO_PI;

    let dh = hue_diff(mean_h, params.target_h);
    let ds = abs(mean_s - params.target_s);
    let dl = abs(mean_l - params.target_l);
    let in_band = (dh <= params.tolerance) && (ds <= params.tolerance) && (dl <= params.tolerance);

    result.evidence[0] = mean_h;
    result.evidence[1] = mean_s;
    result.evidence[2] = mean_l;
    result.evidence[3] = stdev_h;
    result.evidence[4] = params.target_h;
    result.evidence[5] = params.target_s;
    result.evidence[6] = params.target_l;
    result.evidence[7] = params.tolerance;
    result.evidence_count = 8u;

    if (in_band) {
        result.passed = 1u;
        result.reason_code = 0;
    } else {
        result.passed = 0u;
        result.reason_code = 1;
    }
}
