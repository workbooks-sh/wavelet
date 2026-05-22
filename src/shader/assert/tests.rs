use std::path::PathBuf;
use std::sync::Arc;

use super::{
    dispatch_assertion, run_assertion, run_assertion_batch, FrameSource, GpuContext,
    ShaderAssertion, TextureHandle,
};

fn shader(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("src/shader/assert/test_shaders")
        .join(name)
}

fn dummy_frame() -> FrameSource {
    FrameSource::Rgba8 {
        width: 4,
        height: 4,
        pixels: vec![0u8; 4 * 4 * 4],
    }
}

#[test]
fn always_passes() {
    let outcome = dispatch_assertion(
        &shader("always_passes.wgsl"),
        dummy_frame(),
        serde_json::Value::Null,
    )
    .expect("dispatch");
    assert!(outcome.passed, "expected pass, got {outcome:?}");
    assert_eq!(outcome.reason_code, 0);
    assert!(outcome.evidence.is_empty());
}

#[test]
fn json_int_param_arrives_as_u32() {
    let outcome = dispatch_assertion(
        &shader("echo_int_param.wgsl"),
        dummy_frame(),
        serde_json::json!([7]),
    )
    .expect("dispatch");
    assert!(outcome.passed, "expected pass, got {outcome:?}");
    assert_eq!(outcome.evidence.len(), 1);
    assert!(
        (outcome.evidence[0] - 7.0).abs() < 1e-6,
        "expected echoed u32 = 7, got {} (raw bits {:#x})",
        outcome.evidence[0],
        outcome.evidence[0].to_bits(),
    );
}

#[test]
fn always_fails_with_evidence() {
    let outcome = dispatch_assertion(
        &shader("always_fails.wgsl"),
        dummy_frame(),
        serde_json::Value::Null,
    )
    .expect("dispatch");
    assert!(!outcome.passed, "expected fail, got {outcome:?}");
    assert_eq!(outcome.reason_code, 1);
    assert_eq!(outcome.evidence.len(), 2);
    assert!((outcome.evidence[0] - 0.25).abs() < 1e-6);
    assert!((outcome.evidence[1] - 0.75).abs() < 1e-6);
    assert_eq!(outcome.reason, "metric out of bounds");
}

// wb-mxrk.7: shader sources statically embedded for the new runtime tests
// so `ShaderAssertion::wgsl: &'static str` is satisfied without leaking.
const ALWAYS_PASSES_WGSL: &str = include_str!("test_shaders/always_passes.wgsl");
const ALWAYS_FAILS_WGSL: &str = include_str!("test_shaders/always_fails.wgsl");
const WORKGROUP_8X4_WGSL: &str = include_str!("test_shaders/workgroup_8x4.wgsl");

fn dummy_handle(ctx: &GpuContext) -> TextureHandle {
    TextureHandle::from_rgba8(ctx, 4, 4, &vec![0u8; 4 * 4 * 4])
}

#[test]
fn run_assertion_pass() {
    let ctx = GpuContext::shared();
    let outcome = run_assertion(
        &ctx,
        ShaderAssertion {
            shader_id: "test::always_passes",
            wgsl: ALWAYS_PASSES_WGSL,
            params: Vec::new(),
            frame: dummy_handle(&ctx),
            sidecar: None,
            reference: None,
        },
    )
    .expect("run");
    assert!(outcome.passed, "{outcome:?}");
    assert_eq!(outcome.reason_code, 0);
}

#[test]
fn run_assertion_fail_with_evidence() {
    let ctx = GpuContext::shared();
    let outcome = run_assertion(
        &ctx,
        ShaderAssertion {
            shader_id: "test::always_fails",
            wgsl: ALWAYS_FAILS_WGSL,
            params: Vec::new(),
            frame: dummy_handle(&ctx),
            sidecar: None,
            reference: None,
        },
    )
    .expect("run");
    assert!(!outcome.passed, "{outcome:?}");
    assert_eq!(outcome.evidence.len(), 2);
    assert!((outcome.evidence[0] - 0.25).abs() < 1e-6);
    assert!((outcome.evidence[1] - 0.75).abs() < 1e-6);
}

#[test]
fn run_assertion_batch_preserves_order() {
    let ctx = GpuContext::shared();
    let mk = |id: &'static str, wgsl: &'static str| ShaderAssertion {
        shader_id: id,
        wgsl,
        params: Vec::new(),
        frame: dummy_handle(&ctx),
        sidecar: None,
        reference: None,
    };
    let batch = vec![
        mk("test::batch::pass1", ALWAYS_PASSES_WGSL),
        mk("test::batch::fail1", ALWAYS_FAILS_WGSL),
        mk("test::batch::pass2", ALWAYS_PASSES_WGSL),
        mk("test::batch::fail2", ALWAYS_FAILS_WGSL),
    ];
    let outs = run_assertion_batch(&ctx, &batch).expect("batch");
    assert_eq!(outs.len(), 4);
    assert!(outs[0].passed);
    assert!(!outs[1].passed);
    assert!(outs[2].passed);
    assert!(!outs[3].passed);
}

#[test]
fn naga_reflection_reads_workgroup_size() {
    // If reflection fell back to the (8, 8) default, the shader still
    // runs correctly — `dispatch_workgroups` would over-dispatch by a
    // bit and the global-id-0 guard discards extras. So we can't infer
    // workgroup size from outcome alone; assert via the reflection fn.
    let (x, y, z) = super::runtime::reflect_workgroup_size(WORKGROUP_8X4_WGSL, "assert_main")
        .expect("reflect");
    assert_eq!((x, y, z), (8, 4, 1));

    // And a smoke test that the pipeline actually builds + dispatches.
    let ctx = GpuContext::shared();
    let outcome = run_assertion(
        &ctx,
        ShaderAssertion {
            shader_id: "test::workgroup_8x4",
            wgsl: WORKGROUP_8X4_WGSL,
            params: Vec::new(),
            frame: dummy_handle(&ctx),
            sidecar: None,
            reference: None,
        },
    )
    .expect("run");
    assert!(outcome.passed);
    assert_eq!(outcome.evidence, vec![8.0, 4.0]);
}

#[test]
fn pipeline_cache_reuses_compiled_pipeline() {
    let ctx = GpuContext::shared();
    let assertion = |sid: &'static str| ShaderAssertion {
        shader_id: sid,
        wgsl: ALWAYS_PASSES_WGSL,
        params: Vec::new(),
        frame: dummy_handle(&ctx),
        sidecar: None,
        reference: None,
    };
    // First dispatch compiles + caches.
    run_assertion(&ctx, assertion("test::cache::hit")).expect("first");
    let pipeline_a = ctx
        .get_or_compile("test::cache::hit", ALWAYS_PASSES_WGSL)
        .expect("cached");
    // Second dispatch should hit the cache. Confirm by Arc-pointer
    // equality on the cached entry pre/post.
    run_assertion(&ctx, assertion("test::cache::hit")).expect("second");
    let pipeline_b = ctx
        .get_or_compile("test::cache::hit", ALWAYS_PASSES_WGSL)
        .expect("cached");
    assert!(
        Arc::ptr_eq(&pipeline_a, &pipeline_b),
        "cache returned a different ComputePipeline on second call"
    );
}

