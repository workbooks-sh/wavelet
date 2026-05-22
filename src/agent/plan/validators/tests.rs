//! Unit tests for the eight heavy validators (wb-mqsb.3).
//!
//! Subprocess validators are mocked by pointing `ctx.gamut_bin` at a
//! tiny shell script that emits fixture JSON on stdout. This keeps the
//! tests hermetic — no real comp.json, no real Blitz pipeline.
//!
//! Tests that need the full pipeline (real `wavelet query` against a real
//! composition) are `#[ignore]`d so `cargo test` skips them by default.
//! Run with `cargo test -p wavelet agent::plan::validators -- --ignored`.

use std::fs;
use std::path::{Path, PathBuf};

use super::super::validator::{Validator, ValidatorCtx};
use super::{
    C2paVerifyPasses, CompVerifyPasses, QueryBeat, QueryPixels, QuerySceneGraph, QuerySnapshot,
    RubricPasses, UnitTestPasses,
};

fn yaml(s: &str) -> serde_yaml::Value {
    serde_yaml::from_str(s).unwrap()
}

fn ctx<'a>(workdir: &'a Path, gamut_bin: &'a Path) -> ValidatorCtx<'a> {
    ValidatorCtx {
        workdir,
        gamut_bin,
        session_cost_usd: 0.0,
    }
}

/// Write a fake `wavelet` binary that prints `payload` to stdout and exits
/// with `exit_code`. Returns its path.
#[cfg(unix)]
fn fake_gamut(dir: &Path, name: &str, payload: &str, exit_code: i32) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let path = dir.join(name);
    // single-quote the payload, escape any embedded single quotes.
    let escaped = payload.replace('\'', "'\\''");
    let script = format!("#!/bin/sh\nprintf '%s' '{escaped}'\nexit {exit_code}\n");
    fs::write(&path, script).unwrap();
    let mut perms = fs::metadata(&path).unwrap().permissions();
    perms.set_mode(0o755);
    fs::set_permissions(&path, perms).unwrap();
    path
}

#[cfg(unix)]
fn fake_gamut_stderr(dir: &Path, name: &str, stderr_text: &str, exit_code: i32) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let path = dir.join(name);
    let escaped = stderr_text.replace('\'', "'\\''");
    let script = format!("#!/bin/sh\nprintf '%s' '{escaped}' 1>&2\nexit {exit_code}\n");
    fs::write(&path, script).unwrap();
    let mut perms = fs::metadata(&path).unwrap().permissions();
    perms.set_mode(0o755);
    fs::set_permissions(&path, perms).unwrap();
    path
}

// ─── query_scene_graph ─────────────────────────────────────────────

#[cfg(unix)]
#[test]
fn query_scene_graph_visible_pass() {
    let dir = tempfile::tempdir().unwrap();
    let payload = r#"{"visible":{".cta":{"visible":true,"reason":"shown"}}}"#;
    let bin = fake_gamut(dir.path(), "wavelet-vis-ok", payload, 0);
    let c = ctx(dir.path(), &bin);
    let v = QuerySceneGraph;
    let out = v.check(&yaml("{ comp: c.json, expect: { visible: ['.cta'] } }"), &c);
    assert!(out.ok, "detail={:?}", out.detail);
}

#[cfg(unix)]
#[test]
fn query_scene_graph_visible_fail_includes_not_found_marker() {
    let dir = tempfile::tempdir().unwrap();
    let payload = r#"{"visible":{}}"#;
    let bin = fake_gamut(dir.path(), "wavelet-vis-fail", payload, 0);
    let c = ctx(dir.path(), &bin);
    let v = QuerySceneGraph;
    let out = v.check(&yaml("{ comp: c.json, expect: { visible: ['.cta'] } }"), &c);
    assert!(!out.ok);
    assert_eq!(out.detail["failed_clause"], "visible");
    assert_eq!(out.detail["selector"], ".cta");
    assert_eq!(out.detail["actual"]["not_found"], true);
}

#[cfg(unix)]
#[test]
fn query_scene_graph_bbox_at_least_fail_shape() {
    let dir = tempfile::tempdir().unwrap();
    let payload = r#"{"visible":{".hero":{"visible":true}},"bbox":{".hero":{"x":0,"y":0,"w":50,"h":40}}}"#;
    let bin = fake_gamut(dir.path(), "wavelet-bbox-fail", payload, 0);
    let c = ctx(dir.path(), &bin);
    let v = QuerySceneGraph;
    let out = v.check(
        &yaml("{ comp: c.json, expect: { visible: ['.hero'], bbox_at_least: { selector: '.hero', w: 200, h: 100 } } }"),
        &c,
    );
    assert!(!out.ok);
    assert_eq!(out.detail["failed_clause"], "bbox_at_least");
    assert_eq!(out.detail["expected"]["w"].as_f64().unwrap(), 200.0);
}

#[cfg(unix)]
#[test]
fn query_scene_graph_spawn_failure() {
    let dir = tempfile::tempdir().unwrap();
    let bin = dir.path().join("does-not-exist");
    let c = ctx(dir.path(), &bin);
    let v = QuerySceneGraph;
    let out = v.check(&yaml("{ comp: c.json, expect: { visible: ['.cta'] } }"), &c);
    assert!(!out.ok);
    assert_eq!(out.detail["error"], "spawn_failed");
}

// ─── query_pixels ──────────────────────────────────────────────────

#[cfg(unix)]
#[test]
fn query_pixels_color_at_pass_and_fail_diff_shape() {
    let dir = tempfile::tempdir().unwrap();
    let ok_payload = r##"{"color_in":{".hl":{"mean_hex":"#fefefe","delta_e":1.2,"within":true}}}"##;
    let ok_bin = fake_gamut(dir.path(), "wavelet-px-ok", ok_payload, 0);
    let v = QueryPixels;
    let out = v.check(
        &yaml("{ comp: c.json, expect: { color_at: { selector: '.hl', hex: '#ffffff', tolerance: 5 } } }"),
        &ctx(dir.path(), &ok_bin),
    );
    assert!(out.ok, "detail={:?}", out.detail);

    let bad_payload = r##"{"color_in":{".hl":{"mean_hex":"#882222","delta_e":42.7,"within":false}}}"##;
    let bad_bin = fake_gamut(dir.path(), "wavelet-px-bad", bad_payload, 0);
    let bad = v.check(
        &yaml("{ comp: c.json, expect: { color_at: { selector: '.hl', hex: '#ffffff', tolerance: 5 } } }"),
        &ctx(dir.path(), &bad_bin),
    );
    assert!(!bad.ok);
    assert_eq!(bad.detail["failed_clause"], "color_at");
    assert_eq!(bad.detail["expected_hex"], "#ffffff");
    assert_eq!(bad.detail["actual_hex"], "#882222");
    assert!(bad.detail["delta_e"].as_f64().unwrap() > 5.0);
}

#[cfg(unix)]
#[test]
fn query_pixels_contrast_fail() {
    let dir = tempfile::tempdir().unwrap();
    let payload = r#"{"contrast":{".hl":{"ratio":2.1}}}"#;
    let bin = fake_gamut(dir.path(), "wavelet-contrast-fail", payload, 0);
    let v = QueryPixels;
    let out = v.check(
        &yaml("{ comp: c.json, expect: { contrast: { selector: '.hl', min_ratio: 4.5 } } }"),
        &ctx(dir.path(), &bin),
    );
    assert!(!out.ok);
    assert_eq!(out.detail["failed_clause"], "contrast");
    assert_eq!(out.detail["actual_ratio"], 2.1);
}

// ─── query_snapshot ────────────────────────────────────────────────

#[cfg(unix)]
#[test]
fn query_snapshot_writes_file_and_passes() {
    let dir = tempfile::tempdir().unwrap();
    let payload = r#"{"frame":{"w":1920,"h":1080},"nodes":[]}"#;
    let bin = fake_gamut(dir.path(), "wavelet-snap-ok", payload, 0);
    let v = QuerySnapshot;
    let out = v.check(
        &yaml("{ comp: c.json, snapshot: out/snap.json, expect_exists: true }"),
        &ctx(dir.path(), &bin),
    );
    assert!(out.ok, "detail={:?}", out.detail);
    assert_eq!(out.detail["exists"], true);
    let written = fs::read(dir.path().join("out/snap.json")).unwrap();
    assert!(!written.is_empty());
}

#[cfg(unix)]
#[test]
fn query_snapshot_subprocess_failure() {
    let dir = tempfile::tempdir().unwrap();
    let bin = fake_gamut_stderr(dir.path(), "wavelet-snap-fail", "oh no", 2);
    let v = QuerySnapshot;
    let out = v.check(
        &yaml("{ comp: c.json, snapshot: snap.json, expect_exists: true }"),
        &ctx(dir.path(), &bin),
    );
    assert!(!out.ok);
    assert_eq!(out.detail["error"], "subprocess_nonzero");
    assert_eq!(out.detail["exit_code"], 2);
}

// ─── query_beat ────────────────────────────────────────────────────

#[cfg(unix)]
#[test]
fn query_beat_all_within_pass() {
    let dir = tempfile::tempdir().unwrap();
    let payload =
        r#"{"on_beat":[{"event":"scene.0.start","within_tolerance":true,"delta_ms":3},
                       {"event":"scene.1.start","within_tolerance":true,"delta_ms":-7}]}"#;
    let bin = fake_gamut(dir.path(), "wavelet-beat-ok", payload, 0);
    let v = QueryBeat;
    let out = v.check(
        &yaml(
            "{ comp: c.json, on_beat: bg.mp3, tolerance_ms: 33, expect: { within_tolerance: true } }",
        ),
        &ctx(dir.path(), &bin),
    );
    assert!(out.ok, "detail={:?}", out.detail);
    assert_eq!(out.detail["events_total"], 2);
    assert_eq!(out.detail["events_off_beat"], 0);
}

#[cfg(unix)]
#[test]
fn query_beat_off_includes_offenders() {
    let dir = tempfile::tempdir().unwrap();
    let payload = r#"{"on_beat":[{"event":"scene.0.start","within_tolerance":false,"delta_ms":120}]}"#;
    let bin = fake_gamut(dir.path(), "wavelet-beat-bad", payload, 0);
    let v = QueryBeat;
    let out = v.check(
        &yaml(
            "{ comp: c.json, on_beat: bg.mp3, tolerance_ms: 33, expect: { within_tolerance: true } }",
        ),
        &ctx(dir.path(), &bin),
    );
    assert!(!out.ok);
    assert_eq!(out.detail["failed_clause"], "within_tolerance");
    assert_eq!(out.detail["events_off_beat"], 1);
    assert!(out.detail["off_beat_sample"].as_array().unwrap().len() >= 1);
}

// ─── comp_verify_passes ────────────────────────────────────────────

#[cfg(unix)]
#[test]
fn comp_verify_passes_zero_exit() {
    let dir = tempfile::tempdir().unwrap();
    let bin = fake_gamut(dir.path(), "wavelet-verify-ok", "✓ clean\n", 0);
    let v = CompVerifyPasses;
    let out = v.check(&yaml("{ comp: c.json }"), &ctx(dir.path(), &bin));
    assert!(out.ok, "detail={:?}", out.detail);
    assert_eq!(out.detail["exit_code"], 0);
}

#[cfg(unix)]
#[test]
fn comp_verify_passes_nonzero_exit_shape() {
    let dir = tempfile::tempdir().unwrap();
    let bin = fake_gamut_stderr(dir.path(), "wavelet-verify-bad", "ERROR  [scene 0] missing audio cue", 1);
    let v = CompVerifyPasses;
    let out = v.check(&yaml("{ comp: c.json }"), &ctx(dir.path(), &bin));
    assert!(!out.ok);
    assert_eq!(out.detail["error"], "verify_failed");
    assert_eq!(out.detail["exit_code"], 1);
    assert!(out.detail["stderr_tail"].as_str().unwrap().contains("missing audio"));
}

// ─── c2pa_verify_passes ────────────────────────────────────────────

#[cfg(unix)]
#[test]
fn c2pa_verify_passes_clean_manifest() {
    let dir = tempfile::tempdir().unwrap();
    let payload = r#"{"validation_status":[],"ingredients":[]}"#;
    let bin = fake_gamut(dir.path(), "wavelet-c2pa-ok", payload, 0);
    let v = C2paVerifyPasses;
    let out = v.check(&yaml("{ path: out.mp4 }"), &ctx(dir.path(), &bin));
    assert!(out.ok, "detail={:?}", out.detail);
}

#[cfg(unix)]
#[test]
fn c2pa_verify_passes_error_status() {
    let dir = tempfile::tempdir().unwrap();
    let payload = r#"{"validation_status":[{"code":"signingCredential.untrusted","explanation":"dev cert"}]}"#;
    let bin = fake_gamut(dir.path(), "wavelet-c2pa-bad", payload, 0);
    let v = C2paVerifyPasses;
    let out = v.check(&yaml("{ path: out.mp4 }"), &ctx(dir.path(), &bin));
    assert!(!out.ok);
    assert_eq!(out.detail["failed_clause"], "validation_status");
    assert!(out.detail["errors"].as_array().unwrap().len() >= 1);
}

// ─── unit_test_passes ──────────────────────────────────────────────

#[cfg(unix)]
#[test]
fn unit_test_passes_routes_to_cargo_with_args() {
    // Mock `cargo` itself via CARGO=… env var so we don't shell out to
    // the real toolchain. The fake `cargo` exits 0 unconditionally.
    let dir = tempfile::tempdir().unwrap();
    let cargo = fake_gamut(dir.path(), "fake-cargo", "test result: ok. 0 passed", 0);
    std::env::set_var("CARGO", &cargo);
    let bin = PathBuf::from("wavelet");
    let v = UnitTestPasses;
    let out = v.check(
        &yaml("{ pkg: wavelet, test: agent::plan::validators }"),
        &ctx(dir.path(), &bin),
    );
    std::env::remove_var("CARGO");
    assert!(out.ok, "detail={:?}", out.detail);
    let argv = out.detail["argv"].as_array().unwrap();
    assert_eq!(argv[0], "test");
    assert!(argv.iter().any(|a| a == "--no-fail-fast"));
}

#[cfg(unix)]
#[test]
fn unit_test_passes_nonzero_exit_shape() {
    let dir = tempfile::tempdir().unwrap();
    let cargo = fake_gamut_stderr(dir.path(), "fake-cargo-fail", "test failed", 101);
    std::env::set_var("CARGO", &cargo);
    let bin = PathBuf::from("wavelet");
    let v = UnitTestPasses;
    let out = v.check(&yaml("{ pkg: wavelet }"), &ctx(dir.path(), &bin));
    std::env::remove_var("CARGO");
    assert!(!out.ok);
    assert_eq!(out.detail["error"], "tests_failed");
    assert_eq!(out.detail["exit_code"], 101);
}

// ─── rubric_passes ─────────────────────────────────────────────────

#[test]
fn rubric_passes_returns_structured_no_api_key_failure() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("hero.png"), b"\x89PNG\r\n\x1a\nfake").unwrap();
    let bin = PathBuf::from("wavelet");
    // Clear both env vars so we hit the no_api_key branch deterministically.
    let prev_gem = std::env::var("GEMINI_API_KEY").ok();
    let prev_goog = std::env::var("GOOGLE_API_KEY").ok();
    std::env::remove_var("GEMINI_API_KEY");
    std::env::remove_var("GOOGLE_API_KEY");
    let v = RubricPasses;
    let out = v.check(
        &yaml(
            "{ artifact: hero.png, rubric: { prompt: 'is this a hero shot?', must_satisfy: ['subject is centered', 'background is dark'] } }",
        ),
        &ctx(dir.path(), &bin),
    );
    if let Some(k) = prev_gem { std::env::set_var("GEMINI_API_KEY", k); }
    if let Some(k) = prev_goog { std::env::set_var("GOOGLE_API_KEY", k); }
    assert!(!out.ok);
    assert_eq!(out.detail["error"], "no_api_key");
    assert_eq!(out.detail["artifact"], "hero.png");
    assert_eq!(out.cost_usd, 0.0);
}

#[test]
fn rubric_passes_missing_artifact_param_is_structured() {
    let dir = tempfile::tempdir().unwrap();
    let bin = PathBuf::from("wavelet");
    let v = RubricPasses;
    let out = v.check(&yaml("{ rubric: { prompt: 'x', must_satisfy: ['y'] } }"), &ctx(dir.path(), &bin));
    assert!(!out.ok);
    assert_eq!(out.detail["error"], "missing_param");
    assert_eq!(out.detail["param"], "artifact");
}

#[test]
fn rubric_passes_missing_artifact_file_is_structured() {
    let dir = tempfile::tempdir().unwrap();
    let bin = PathBuf::from("wavelet");
    std::env::set_var("GEMINI_API_KEY", "dummy");
    let v = RubricPasses;
    let out = v.check(
        &yaml(
            "{ artifact: nope.png, rubric: { prompt: 'x', must_satisfy: ['y'] } }",
        ),
        &ctx(dir.path(), &bin),
    );
    std::env::remove_var("GEMINI_API_KEY");
    assert!(!out.ok);
    assert_eq!(out.detail["error"], "artifact_read_failed");
}

/// Real-API smoke test. Requires GEMINI_API_KEY in the environment +
/// a real PNG at tests/fixtures/rubric_hero.png. Gated behind --ignored.
#[test]
#[ignore]
fn rubric_passes_real_gemini_smoke() {
    let dir = tempfile::tempdir().unwrap();
    // Minimal 1x1 PNG so the call doesn't bounce on payload size.
    let png: [u8; 67] = [
        0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44, 0x52,
        0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1f, 0x15, 0xc4,
        0x89, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9c, 0x63, 0x00, 0x01, 0x00, 0x00,
        0x05, 0x00, 0x01, 0x0d, 0x0a, 0x2d, 0xb4, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4e, 0x44, 0xae,
        0x42, 0x60, 0x82,
    ];
    fs::write(dir.path().join("pixel.png"), png).unwrap();
    let bin = PathBuf::from("wavelet");
    let v = RubricPasses;
    let out = v.check(
        &yaml(
            "{ artifact: pixel.png, rubric: { prompt: 'Describe the dominant color of the image.', must_satisfy: ['image is at most 1x1 pixel'] } }",
        ),
        &ctx(dir.path(), &bin),
    );
    // Don't assert ok/!ok — only that we got a structured response with
    // either clauses or a documented error reason. Cost should be set.
    assert!(out.cost_usd >= 0.0);
    println!("rubric smoke detail: {}", out.detail);
}

/// Real-pipeline smoke test for query_scene_graph against a fixture
/// composition. Requires the wavelet binary built + a fixture comp.json.
/// Gated.
#[test]
#[ignore]
fn query_scene_graph_real_pipeline_smoke() {
    let workdir = PathBuf::from("/tmp/wb-mqsb-3-smoke");
    std::fs::create_dir_all(&workdir).unwrap();
    // Use PATH-resolved wavelet. Build it first with `cargo build -p wavelet`.
    let bin = PathBuf::from("wavelet");
    let v = QuerySceneGraph;
    let out = v.check(
        &yaml("{ comp: smoke.json, expect: { visible: ['#root'] } }"),
        &ctx(&workdir, &bin),
    );
    println!("scene-graph smoke detail: {}", out.detail);
    // Just assert structured output exists.
    assert!(out.detail.is_object());
}
