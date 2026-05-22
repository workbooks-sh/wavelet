// Test shader for wb-mxrk.7 naga reflection: distinct @workgroup_size(8, 4).
// The shader writes the workgroup dimensions it was compiled with into
// the evidence buffer so the host test can confirm the runtime read the
// right values (vs falling back to the (8, 8) default).

struct AssertionResult {
    passed: u32,
    reason_code: i32,
    evidence_count: u32,
    evidence: array<f32, 64>,
};

struct Params {
    frame_width: u32,
    frame_height: u32,
};

@group(0) @binding(3) var<uniform> params: Params;
@group(0) @binding(4) var<storage, read_write> result: AssertionResult;

@compute @workgroup_size(8, 4)
fn assert_main(@builtin(global_invocation_id) gid: vec3<u32>,
               @builtin(local_invocation_id) lid: vec3<u32>) {
    if (gid.x == 0u && gid.y == 0u) {
        result.passed = 1u;
        result.reason_code = 0;
        result.evidence[0] = 8.0;
        result.evidence[1] = 4.0;
        result.evidence_count = 2u;
    }
}
