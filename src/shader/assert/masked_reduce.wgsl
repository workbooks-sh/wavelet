// Region-masked reduce. Walk the frame, keep pixels whose ID-buffer value
// equals `target_id`, accumulate the selected `op` over the metric channel
// (R of the color texture), write {sum, count} to the output buffer. CPU
// finalizes mean = sum/count when op == Mean.

struct Params {
    width: u32,
    height: u32,
    target_id: u32,
    op: u32,       // 0 = mean (sum + count, finalize CPU), 1 = sum
};

struct Acc {
    sum: atomic<u32>,    // f32 sum scaled by SCALE then bit-cast via u32 atomic add
    count: atomic<u32>,
};

// Fixed-point scale for the u32 atomic accumulator. 1024 → ~10-bit
// precision on a [0,1] R-channel and safe up to ~4M pixels of saturated
// signal before u32 overflow. Single-pass reduce; if/when larger frames
// land, switch to a workgroup-local f32 reduction with one final atomic.
const SCALE: f32 = 1024.0;

@group(0) @binding(0) var color: texture_2d<f32>;
@group(0) @binding(1) var ids: texture_2d<u32>;
@group(0) @binding(2) var<uniform> params: Params;
@group(0) @binding(3) var<storage, read_write> acc: Acc;

@compute @workgroup_size(8, 8)
fn masked_reduce_main(@builtin(global_invocation_id) gid: vec3<u32>) {
    if (gid.x >= params.width || gid.y >= params.height) {
        return;
    }
    let xy = vec2<i32>(i32(gid.x), i32(gid.y));
    let pid = textureLoad(ids, xy, 0).r;
    if (pid != params.target_id) {
        return;
    }
    let v = textureLoad(color, xy, 0).r;
    let scaled = u32(clamp(v, 0.0, 1.0) * SCALE + 0.5);
    atomicAdd(&acc.sum, scaled);
    atomicAdd(&acc.count, 1u);
}
