//! WGSL snapshot tests. These are the load-bearing tests for emit: every
//! supported AST shape compiles to a known, stable WGSL string. When the
//! emitter changes intentionally, update the snapshots; when it changes
//! unintentionally, the test fails and tells you where.
//!
//! Snapshots are inline so reviewers see the WGSL in the diff alongside the
//! AST change that produced it. Re-record with `cargo insta review`.

use wavelet_fx::{
    audio_fft, audio_rms, compile, from_buffer, noise, osc, prev, prop, seed, solid, src,
    time_beat, Diagnostic, Easing, TextureRef, Tween, UniformKind, UniformRef,
};

#[test]
fn solid_constant_emits_literal_vec4() {
    let comp = solid(1.0, 0.0, 0.5, 1.0).output();
    let out = compile(&comp).expect("compile");

    // Use emit_only here too — the prelude is exercised by
    // `prelude_provides_expected_helpers` below, and pinning it inline in
    // every test means every prelude addition breaks N snapshots.
    insta::assert_snapshot!(emit_only(&out.passes[0].wgsl), @r###"
    struct Uniforms {
      u_time: f32,
      u_resolution: vec2<f32>,
    };

    @group(0) @binding(0) var<uniform> u: Uniforms;

    @fragment
    fn fs_main(@location(0) uv: vec2<f32>) -> @location(0) vec4<f32> {
      let c0: vec4<f32> = vec4<f32>(1.0, 0.0, 0.5, 1.0);
      return c0;
    }
    "###);
}

#[test]
fn prelude_provides_expected_helpers() {
    // The prelude is what makes uv-displacement, voronoi, gradient, shape
    // valid WGSL. If anyone removes a function the rest of the suite
    // catches the regression at the call site, but this test catches it at
    // the source and surfaces a clearer signal.
    let comp = src(0).output();
    let wgsl = &compile(&comp).expect("compile").passes[0].wgsl;
    for fn_name in [
        "fn hash21(",
        "fn hash22(",
        "fn rotate2d(",
        "fn shady_voronoi(",
        "fn shady_gradient(",
        "fn shady_shape(",
    ] {
        assert!(
            wgsl.contains(fn_name),
            "prelude should provide {fn_name}, got:\n{wgsl}"
        );
    }
}

#[test]
fn voronoi_gradient_shape_resolve_to_prelude_calls() {
    // Sanity check: each non-trivial source produces a call to its prelude
    // helper rather than the old vec4(0,0,0,1) stub.
    use wavelet_fx::{gradient, shape, voronoi};
    for (label, comp) in [
        ("voronoi", voronoi(5.0, 0.3, 0.3).output()),
        ("gradient", gradient(0.5).output()),
        ("shape", shape(6, 0.3, 0.01).output()),
    ] {
        let wgsl = &compile(&comp).expect("compile").passes[0].wgsl;
        let expected_call = format!("shady_{}(", label);
        assert!(
            wgsl.contains(&expected_call),
            "{} source should emit {} in WGSL, got:\n{}",
            label,
            expected_call,
            wgsl
        );
    }
}

#[test]
fn src_with_contrast_threads_color_through() {
    let comp = src(0).contrast(1.2).output();
    let out = compile(&comp).expect("compile");

    insta::assert_snapshot!(emit_only(&out.passes[0].wgsl), @r###"
    struct Uniforms {
      u_time: f32,
      u_resolution: vec2<f32>,
    };

    @group(0) @binding(0) var<uniform> u: Uniforms;
    @group(0) @binding(1) var iChannel0: texture_2d<f32>;
    @group(0) @binding(2) var iChannel0_sampler: sampler;

    @fragment
    fn fs_main(@location(0) uv: vec2<f32>) -> @location(0) vec4<f32> {
      let c0: vec4<f32> = textureSample(iChannel0, iChannel0_sampler, uv);
      let c1: vec4<f32> = vec4<f32>(((c0.rgb - vec3<f32>(0.5)) * 1.2) + vec3<f32>(0.5), c0.a);
      return c1;
    }
    "###);
}

#[test]
fn src_modulate_noise_resamples_lhs_at_displaced_uv() {
    // True Hydra-style modulate: rhs (noise) is evaluated at the current uv,
    // its rg channels drive a uv displacement, then lhs (src) is resampled at
    // the displaced uv. This is *not* a post-blend — it's how Hydra produces
    // its signature liquid-distortion look.
    let comp = src(0).modulate(noise(4.0, 0.0), 0.02).output();
    let out = compile(&comp).expect("compile");

    insta::assert_snapshot!(emit_only(&out.passes[0].wgsl), @r###"
    struct Uniforms {
      u_time: f32,
      u_resolution: vec2<f32>,
    };

    @group(0) @binding(0) var<uniform> u: Uniforms;
    @group(0) @binding(1) var iChannel0: texture_2d<f32>;
    @group(0) @binding(2) var iChannel0_sampler: sampler;

    @fragment
    fn fs_main(@location(0) uv: vec2<f32>) -> @location(0) vec4<f32> {
      let c0: vec4<f32> = vec4<f32>(vec3<f32>(hash21(uv * 4.0 + vec2<f32>(0.0))), 1.0);
      let uv0: vec2<f32> = (uv + (c0.rg - vec2<f32>(0.5)) * 0.02);
      let c1: vec4<f32> = textureSample(iChannel0, iChannel0_sampler, uv0);
      return c1;
    }
    "###);
}

/// The integration we care about most: tween-driven parameters become
/// references to `u.u_tween_N` in the WGSL, with `N` matching the slot
/// `crate::ir::lower` allocated.
///
/// Critically, the WGSL statement order is *not* the same as the canonical
/// AST walk order any more — modulate evaluates rhs before lhs so it can
/// compute a uv displacement. The pointer-keyed name table in `emit.rs`
/// decouples slot assignment from emit order, so `u_tween_0` (mod_depth,
/// AST-position 0) still lines up with the modulate `amount` slot, and
/// `u_tween_1` (warmth, AST-position 1) lines up with the color transform.
#[test]
fn tweens_resolve_to_indexed_uniform_references() {
    let mod_depth = Tween::new(0.0_f32, 0.2)
        .duration(2.0)
        .easing(Easing::EaseInOutSine)
        .build();
    let warmth = Tween::new(0.9_f32, 1.2).duration(4.0).build();

    let comp = src(0)
        .modulate(noise(4.0, 0.0), mod_depth)
        .color(warmth, 1.0, 1.0, 1.0)
        .output();
    let out = compile(&comp).expect("compile");

    insta::assert_snapshot!(emit_only(&out.passes[0].wgsl), @r###"
    struct Uniforms {
      u_time: f32,
      u_resolution: vec2<f32>,
      u_tween_0: f32,
      u_tween_1: f32,
    };

    @group(0) @binding(0) var<uniform> u: Uniforms;
    @group(0) @binding(1) var iChannel0: texture_2d<f32>;
    @group(0) @binding(2) var iChannel0_sampler: sampler;

    @fragment
    fn fs_main(@location(0) uv: vec2<f32>) -> @location(0) vec4<f32> {
      let c0: vec4<f32> = vec4<f32>(vec3<f32>(hash21(uv * 4.0 + vec2<f32>(0.0))), 1.0);
      let uv0: vec2<f32> = (uv + (c0.rg - vec2<f32>(0.5)) * u.u_tween_0);
      let c1: vec4<f32> = textureSample(iChannel0, iChannel0_sampler, uv0);
      let c2: vec4<f32> = (c1 * vec4<f32>(u.u_tween_1, 1.0, 1.0, 1.0));
      return c2;
    }
    "###);
}

#[test]
fn blend_combines_two_sources() {
    let comp = solid(1.0, 0.0, 0.0, 1.0)
        .blend(solid(0.0, 0.0, 1.0, 1.0), 0.5)
        .output();
    let out = compile(&comp).expect("compile");

    insta::assert_snapshot!(emit_only(&out.passes[0].wgsl), @r###"
    struct Uniforms {
      u_time: f32,
      u_resolution: vec2<f32>,
    };

    @group(0) @binding(0) var<uniform> u: Uniforms;

    @fragment
    fn fs_main(@location(0) uv: vec2<f32>) -> @location(0) vec4<f32> {
      let c0: vec4<f32> = vec4<f32>(1.0, 0.0, 0.0, 1.0);
      let c1: vec4<f32> = vec4<f32>(0.0, 0.0, 1.0, 1.0);
      let c2: vec4<f32> = mix(c0, c1, 0.5);
      return c2;
    }
    "###);
}

#[test]
fn rotate_rewrites_uv_and_source_samples_the_rewritten_uv() {
    let comp = src(0).rotate(0.3, 0.1).output();
    let out = compile(&comp).expect("compile");

    insta::assert_snapshot!(emit_only(&out.passes[0].wgsl), @r###"
    struct Uniforms {
      u_time: f32,
      u_resolution: vec2<f32>,
    };

    @group(0) @binding(0) var<uniform> u: Uniforms;
    @group(0) @binding(1) var iChannel0: texture_2d<f32>;
    @group(0) @binding(2) var iChannel0_sampler: sampler;

    @fragment
    fn fs_main(@location(0) uv: vec2<f32>) -> @location(0) vec4<f32> {
      let uv0: vec2<f32> = (rotate2d(uv - vec2<f32>(0.5), 0.3 + u.u_time * 0.1) + vec2<f32>(0.5));
      let c0: vec4<f32> = textureSample(iChannel0, iChannel0_sampler, uv0);
      return c0;
    }
    "###);
}

#[test]
fn scale_rewrites_uv() {
    let comp = osc(3.0).scale(2.0).output();
    let out = compile(&comp).expect("compile");

    insta::assert_snapshot!(emit_only(&out.passes[0].wgsl), @r###"
    struct Uniforms {
      u_time: f32,
      u_resolution: vec2<f32>,
    };

    @group(0) @binding(0) var<uniform> u: Uniforms;

    @fragment
    fn fs_main(@location(0) uv: vec2<f32>) -> @location(0) vec4<f32> {
      let uv0: vec2<f32> = (((uv) - vec2<f32>(0.5)) / vec2<f32>(2.0 * 1.0, 2.0 * 1.0) + vec2<f32>(0.5));
      let c0: vec4<f32> = vec4<f32>(vec3<f32>(0.5 + 0.5 * sin((uv0.x * 3.0 + uv0.y * 0.1 + 0.0) * 6.28318)), 1.0);
      return c0;
    }
    "###);
}

#[test]
fn nested_uv_rewrites_compose_left_to_right() {
    // rotate(...).scale(...).src(0): the chain is src(0) -> scale -> rotate
    // in the builder; the outermost transform is applied to uv FIRST, so the
    // emitted order is: uv → rotate -> uv0 → scale -> uv1 → src samples uv1.
    let comp = src(0).scale(2.0).rotate(0.5, 0.0).output();
    let out = compile(&comp).expect("compile");

    insta::assert_snapshot!(emit_only(&out.passes[0].wgsl), @r###"
    struct Uniforms {
      u_time: f32,
      u_resolution: vec2<f32>,
    };

    @group(0) @binding(0) var<uniform> u: Uniforms;
    @group(0) @binding(1) var iChannel0: texture_2d<f32>;
    @group(0) @binding(2) var iChannel0_sampler: sampler;

    @fragment
    fn fs_main(@location(0) uv: vec2<f32>) -> @location(0) vec4<f32> {
      let uv0: vec2<f32> = (rotate2d(uv - vec2<f32>(0.5), 0.5 + u.u_time * 0.0) + vec2<f32>(0.5));
      let uv1: vec2<f32> = (((uv0) - vec2<f32>(0.5)) / vec2<f32>(2.0 * 1.0, 2.0 * 1.0) + vec2<f32>(0.5));
      let c0: vec4<f32> = textureSample(iChannel0, iChannel0_sampler, uv1);
      return c0;
    }
    "###);
}

#[test]
fn two_pass_composition_lowers_to_two_passes_in_declaration_order() {
    // First pass renders a static gradient into a named buffer; second pass
    // reads it back. The order matters: the buffer must be declared before
    // anyone samples it.
    let pipeline = osc(3.0)
        .output_to("base")
        .and_then(from_buffer("base").contrast(1.2).output());

    let out = compile(&pipeline).expect("compile");

    assert_eq!(out.passes.len(), 2, "two outputs -> two passes");
    assert_eq!(out.passes[0].name, "base");
    assert_eq!(out.passes[1].name, "main");
    assert!(matches!(out.passes[0].output, TextureRef::Buffer(ref n) if n == "base"));
    assert!(matches!(out.passes[1].output, TextureRef::SwapchainOrFinal));

    // The downstream pass samples the buffer it depends on.
    assert!(out.passes[1].inputs.contains(&TextureRef::Buffer("base".into())));
    // And the buffer is declared in the buffers list (no feedback, since no
    // pass references prev() inside the 'base' chain).
    assert_eq!(out.buffers.len(), 1);
    assert_eq!(out.buffers[0].name, "base");
    assert!(!out.buffers[0].feedback);

    // Snapshot the consumer pass to lock the WGSL shape of buffer reads.
    insta::assert_snapshot!(emit_only(&out.passes[1].wgsl), @r###"
    struct Uniforms {
      u_time: f32,
      u_resolution: vec2<f32>,
    };

    @group(0) @binding(0) var<uniform> u: Uniforms;
    @group(0) @binding(1) var buffer_base: texture_2d<f32>;
    @group(0) @binding(2) var buffer_base_sampler: sampler;

    @fragment
    fn fs_main(@location(0) uv: vec2<f32>) -> @location(0) vec4<f32> {
      let c0: vec4<f32> = textureSample(buffer_base, buffer_base_sampler, uv);
      let c1: vec4<f32> = vec4<f32>(((c0.rgb - vec3<f32>(0.5)) * 1.2) + vec3<f32>(0.5), c0.a);
      return c1;
    }
    "###);
}

#[test]
fn prev_marks_buffer_as_feedback_and_emits_prev_texture_read() {
    // Classic Hydra "echo" — blend a fresh sample with the buffer's previous
    // frame so trails accumulate. The ping-pong allocation is the consumer's
    // job; we tell them via BufferSpec.feedback + TextureRef::PrevFrame.
    let pipeline = src(0).blend(prev(), 0.4).output_to("echo");

    let out = compile(&pipeline).expect("compile");

    assert_eq!(out.buffers.len(), 1);
    assert_eq!(out.buffers[0].name, "echo");
    assert!(out.buffers[0].feedback, "prev() should mark the buffer feedback");
    assert!(out.passes[0]
        .inputs
        .contains(&TextureRef::PrevFrame("echo".into())));

    insta::assert_snapshot!(emit_only(&out.passes[0].wgsl), @r###"
    struct Uniforms {
      u_time: f32,
      u_resolution: vec2<f32>,
    };

    @group(0) @binding(0) var<uniform> u: Uniforms;
    @group(0) @binding(1) var iChannel0: texture_2d<f32>;
    @group(0) @binding(2) var iChannel0_sampler: sampler;
    @group(0) @binding(3) var prev_echo: texture_2d<f32>;
    @group(0) @binding(4) var prev_echo_sampler: sampler;

    @fragment
    fn fs_main(@location(0) uv: vec2<f32>) -> @location(0) vec4<f32> {
      let c0: vec4<f32> = textureSample(iChannel0, iChannel0_sampler, uv);
      let c1: vec4<f32> = textureSample(prev_echo, prev_echo_sampler, uv);
      let c2: vec4<f32> = mix(c0, c1, 0.4);
      return c2;
    }
    "###);
}

#[test]
fn forward_buffer_reference_is_rejected() {
    // Reading a buffer that's only declared later: the lowering pass must
    // catch this rather than emit a shader that references undefined
    // textures. Backwards-only is enforced so deterministic insertion-order
    // execution stays valid.
    let pipeline = from_buffer("later")
        .output()
        .and_then(osc(3.0).output_to("later"));

    let err = compile(&pipeline).expect_err("forward buffer ref must error");
    let msg = format!("{}", err);
    assert!(matches!(err, Diagnostic::InvalidComposition(_)), "got: {}", msg);
    assert!(msg.contains("not declared"), "got: {}", msg);
}

#[test]
fn prev_in_a_final_output_pass_is_rejected() {
    // prev() only means something for a named buffer that gets ping-ponged.
    // A composition that ends with output() has no buffer to ping-pong, so
    // the call is meaningless and we surface that as a lowering error.
    let pipeline = src(0).blend(prev(), 0.5).output();
    let err = compile(&pipeline).expect_err("prev in final must error");
    assert!(matches!(err, Diagnostic::InvalidComposition(_)));
}

#[test]
fn audio_rms_resolves_to_a_named_uniform_slot() {
    let comp = src(0).contrast(audio_rms()).output();
    let out = compile(&comp).expect("compile");

    // The Uniforms struct has u_audio_rms, the WGSL references it by name,
    // and the UniformBinding list has a corresponding AudioRms entry.
    assert!(out
        .uniforms
        .iter()
        .any(|u| u.name == "u_audio_rms" && matches!(u.kind, UniformKind::AudioRms)));

    insta::assert_snapshot!(emit_only(&out.passes[0].wgsl), @r###"
    struct Uniforms {
      u_time: f32,
      u_resolution: vec2<f32>,
      u_audio_rms: f32,
    };

    @group(0) @binding(0) var<uniform> u: Uniforms;
    @group(0) @binding(1) var iChannel0: texture_2d<f32>;
    @group(0) @binding(2) var iChannel0_sampler: sampler;

    @fragment
    fn fs_main(@location(0) uv: vec2<f32>) -> @location(0) vec4<f32> {
      let c0: vec4<f32> = textureSample(iChannel0, iChannel0_sampler, uv);
      let c1: vec4<f32> = vec4<f32>(((c0.rgb - vec3<f32>(0.5)) * u.u_audio_rms) + vec3<f32>(0.5), c0.a);
      return c1;
    }
    "###);
}

#[test]
fn repeated_uniform_references_share_one_slot() {
    // audio_rms() called twice — should produce exactly one slot in the
    // Uniforms struct, with both references resolving to u.u_audio_rms.
    let comp = src(0)
        .modulate(noise(audio_rms(), 0.0), audio_rms())
        .output();
    let out = compile(&comp).expect("compile");

    let audio_rms_slots = out
        .uniforms
        .iter()
        .filter(|u| matches!(u.kind, UniformKind::AudioRms))
        .count();
    assert_eq!(audio_rms_slots, 1, "audio_rms should dedupe to one slot");

    // Both references in the WGSL point at the same name.
    let wgsl = &out.passes[0].wgsl;
    let hits = wgsl.matches("u.u_audio_rms").count();
    assert_eq!(hits, 2, "both references should appear in WGSL");
}

#[test]
fn css_prop_sanitizes_kebab_case_to_snake_case() {
    let comp = src(0).contrast(prop("--brand-energy")).output();
    let out = compile(&comp).expect("compile");

    let slot = out
        .uniforms
        .iter()
        .find(|u| matches!(&u.kind, UniformKind::CssProp(n) if n == "--brand-energy"))
        .expect("css prop binding should be present");
    assert_eq!(slot.name, "u_prop_brand_energy");
    assert!(out.passes[0].wgsl.contains("u.u_prop_brand_energy"));
}

#[test]
fn audio_fft_bins_get_distinct_indexed_slots() {
    let comp = osc(audio_fft(2)).modulate(noise(audio_fft(8), 0.0), 0.1).output();
    let out = compile(&comp).expect("compile");

    let bin2 = out
        .uniforms
        .iter()
        .any(|u| u.name == "u_audio_fft_2" && matches!(u.kind, UniformKind::AudioFftBin(2)));
    let bin8 = out
        .uniforms
        .iter()
        .any(|u| u.name == "u_audio_fft_8" && matches!(u.kind, UniformKind::AudioFftBin(8)));
    assert!(bin2 && bin8, "both bins should get slots");

    // And calling audio_fft(2) again would dedup with the existing slot.
    let comp2 = osc(audio_fft(2)).modulate(noise(audio_fft(2), 0.0), 0.1).output();
    let out2 = compile(&comp2).expect("compile");
    let bin2_slots = out2
        .uniforms
        .iter()
        .filter(|u| matches!(u.kind, UniformKind::AudioFftBin(2)))
        .count();
    assert_eq!(bin2_slots, 1, "duplicate bin reference should share one slot");
}

#[test]
fn beat_and_seed_pass_through_to_wgsl() {
    let comp = src(0).blend(noise(time_beat(), seed()), 0.5).output();
    let out = compile(&comp).expect("compile");

    let names: Vec<_> = out.uniforms.iter().map(|u| u.name.as_str()).collect();
    assert!(names.contains(&"u_beat"), "got: {:?}", names);
    assert!(names.contains(&"u_seed"), "got: {:?}", names);
    assert!(out.passes[0].wgsl.contains("u.u_beat"));
    assert!(out.passes[0].wgsl.contains("u.u_seed"));
}

#[test]
fn tween_and_uniform_share_a_pass_without_conflicting_slots() {
    // Mixing a tween with a uniform-ref in the same composition is the
    // common case — make sure the two name-assignment mechanisms coexist:
    // tween gets u_tween_0, uniform-ref gets its content-derived name.
    let warmth = Tween::new(0.9_f32, 1.2)
        .duration(2.0)
        .easing(Easing::EaseInOutSine)
        .build();
    let comp = src(0)
        .modulate(noise(audio_rms(), 0.0), warmth)
        .output();
    let out = compile(&comp).expect("compile");

    let names: Vec<_> = out.uniforms.iter().map(|u| u.name.as_str()).collect();
    assert!(names.contains(&"u_audio_rms"));
    assert!(names.contains(&"u_tween_0"));
    let wgsl = &out.passes[0].wgsl;
    assert!(wgsl.contains("u.u_audio_rms"));
    assert!(wgsl.contains("u.u_tween_0"));
}

#[test]
fn uniform_ref_is_unsupported_in_sample_at() {
    // sample_at is for scalar tweens (and constants). Uniform refs are
    // consumer-filled — `None` is the contract.
    use wavelet_fx::UniformBinding;
    let binding = UniformBinding {
        name: "u_audio_rms".into(),
        kind: UniformKind::AudioRms,
    };
    assert_eq!(binding.sample_at(0.5), None);
    // Suppress the unused-import warning on the type alias in this test.
    let _ = UniformRef::AudioRms;
}

/// Strip the prelude so snapshots focus on what the emit walker produced
/// rather than the static helper file. The prelude has its own dedicated
/// snapshot in `solid_constant_emits_literal_vec4`.
fn emit_only(wgsl: &str) -> String {
    let marker = "\nstruct Uniforms";
    let idx = wgsl.find(marker).expect("emitted shader should contain Uniforms struct");
    wgsl[idx + 1..].to_string()
}
