//! Text-parser tests. The parser is a thin frontend that produces the same
//! AST as the builder — so the load-bearing assertion is "parsed program +
//! compile produces the same EmitOutput as a hand-built composition." We
//! verify by comparing emitted WGSL.

use wavelet_fx::{compile, from_buffer, noise, parse, src, Diagnostic, UniformKind};

#[test]
fn empty_program_is_rejected() {
    let err = parse("").expect_err("empty source must error");
    assert!(matches!(err, Diagnostic::InvalidComposition(_)));
}

#[test]
fn program_without_out_call_is_rejected() {
    // A free-standing chain that never terminates in .out() can't contribute
    // to the composition. The parser surfaces this on the statement (as a
    // ParseError pointing at the offending expression) — more useful than a
    // generic "no outputs" error at compile time.
    let err = parse("noise(4, 0.1).contrast(1.1)").expect_err("missing .out() must error");
    assert!(
        matches!(err, Diagnostic::ParseError { .. } | Diagnostic::InvalidComposition(_)),
        "got {:?}",
        err
    );
}

#[test]
fn unknown_function_errors_with_line_col() {
    let err = parse("plasma(1.0).out()").expect_err("unknown function must error");
    match err {
        Diagnostic::ParseError { line, col, message } => {
            assert_eq!(line, 1);
            assert!(col >= 1);
            assert!(message.contains("plasma"), "got: {}", message);
        }
        other => panic!("expected ParseError, got {:?}", other),
    }
}

#[test]
fn unterminated_string_reports_position() {
    let err = parse("from_buffer(\"oops").expect_err("unterminated string must error");
    assert!(matches!(err, Diagnostic::ParseError { .. }));
}

#[test]
fn simple_chain_parses_and_compiles() {
    let comp = parse("noise(4, 0.0).contrast(1.1).out()").expect("parse");
    let out = compile(&comp).expect("compile");
    assert_eq!(out.passes.len(), 1);
    assert!(out.passes[0].wgsl.contains("hash21"));
    assert!(out.passes[0].wgsl.contains("0.5)) * 1.1"));
}

#[test]
fn parsed_program_matches_builder_emit_byte_for_byte() {
    // This is the test that earns the parser its keep: if surface syntax
    // produces a different AST shape from the equivalent builder calls, the
    // emitted WGSL would differ. We pin them equal.
    let parsed = parse(
        r#"
        let n = noise(4, 0.0)
        src(0).modulate(n, 0.02).contrast(1.1).out()
        "#,
    )
    .expect("parse");
    let built = src(0).modulate(noise(4.0, 0.0), 0.02).contrast(1.1).output();

    let parsed_wgsl = &compile(&parsed).expect("parse compile").passes[0].wgsl;
    let built_wgsl = &compile(&built).expect("build compile").passes[0].wgsl;
    assert_eq!(parsed_wgsl, built_wgsl);
}

#[test]
fn comments_and_blank_lines_are_skipped() {
    let comp = parse(
        r#"
        // this is a comment
        let s = src(0)

        // another comment
        s.contrast(1.2).out()
        "#,
    )
    .expect("parse");
    let out = compile(&comp).expect("compile");
    assert_eq!(out.passes.len(), 1);
}

#[test]
fn multi_pass_composition_via_parser() {
    let comp = parse(
        r#"
        osc(3.0).out("base")
        from_buffer("base").contrast(1.2).out()
        "#,
    )
    .expect("parse");
    let out = compile(&comp).expect("compile");
    assert_eq!(out.passes.len(), 2);
    assert_eq!(out.passes[0].name, "base");
    assert_eq!(out.passes[1].name, "main");

    // Builder equivalent should produce the same WGSL.
    use wavelet_fx::osc;
    let built = osc(3.0)
        .output_to("base")
        .and_then(from_buffer("base").contrast(1.2).output());
    let built_out = compile(&built).expect("compile");
    assert_eq!(out.passes[0].wgsl, built_out.passes[0].wgsl);
    assert_eq!(out.passes[1].wgsl, built_out.passes[1].wgsl);
}

#[test]
fn semicolons_or_newlines_separate_statements() {
    let a = parse("let n = noise(4, 0); src(0).modulate(n, 0.1).out()").expect("parse a");
    let b = parse(
        r#"
        let n = noise(4, 0)
        src(0).modulate(n, 0.1).out()
        "#,
    )
    .expect("parse b");
    let wgsl_a = &compile(&a).unwrap().passes[0].wgsl;
    let wgsl_b = &compile(&b).unwrap().passes[0].wgsl;
    assert_eq!(wgsl_a, wgsl_b);
}

#[test]
fn parser_recognizes_dynamic_uniform_constructors() {
    let comp = parse(
        r#"
        src(0).contrast(audio_rms()).modulate(noise(prop("--energy"), 0), 0.1).out()
        "#,
    )
    .expect("parse");
    let out = compile(&comp).expect("compile");

    let kinds: Vec<_> = out
        .uniforms
        .iter()
        .map(|u| format!("{:?}", u.kind))
        .collect();
    assert!(
        kinds.iter().any(|k| k.starts_with("AudioRms")),
        "got {:?}",
        kinds
    );
    assert!(
        out.uniforms
            .iter()
            .any(|u| matches!(&u.kind, UniformKind::CssProp(n) if n == "--energy")),
        "got {:?}",
        kinds
    );
}

#[test]
fn parser_supports_negative_numbers_in_args() {
    // Hydra often passes negative scroll speeds and modulate offsets — make
    // sure the tokenizer handles a leading '-' on a numeric literal.
    let comp = parse("osc(3).scroll(0, 0, -0.1, 0).out()").expect("parse");
    let out = compile(&comp).expect("compile");
    assert!(out.passes[0].wgsl.contains("-0.1"));
}

#[test]
fn bare_progress_identifier_binds_implicit_css_prop_uniform() {
    // Agents writing transition shaders (crossfade, wipe, dip-to-black)
    // reach for `progress` as a bare identifier — CSS / GLSL muscle
    // memory. Before the fix this errored with `unknown identifier
    // 'progress'`; the parser now treats it as shorthand for
    // `prop("progress")`, the same uniform the wavelet transition
    // pipeline already fills per-frame.
    let comp = parse("src(0).blend(src(1), progress).out()").expect("parse");
    let out = compile(&comp).expect("compile");
    assert!(
        out.uniforms
            .iter()
            .any(|u| matches!(&u.kind, UniformKind::CssProp(n) if n == "progress")),
        "expected a CssProp(\"progress\") uniform, got: {:?}",
        out.uniforms.iter().map(|u| &u.kind).collect::<Vec<_>>()
    );

    // Sanity: implicit-`progress` shorthand must lower to the same WGSL
    // as the explicit `prop("progress")` form, so the renderer can't tell
    // them apart.
    let explicit = parse("src(0).blend(src(1), prop(\"progress\")).out()").expect("parse");
    let explicit_out = compile(&explicit).expect("compile");
    assert_eq!(out.passes[0].wgsl, explicit_out.passes[0].wgsl);
}

#[test]
fn unknown_bare_identifier_still_errors() {
    // The implicit-uniform fallback is a tight allowlist — unrecognized
    // bare identifiers must still surface a clear parse error rather
    // than silently binding zero-valued uniforms.
    let err = parse("src(0).blend(src(1), wobble).out()").expect_err("must error");
    match err {
        Diagnostic::ParseError { message, .. } => {
            assert!(message.contains("wobble"), "got: {}", message);
        }
        other => panic!("expected ParseError, got {:?}", other),
    }
}
