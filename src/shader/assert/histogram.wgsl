// Atomic histogram primitive (wb-mxrk.3).
//
// Two-stage reduction in a single dispatch:
//   1. Each workgroup accumulates local bins via `var<workgroup> atomic<u32>`.
//   2. Workgroup 0,0 thread 0 (or any-thread per workgroup) flushes its
//      local bins into the global storage via global atomics.
//
// `MAX_BINS = 256` — the hardcoded upper bound. Callers passing fewer
// bins only touch the prefix; the rest stay zero. WGSL needs constant
// workgroup array sizes, so we always allocate 256 slots.

struct Params {
    frame_width: u32,
    frame_height: u32,
    region_x: u32,
    region_y: u32,
    region_w: u32,
    region_h: u32,
    channel: u32,     // 0=R, 1=G, 2=B, 3=A, 4=luma
    bins: u32,        // 1..=256
};

@group(0) @binding(0) var color_tex: texture_2d<f32>;
@group(0) @binding(1) var<uniform> params: Params;
@group(0) @binding(2) var<storage, read_write> global_bins: array<atomic<u32>>;

const WG_X: u32 = 8u;
const WG_Y: u32 = 8u;
const MAX_BINS: u32 = 256u;

var<workgroup> local_bins: array<atomic<u32>, 256>;

fn sample_channel(px: vec4<f32>, channel: u32) -> f32 {
    if (channel == 0u) { return px.r; }
    if (channel == 1u) { return px.g; }
    if (channel == 2u) { return px.b; }
    if (channel == 3u) { return px.a; }
    return 0.2126 * px.r + 0.7152 * px.g + 0.0722 * px.b;
}

@compute @workgroup_size(8, 8)
fn assert_main(
    @builtin(global_invocation_id) gid: vec3<u32>,
    @builtin(local_invocation_index) lid: u32,
) {
    let local_id = lid;
    if (local_id < MAX_BINS) {
        atomicStore(&local_bins[local_id], 0u);
    }
    workgroupBarrier();

    let x = params.region_x + gid.x;
    let y = params.region_y + gid.y;
    let inside_region = (gid.x < params.region_w) && (gid.y < params.region_h);
    let inside_frame = (x < params.frame_width) && (y < params.frame_height);
    if (inside_region && inside_frame) {
        let px = textureLoad(color_tex, vec2<i32>(i32(x), i32(y)), 0);
        let v = clamp(sample_channel(px, params.channel), 0.0, 1.0);
        let bins_f = f32(params.bins);
        var bin = u32(floor(v * bins_f));
        if (bin >= params.bins) {
            bin = params.bins - 1u;
        }
        atomicAdd(&local_bins[bin], 1u);
    }
    workgroupBarrier();

    if (local_id < params.bins) {
        let count = atomicLoad(&local_bins[local_id]);
        if (count > 0u) {
            atomicAdd(&global_bins[local_id], count);
        }
    }
}
