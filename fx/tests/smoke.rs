//! End-to-end smoke test: builder → compile → EmitOutput.
//!
//! v0 emits a stub fragment shader and an empty render graph; this test
//! exists to keep the seams wired so v1 work can replace bodies one at a
//! time without breaking the API shape.

use wavelet_fx::{compile, from_buffer, noise, osc, prev, src, Composition, Diagnostic, Easing, TextureRef, Tween, Value};

#[test]
fn builder_to_emit_runs_end_to_end() {
    let comp = src(0).modulate(noise(4.0, 0.1), 0.02).output();

    let out = compile(&comp).expect("compile should succeed for a single-output comp");

    assert_eq!(out.passes.len(), 1, "v0 produces exactly one pass");
    assert!(!out.passes[0].wgsl.is_empty(), "pass body should be non-empty");
    assert!(out.passes[0].wgsl.contains("@fragment"));
}

#[test]
fn empty_composition_is_rejected() {
    let comp = Composition { outputs: vec![] };
    let err = compile(&comp).expect_err("empty composition must error");
    assert!(matches!(err, Diagnostic::InvalidComposition(_)));
}

#[test]
fn animato_tween_is_accepted_anywhere_a_constant_is() {
    // Same Tween type wavelet uses for DOM animation. The fact that this
    // type-checks and compiles is the integration: there is no separate
    // "shader time" or "shader animation" concept — WaveletFx reads the timeline
    // model straight from Animato.
    let pulse = Tween::new(0.0_f32, 0.1)
        .duration(2.0)
        .easing(Easing::EaseInOutSine)
        .build();

    let comp = src(0)
        .modulate(noise(4.0, 0.0), pulse)
        .contrast(1.1)
        .output();

    let out = compile(&comp).expect("compile should succeed with a tweened parameter");
    assert_eq!(out.passes.len(), 1);
}

#[test]
fn tweens_become_uniform_bindings_with_stable_names() {
    use wavelet_fx::UniformKind;

    let mod_depth = Tween::new(0.0_f32, 0.2).duration(2.0).build();
    let warmth = Tween::new(0.9_f32, 1.2).duration(4.0).build();

    let comp = src(0)
        .modulate(noise(4.0, 0.0), mod_depth)
        .color(warmth, 1.0, 1.0, 1.0)
        .output();

    let out = compile(&comp).expect("compile should succeed");

    // Two tweens in source order → two `u_tween_*` slots in source order.
    // Plus the always-present u_time + u_resolution.
    let tween_uniforms: Vec<_> = out
        .uniforms
        .iter()
        .filter(|u| matches!(u.kind, UniformKind::Tween(_)))
        .collect();
    assert_eq!(tween_uniforms.len(), 2);
    assert_eq!(tween_uniforms[0].name, "u_tween_0");
    assert_eq!(tween_uniforms[1].name, "u_tween_1");

    assert!(out
        .uniforms
        .iter()
        .any(|u| matches!(u.kind, UniformKind::Time)));
    assert!(out
        .uniforms
        .iter()
        .any(|u| matches!(u.kind, UniformKind::Resolution)));
}

#[test]
fn input_channels_are_collected_into_pass_inputs() {
    use wavelet_fx::TextureRef;

    let comp = src(0).modulate(src(1), 0.1).output();
    let out = compile(&comp).expect("compile should succeed");

    assert_eq!(
        out.passes[0].inputs,
        vec![TextureRef::InputChannel(0), TextureRef::InputChannel(1)]
    );
}

#[test]
fn const_only_compositions_have_no_tween_uniforms() {
    use wavelet_fx::UniformKind;

    let comp = src(0).modulate(noise(4.0, 0.0), 0.1).output();
    let out = compile(&comp).expect("compile should succeed");

    let tween_count = out
        .uniforms
        .iter()
        .filter(|u| matches!(u.kind, UniformKind::Tween(_)))
        .count();
    assert_eq!(tween_count, 0);
}

#[test]
fn bindings_match_the_emitted_wgsl_and_describe_every_texture() {
    // The whole point of `PassBindings` is the consumer can build a wgpu
    // BindGroup mechanically without parsing the WGSL string. Pin the
    // invariant: every `@binding(N)` declaration in the WGSL must appear
    // exactly once in `pass.bindings`, and the kinds line up.
    let pipeline = osc(3.0)
        .output_to("base")
        .and_then(src(0).blend(prev(), 0.4).output_to("echo"))
        .and_then(from_buffer("base").modulate(from_buffer("echo"), 0.05).output());

    let out = compile(&pipeline).expect("compile");
    assert_eq!(out.passes.len(), 3);

    for pass in &out.passes {
        // Count @binding declarations in the WGSL.
        let wgsl_bindings: Vec<u32> = pass
            .wgsl
            .lines()
            .filter_map(parse_binding_attr)
            .collect();

        // Build the expected list from PassBindings: uniforms slot plus one
        // texture + one sampler slot per TextureBinding, in declared order.
        let mut expected: Vec<u32> = vec![pass.bindings.uniforms.binding];
        for t in &pass.bindings.textures {
            expected.push(t.texture.binding);
            expected.push(t.sampler.binding);
        }
        assert_eq!(
            wgsl_bindings, expected,
            "pass '{}' bindings don't match WGSL: wgsl={:?} expected={:?}",
            pass.name, wgsl_bindings, expected
        );

        // pass.inputs and pass.bindings.textures describe the same textures
        // in the same order.
        let from_bindings: Vec<&TextureRef> =
            pass.bindings.textures.iter().map(|b| &b.source).collect();
        let from_inputs: Vec<&TextureRef> = pass
            .inputs
            .iter()
            .filter(|t| !matches!(t, TextureRef::SwapchainOrFinal))
            .collect();
        assert_eq!(from_bindings, from_inputs, "binding sources should mirror pass.inputs");
    }
}

fn parse_binding_attr(line: &str) -> Option<u32> {
    let s = line.trim();
    let i = s.find("@binding(")?;
    let after = &s[i + "@binding(".len()..];
    let end = after.find(')')?;
    after[..end].trim().parse().ok()
}

#[test]
fn const_and_tween_serialize_through_the_same_value() {
    let v_const: Value = 0.5_f32.into();
    let v_tween: Value = Tween::new(0.0_f32, 1.0).duration(1.0).build().into();

    assert!(!v_const.is_dynamic());
    assert!(v_tween.is_dynamic());

    // Both round-trip through serde — proves the EmitOutput spec stays
    // self-describing when tweens are present.
    let json_const = serde_json::to_string(&v_const).unwrap();
    let json_tween = serde_json::to_string(&v_tween).unwrap();
    assert!(json_const.contains("Const"));
    assert!(json_tween.contains("Tween"));
}
