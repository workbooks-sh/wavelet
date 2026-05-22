//! naga validation tests for wavelet_fx's emitted WGSL.
//!
//! Phase 1 of the WGSL ecosystem integration epic (wb-ouds): every
//! wavelet_fx compile that produces WGSL also runs naga's parser/validator
//! against it. Syntax / type errors caught at `compile()` time, not
//! at wgpu pipeline-creation time.
//!
//! These tests exercise the full surface — every Source, Transform,
//! and Combinator path that emits WGSL must produce a string naga
//! accepts. If naga rejects anything emitted by builder API or parser,
//! that's a wavelet_fx bug.

use wavelet_fx::{
    audio_fft, audio_rms, compile, compile_unvalidated, from_buffer, gradient, noise, osc,
    parse, prev, prop, seed, shape, solid, src, time_beat, voronoi, Composition, Diagnostic,
    Easing, Tween,
};

/// The headline integration test for Phase 1. A wide net of real
/// authoring shapes — sources, transforms, combinators, multi-pass,
/// dynamic uniforms — must all compile and pass naga validation.
#[test]
fn wgsl_emits_valid_naga_parseable() {
    // Single source.
    compile(&src(0).output()).expect("plain src(0).output()");

    // Every source primitive in isolation.
    compile(&osc(3.0).output()).expect("osc");
    compile(&noise(4.0, 0.1).output()).expect("noise");
    compile(&voronoi(5.0, 0.1, 0.3).output()).expect("voronoi");
    compile(&gradient(0.5).output()).expect("gradient");
    compile(&shape(6, 0.3, 0.01).output()).expect("shape");
    compile(&solid(0.5, 0.6, 0.7, 1.0).output()).expect("solid");

    // Transform chain — rotate, scale, scroll, pixelate, repeat, color,
    // brightness, contrast, invert.
    let chain = src(0)
        .rotate(0.1, 0.05)
        .scale(1.2)
        .scroll(0.0, 0.0, 0.1, 0.0)
        .pixelate(80.0, 60.0)
        .repeat(2.0, 2.0, 0.0, 0.0)
        .color(1.0, 0.9, 0.8, 1.0)
        .brightness(0.1)
        .contrast(1.2)
        .invert(0.5)
        .output();
    compile(&chain).expect("transform chain");

    // Every combinator.
    compile(&src(0).add(src(1), 0.5).output()).expect("add");
    compile(&src(0).mult(src(1), 0.5).output()).expect("mult");
    compile(&src(0).blend(src(1), 0.5).output()).expect("blend");
    compile(&src(0).diff(src(1)).output()).expect("diff");
    compile(&src(0).mask(src(1)).output()).expect("mask");
    compile(&src(0).modulate(noise(4.0, 0.0), 0.1).output()).expect("modulate");
    compile(&src(0).modulate_scale(noise(4.0, 0.0), 0.5, 1.0).output())
        .expect("modulate_scale");
    compile(&src(0).modulate_rotate(noise(4.0, 0.0), 0.5, 0.0).output())
        .expect("modulate_rotate");

    // Dynamic uniforms — tween + uniform refs.
    let tween = Tween::new(0.0_f32, 0.2)
        .duration(2.0)
        .easing(Easing::EaseInOutSine)
        .build();
    compile(
        &src(0)
            .modulate(noise(4.0, 0.0), tween)
            .color(audio_rms(), audio_fft(3), time_beat(), seed())
            .brightness(prop("--energy"))
            .output(),
    )
    .expect("dynamic uniforms");

    // Multi-pass with feedback (`prev()`).
    let multi = osc(3.0)
        .output_to("base")
        .and_then(src(0).blend(prev(), 0.4).output_to("echo"))
        .and_then(
            from_buffer("base")
                .modulate(from_buffer("echo"), 0.05)
                .output(),
        );
    compile(&multi).expect("multi-pass with feedback");
}

/// Hydra-shaped text parser path: a representative snippet must parse,
/// compile, and validate.
#[test]
fn parser_output_passes_naga() {
    let text = "src(0).modulate(noise(4, 0.1), 0.02).contrast(1.1).out()";
    let comp = parse(text).expect("parse");
    compile(&comp).expect("parsed comp must compile + validate");
}

/// The escape hatch: when wavelet_fx's emit pipeline is deliberately fed a
/// pass with broken WGSL, [`compile`] must surface
/// [`Diagnostic::InvalidEmittedWgsl`] rather than silently accepting it.
/// We forge an EmitOutput post-emit because wavelet_fx's stdlib currently
/// emits only valid WGSL — the test mimics what would happen if a
/// future `@raw-wgsl { ... }` block (planned in SHADY.md) embedded
/// invalid syntax.
#[test]
fn broken_wgsl_is_rejected_by_validate() {
    use wavelet_fx::emit::{validate_with_naga, EmitOutput, EmittedPass, PassBindings, BindingSlot};

    let bad_pass = EmittedPass {
        name: "deliberately_broken".to_string(),
        wgsl: "fn this is not wgsl at all".to_string(),
        inputs: vec![],
        output: wavelet_fx::TextureRef::SwapchainOrFinal,
        bindings: PassBindings {
            uniforms: BindingSlot { group: 0, binding: 0 },
            textures: vec![],
        },
        pre_effects: vec![],
    };
    let out = EmitOutput {
        passes: vec![bad_pass],
        uniforms: vec![],
        buffers: vec![],
    };

    let err = validate_with_naga(&out).expect_err("must reject broken WGSL");
    match err {
        Diagnostic::InvalidEmittedWgsl { pass_name, message } => {
            assert_eq!(pass_name, "deliberately_broken");
            assert!(
                !message.is_empty(),
                "naga's error message should be non-empty"
            );
        }
        other => panic!("expected InvalidEmittedWgsl, got {:?}", other),
    }
}

/// `compile_unvalidated` exists so harnesses can inspect raw emit
/// output without naga. Confirm it doesn't run validation by passing it
/// a composition that wavelet_fx accepts (proving the surface is wired) —
/// negative-path coverage comes from `broken_wgsl_is_rejected_by_validate`.
#[test]
fn compile_unvalidated_skips_naga() {
    let comp = src(0).output();
    let out = compile_unvalidated(&comp).expect("compile_unvalidated");
    assert_eq!(out.passes.len(), 1);
}

/// The Bevy-lifted separable Gaussian WGSL in `stdlib::blur` must parse
/// cleanly through naga in isolation — consumers compose it into their
/// own fragment shaders (see `wavelet::shader::gpu_blur`), and a syntax
/// drift here would silently break GPU blur for everyone.
#[test]
fn stdlib_blur_helpers_parse_through_naga() {
    // Wrap each helper in a minimal fragment-shader skeleton so it has
    // a valid context (binding declarations + a main entry point).
    let wrap = |helper: &str, entry: &str| -> String {
        format!(
            r#"
struct U {{ sigma: f32, _pad: f32, resolution: vec2<f32> }};
@group(0) @binding(0) var<uniform> u: U;
@group(0) @binding(1) var src: texture_2d<f32>;
@group(0) @binding(2) var smp: sampler;

{helper}

@fragment
fn fs_main(@location(0) uv: vec2<f32>) -> @location(0) vec4<f32> {{
  return {entry}(src, smp, uv, u.sigma, u.resolution);
}}
"#
        )
    };

    let h = wrap(wavelet_fx::stdlib::blur::SEPARABLE_GAUSSIAN_H, "fx_blur_h");
    let v = wrap(wavelet_fx::stdlib::blur::SEPARABLE_GAUSSIAN_V, "fx_blur_v");

    naga::front::wgsl::parse_str(&h)
        .expect("SEPARABLE_GAUSSIAN_H must parse — Bevy-lifted blur shader is broken");
    naga::front::wgsl::parse_str(&v)
        .expect("SEPARABLE_GAUSSIAN_V must parse — Bevy-lifted blur shader is broken");
}

/// `src(N).blur(R)` emits `PreEffect::GpuBlur` (Phase 2 default), not
/// the CPU fallback. Confirms wavelet_fx's policy when the GPU path applies.
#[test]
fn blur_on_src_emits_gpu_pre_effect() {
    use wavelet_fx::PreEffect;
    let comp = src(0).blur(8.0).output();
    let out = compile(&comp).expect("compile");
    let pass = &out.passes[0];
    assert_eq!(pass.pre_effects.len(), 1);
    match pass.pre_effects[0] {
        PreEffect::GpuBlur { input_channel, radius } => {
            assert_eq!(input_channel, 0);
            assert!((radius - 8.0).abs() < f32::EPSILON);
        }
        ref other => panic!("expected GpuBlur, got {other:?}"),
    }
}

/// Sanity check that the existing arizona-shape demo composition
/// (the one wavelet's TransitionPipeline drives in examples) passes
/// naga end-to-end. If this ever breaks, the failure should land
/// here, not in a wgpu submission deep in a render loop.
#[test]
fn representative_transition_pipeline_passes_naga() {
    let comp: Composition = src(0)
        .blend(src(1), 0.5)
        .contrast(1.1)
        .brightness(0.0)
        .output();
    let out = compile(&comp).expect("transition-shaped comp");
    assert!(!out.passes.is_empty());
    for pass in &out.passes {
        assert!(pass.wgsl.contains("@fragment"));
    }
}
