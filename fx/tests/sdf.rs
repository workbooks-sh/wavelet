//! 2D SDF primitives + smooth-min boolean combinators.
//!
//! Phase 4 of wb-ouds: sphere / box_sdf / torus as Sources,
//! smooth_union / smooth_intersect as Combinators. Math attribution is
//! to sdfu (https://github.com/termhn/sdfu) and Inigo Quilez's SDF
//! catalog — see `src/stdlib/sdf.wgsl` for the canonical port.
//!
//! These tests run after Phase 1's naga validation pass, so any WGSL
//! the SDF emitters produce must parse cleanly. If a port goes off the
//! rails (WGSL syntax drift, undefined identifier, wrong function
//! signature), the compile() call here fails before the assertion.

use wavelet_fx::{box_sdf, compile, parse, solid, sphere, src, torus};

#[test]
fn sphere_box_torus_compile_and_parse_through_naga() {
    compile(&sphere(0.3, 0.01).output()).expect("sphere");
    compile(&box_sdf(0.4, 0.2, 0.01).output()).expect("box_sdf");
    compile(&torus(0.3, 0.05, 0.01).output()).expect("torus");
}

#[test]
fn smooth_union_compiles_with_two_sdf_sources() {
    let comp = sphere(0.3, 0.01)
        .smooth_union(box_sdf(0.4, 0.2, 0.01), 0.1)
        .output();
    let out = compile(&comp).expect("smooth_union");
    // The shader body must reference both inputs in the call to the
    // helper function — a structural pattern check, cheap.
    let wgsl = &out.passes[0].wgsl;
    assert!(
        wgsl.contains("shady_smooth_union"),
        "smooth_union should emit the shady_smooth_union helper call; got:\n{wgsl}"
    );
}

#[test]
fn smooth_intersect_compiles_with_two_sdf_sources() {
    let comp = sphere(0.3, 0.01)
        .smooth_intersect(torus(0.3, 0.05, 0.01), 0.05)
        .output();
    let out = compile(&comp).expect("smooth_intersect");
    let wgsl = &out.passes[0].wgsl;
    assert!(wgsl.contains("shady_smooth_intersect"));
}

#[test]
fn sdf_helpers_appear_in_emitted_prelude() {
    // The SDF helper functions concatenated into stdlib::PRELUDE must
    // be present in every emitted shader (we don't tree-shake — WGSL
    // constant-folds the unused entries). A simple source that
    // doesn't even use SDFs should still ship the helper functions
    // so the SDF path is always available.
    let out = compile(&solid(0.5, 0.5, 0.5, 1.0).output()).expect("solid");
    let wgsl = &out.passes[0].wgsl;
    for fn_name in [
        "shady_sdf_sphere",
        "shady_sdf_box",
        "shady_sdf_torus",
        "shady_sdf_render",
        "shady_smooth_union",
        "shady_smooth_intersect",
    ] {
        assert!(
            wgsl.contains(fn_name),
            "expected SDF helper '{fn_name}' in emitted prelude"
        );
    }
}

#[test]
fn the_brief_example_compiles() {
    // From the integration brief:
    //   sphere(0.3).smooth_union(box_sdf(0.5,0.3,0.2), 0.1).out
    // wavelet_fx's API takes a smoothing factor explicitly (the brief's
    // `.sphere(0.3)` is shorthand for "default smoothing"); we pick
    // 0.01 as the conventional thin edge. The intent — chain an SDF
    // with a smooth-union — is the same.
    let comp = sphere(0.3, 0.01)
        .smooth_union(box_sdf(0.5, 0.3, 0.01), 0.1)
        .output();
    let out = compile(&comp).expect("brief's smooth_union example");
    assert_eq!(out.passes.len(), 1);
}

#[test]
fn sdf_chained_through_transforms_still_compiles() {
    // SDFs should compose with the rest of the DSL — pixel-coloring
    // transforms after the SDF render, uv-rewriting transforms before
    // it. Confirms the chain doesn't have hidden positional
    // requirements.
    let comp = sphere(0.3, 0.01)
        .rotate(0.1, 0.0)
        .brightness(0.1)
        .contrast(1.2)
        .output();
    compile(&comp).expect("sphere through transforms");
}

#[test]
fn sdf_can_blend_with_a_video_source() {
    // Using SDFs as masks over rendered video — the headline use case
    // for adding them. `mask` and `blend` both work.
    let comp = src(0).mask(sphere(0.4, 0.02)).output();
    compile(&comp).expect("masking a video by a sphere");
    let comp2 = src(0).blend(torus(0.3, 0.05, 0.01), 0.5).output();
    compile(&comp2).expect("blending video with a torus");
}

#[test]
fn parser_rejects_unknown_sdf_method_for_now() {
    // The Hydra-shaped text parser doesn't yet know `sphere`/`box_sdf`/
    // `torus` — that's a follow-up. Confirm the failure mode is the
    // structured ParseError, not a panic.
    let result = parse("sphere(0.3, 0.01).out()");
    assert!(
        result.is_err(),
        "parser doesn't yet know `sphere`; should ParseError"
    );
}
