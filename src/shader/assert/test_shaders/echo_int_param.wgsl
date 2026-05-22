struct AssertionResult {
    passed: u32,
    reason_code: i32,
    evidence_count: u32,
    evidence: array<f32, 64>,
};

struct Params {
    frame_width: u32,
    frame_height: u32,
    some_int: u32,
};

@group(0) @binding(3) var<uniform> params: Params;
@group(0) @binding(4) var<storage, read_write> result: AssertionResult;

@compute @workgroup_size(1, 1)
fn assert_main(@builtin(global_invocation_id) gid: vec3<u32>) {
    if (gid.x == 0u && gid.y == 0u) {
        result.passed = 1u;
        result.reason_code = 0;
        result.evidence[0] = f32(params.some_int);
        result.evidence_count = 1u;
    }
}
