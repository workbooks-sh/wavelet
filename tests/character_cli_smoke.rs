//! End-to-end CLI smoke for `wavelet character define` (wb-cx08).
//!
//! Invokes the built binary against a temp workdir and asserts the
//! emitted clip-HTML lands at the expected path with the expected
//! author-visible fields (`name`, `reference-images`, `character-type`).

use std::path::PathBuf;
use std::process::Command;

fn wavelet_bin() -> PathBuf {
    // Cargo sets CARGO_BIN_EXE_<name> for integration tests so we don't
    // have to guess the target/ layout.
    PathBuf::from(env!("CARGO_BIN_EXE_wavelet"))
}

#[test]
fn character_define_writes_clip_html_with_expected_fields() {
    let tmp = tempfile::tempdir().unwrap();
    let workdir = tmp.path();
    // Use a fixture path that doesn't have to exist — `character define`
    // doesn't open the reference files, just records the strings.
    let fixture = workdir.join("alex-face-1.jpg");
    std::fs::write(&fixture, b"fake-jpeg").unwrap();

    let output = Command::new(wavelet_bin())
        .arg("character")
        .arg("define")
        .arg("ALEX")
        .arg("--reference")
        .arg(&fixture)
        .arg("--workdir")
        .arg(workdir)
        .output()
        .expect("spawn wavelet");
    assert!(
        output.status.success(),
        "wavelet character define failed: {}\nstderr: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr),
    );

    let expected_path = workdir.join("refs/character/alex.clip.html");
    assert!(
        expected_path.exists(),
        "expected emission at {}\nstderr: {}",
        expected_path.display(),
        String::from_utf8_lossy(&output.stderr),
    );

    let raw = std::fs::read_to_string(&expected_path).unwrap();
    // Front-matter sanity: kind on wire is `character-ref`, name field
    // is the canonical uppercase form, references survive verbatim,
    // character-type defaults to `full-body`.
    assert!(raw.contains("kind: character-ref"), "front matter:\n{raw}");
    assert!(raw.contains("name: ALEX"), "name field:\n{raw}");
    assert!(
        raw.contains("character-type: full-body"),
        "character-type:\n{raw}",
    );
    assert!(raw.contains("alex-face-1.jpg"), "reference path:\n{raw}");

    // The loader should now pick it up via load_characters.
    let loaded = wavelet::clipref::character::load_characters(workdir).unwrap();
    assert_eq!(loaded.len(), 1);
    let alex = loaded.lookup_full("ALEX").expect("ALEX full-body is keyed");
    assert_eq!(alex.name, "ALEX");
    assert_eq!(alex.reference_images.len(), 1);
    assert_eq!(
        alex.character_type,
        wavelet::clipref::character::CharacterType::FullBody,
    );
}

#[test]
fn character_define_canonicalizes_extension_form() {
    // `Alex (V.O.)` → file lands at `alex.clip.html`, key is `ALEX`.
    let tmp = tempfile::tempdir().unwrap();
    let workdir = tmp.path();
    let fixture = workdir.join("a.jpg");
    std::fs::write(&fixture, b"x").unwrap();

    let output = Command::new(wavelet_bin())
        .arg("character")
        .arg("define")
        .arg("Alex (V.O.)")
        .arg("--reference")
        .arg(&fixture)
        .arg("--workdir")
        .arg(workdir)
        .output()
        .expect("spawn wavelet");
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr),
    );

    assert!(workdir.join("refs/character/alex.clip.html").exists());
    let loaded = wavelet::clipref::character::load_characters(workdir).unwrap();
    assert!(loaded.contains_key("ALEX"));
}

#[test]
fn character_define_rejects_missing_references() {
    let tmp = tempfile::tempdir().unwrap();
    let workdir = tmp.path();
    let output = Command::new(wavelet_bin())
        .arg("character")
        .arg("define")
        .arg("ALEX")
        .arg("--workdir")
        .arg(workdir)
        .output()
        .expect("spawn wavelet");
    // Either clap rejects (no --reference provided) or our handler does
    // (empty vec). Both are non-zero exits.
    assert!(!output.status.success(), "should reject zero references");
}
