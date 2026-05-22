// Reduction primitive shader (wb-mxrk.2).
//
// Two entry points:
//   reduce_tile  — pass 1: each workgroup reduces one tile of the input
//                  texture's region into one partial result per workgroup,
//                  written to `partials_out[wg_index * 4 .. +4]` (RGBA).
//   reduce_pass  — pass N>1: reduces a partials buffer in-place across
//                  workgroups, halving the active count per dispatch.
//
// Op codes (must match `ReduceOp` in reduce.rs):
//   0 = sum
//   1 = max
//   2 = mean (sum + post-divide on CPU)
//   3 = sum-of-squares (used by variance, combined with sum on CPU)
//
// Layout:
//   binding 0: input color texture (texture_2d<f32>)
//   binding 1: ReduceParams uniform
//   binding 2: partials_in  storage<read>      (pass 2+)
//   binding 3: partials_out storage<read_write>
//
// All reductions are channel-wise on vec4<f32>. Scalar callers can sum
// channels and divide by 3 (or pull `.x` for single-channel inputs).

struct ReduceParams {
    region_x: u32,
    region_y: u32,
    region_w: u32,
    region_h: u32,
    op: u32,
    pass_count: u32,    // pass-2+ only: number of valid partials in input
    mean_r: f32,        // pass-2+ for sum-of-squares: subtract this mean
    mean_g: f32,
    mean_b: f32,
    mean_a: f32,
};

@group(0) @binding(0) var src: texture_2d<f32>;
@group(0) @binding(1) var<uniform> params: ReduceParams;
@group(0) @binding(2) var<storage, read> partials_in: array<vec4<f32>>;
@group(0) @binding(3) var<storage, read_write> partials_out: array<vec4<f32>>;

const WG: u32 = 16u;
const TILE: u32 = WG * WG;

var<workgroup> scratch: array<vec4<f32>, TILE>;

fn combine(op: u32, a: vec4<f32>, b: vec4<f32>) -> vec4<f32> {
    if (op == 1u) {
        return max(a, b);
    }
    return a + b;
}

fn identity(op: u32) -> vec4<f32> {
    if (op == 1u) {
        return vec4<f32>(-3.4028235e38);
    }
    return vec4<f32>(0.0);
}

fn load_pixel(px: u32, py: u32) -> vec4<f32> {
    let sx = i32(params.region_x + px);
    let sy = i32(params.region_y + py);
    var v = textureLoad(src, vec2<i32>(sx, sy), 0);
    if (params.op == 3u) {
        let m = vec4<f32>(params.mean_r, params.mean_g, params.mean_b, params.mean_a);
        let d = v - m;
        v = d * d;
    }
    return v;
}

fn workgroup_reduce(local: u32, op: u32) {
    var stride: u32 = TILE >> 1u;
    loop {
        if (stride == 0u) { break; }
        workgroupBarrier();
        if (local < stride) {
            scratch[local] = combine(op, scratch[local], scratch[local + stride]);
        }
        stride = stride >> 1u;
    }
    workgroupBarrier();
}

@compute @workgroup_size(WG, WG, 1)
fn reduce_tile(
    @builtin(global_invocation_id) gid: vec3<u32>,
    @builtin(local_invocation_index) local: u32,
    @builtin(workgroup_id) wg: vec3<u32>,
    @builtin(num_workgroups) num_wg: vec3<u32>,
) {
    let op = params.op;
    var acc = identity(op);
    if (gid.x < params.region_w && gid.y < params.region_h) {
        acc = load_pixel(gid.x, gid.y);
    }
    scratch[local] = acc;
    workgroup_reduce(local, op);
    if (local == 0u) {
        let idx = wg.y * num_wg.x + wg.x;
        partials_out[idx] = scratch[0];
    }
}

@compute @workgroup_size(WG * WG, 1, 1)
fn reduce_pass(
    @builtin(local_invocation_index) local: u32,
    @builtin(workgroup_id) wg: vec3<u32>,
    @builtin(num_workgroups) num_wg: vec3<u32>,
) {
    let op = params.op;
    let total = params.pass_count;
    let per_wg = (total + num_wg.x - 1u) / num_wg.x;
    let base = wg.x * per_wg;

    var acc = identity(op);
    var i: u32 = local;
    loop {
        if (i >= per_wg) { break; }
        let g = base + i;
        if (g < total) {
            acc = combine(op, acc, partials_in[g]);
        }
        i = i + TILE;
    }
    scratch[local] = acc;
    workgroup_reduce(local, op);
    if (local == 0u) {
        partials_out[wg.x] = scratch[0];
    }
}
