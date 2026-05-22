//! Tier-1 validator hermetic-fixture round-trip with REAL binaries (wb-mxrk.15).
//!
//! The unit tests in `src/agent/plan/validators/tests.rs` (wb-mqsb.3) point
//! `ValidatorCtx::gamut_bin` at a `fake_gamut` shell script that emits
//! canned JSON. That covers validator logic, but it can't catch
//! argv-shape regressions — flag renames, subcommand reshuffles, exit-code
//! changes in the real `wavelet` binary. These integration tests spawn the
//! actual built binary against a tiny hermetic fixture composition to
//! shake those out.
//!
//! Each test is `#[ignore]` because spawning the real binary is slow
//! (~hundreds of ms). Default `cargo test` skips them; CI / manual runs
//! opt in:
//!
//!   cargo test -p wavelet --test validators_tier1_smoke -- --ignored
//!
//! `rubric_passes` is intentionally NOT covered here — it needs a Gemini
//! API key. The other Tier-1 kinds are all keyless.

#![cfg(unix)]

use std::fs;
use std::path::{Path, PathBuf};

use wavelet::agent::plan::{
    ArtifactExists, CompVerifyPasses, UnitTestPasses, Validator, ValidatorCtx,
    ValidatorRegistry,
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

/// Path to the built `wavelet` binary. Cargo wires `CARGO_BIN_EXE_<name>`
/// at integration-test compile time and auto-builds the bin first.
fn gamut_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_wavelet"))
}

/// Source fixture directory — copied into a tempdir per-test so workdir
/// resolution mirrors how validators are invoked at runtime.
fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

fn stage_fixture(name: &str, into: &Path) -> PathBuf {
    let src = fixtures_dir().join(name);
    let dst = into.join(name);
    fs::copy(&src, &dst).expect("copy fixture");
    dst
}

#[test]
#[ignore]
fn comp_verify_passes_real_binary_ok() {
    let dir = tempfile::tempdir().unwrap();
    let workdir = dir.path();
    stage_fixture("tier1_comp.json", workdir);
    stage_fixture("tier1_scene.html", workdir);

    let bin = gamut_bin();
    let c = ctx(workdir, &bin);

    let registry = ValidatorRegistry::with_builtins();
    let v = registry.get("comp_verify_passes").unwrap();
    let out = v.check(&yaml("{ comp: tier1_comp.json }"), &c);

    assert!(
        out.ok,
        "expected verify to pass on the minimal fixture, detail={}",
        out.detail
    );
    assert_eq!(out.detail["exit_code"], serde_json::Value::from(0));
}

#[test]
#[ignore]
fn comp_verify_passes_real_binary_fail_on_broken_comp() {
    let dir = tempfile::tempdir().unwrap();
    let workdir = dir.path();
    stage_fixture("tier1_broken.json", workdir);

    let bin = gamut_bin();
    let c = ctx(workdir, &bin);

    let v = CompVerifyPasses;
    let out = v.check(&yaml("{ comp: tier1_broken.json }"), &c);

    assert!(
        !out.ok,
        "expected verify to fail on a malformed composition, detail={}",
        out.detail
    );
    assert!(
        out.detail.is_object(),
        "failure detail must be structured JSON, got {:?}",
        out.detail
    );
    assert_eq!(
        out.detail["error"],
        serde_json::Value::String("verify_failed".into()),
        "detail={}",
        out.detail
    );
    assert!(out.detail.get("exit_code").is_some());
    assert!(out.detail.get("argv").is_some());
}

/// Crate manifest dir — `cargo test` needs a Cargo.toml in workdir or an
/// ancestor to discover the workspace. The `unit_test_passes` validator
/// inherits the agent's workdir, which in real use lives inside the
/// repo; mirror that by pointing at the crate root.
fn crate_workdir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

#[test]
#[ignore]
fn unit_test_passes_real_cargo_ok() {
    let workdir = crate_workdir();
    let bin = gamut_bin();
    let c = ctx(&workdir, &bin);

    let v = UnitTestPasses;
    // round_trip_single_task lives in src/agent/plan/schema.rs tests.
    // It's fast and stable — exactly what we want for argv-shape smoke.
    let params = yaml(
        "{ pkg: wavelet, test: agent::plan::schema::tests::round_trip_single_task }",
    );
    let out = v.check(&params, &c);

    assert!(
        out.ok,
        "expected cargo test on a known-passing test to pass, detail={}",
        out.detail
    );
    assert_eq!(out.detail["exit_code"], serde_json::Value::from(0));
}

/// argv-shape finding (wb-mxrk.15): `cargo test -p <pkg> <filter>`
/// exits 0 when the filter matches zero tests — cargo considers that
/// "nothing failed." `unit_test_passes` grades only on exit code, so a
/// typo'd test name silently passes. The failure-mode this test pins
/// down is "the package doesn't exist," which cargo does treat as
/// nonzero. Worth a follow-on: have `unit_test_passes` parse the cargo
/// summary line and fail when `0 passed` AND a filter was supplied.
#[test]
#[ignore]
fn unit_test_passes_real_cargo_fail_on_nonexistent_package() {
    let workdir = crate_workdir();
    let bin = gamut_bin();
    let c = ctx(&workdir, &bin);

    let v = UnitTestPasses;
    let params = yaml(
        "{ pkg: zzzz_not_a_real_crate_zzzz_wb_mxrk_15, test: anything }",
    );
    let out = v.check(&params, &c);

    assert!(
        !out.ok,
        "expected cargo test on a nonexistent package to fail, detail={}",
        out.detail
    );
    assert!(out.detail.is_object());
    assert_eq!(
        out.detail["error"],
        serde_json::Value::String("tests_failed".into()),
        "detail={}",
        out.detail
    );
}

#[test]
#[ignore]
fn artifact_exists_real_tempfile_roundtrip() {
    let dir = tempfile::tempdir().unwrap();
    let workdir = dir.path();
    fs::write(workdir.join("output.bin"), b"sentinel\n").unwrap();
    let bin = gamut_bin();
    let c = ctx(workdir, &bin);

    let v = ArtifactExists;
    let ok = v.check(&yaml("{ path: output.bin }"), &c);
    assert!(ok.ok, "detail={}", ok.detail);
    assert_eq!(ok.detail["exists"], serde_json::Value::Bool(true));

    let missing = v.check(&yaml("{ path: nope.bin }"), &c);
    assert!(!missing.ok);
    assert_eq!(missing.detail["exists"], serde_json::Value::Bool(false));
}

// c2pa_verify_passes intentionally not covered with a real-binary smoke:
// no C2PA-signed fixture asset exists in this tree (audited under
// packages/wavelet/tests/ and examples/). When a signed test asset lands
// (wb-mxrk follow-on), add a smoke that points `path` at it and asserts
// `outcome.ok == true`. The mock-tested unit coverage in
// agent::plan::validators::tests covers the JSON-grading logic.
