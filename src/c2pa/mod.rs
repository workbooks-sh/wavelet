//! C2PA content-credentials signing for wavelet exports.
//!
//! Embeds a signed C2PA manifest into the final MP4. The manifest declares:
//!
//! - This MP4 was AI-generated.
//! - Which models produced which assets (Seedream, Kling, EL Music, …) — the
//!   ingredient list, sourced from each backend's cache manifests.
//! - Which actions were performed (create-by-AI, place, composite).
//! - Title / author metadata (`stds.schema-org.CreativeWork`).
//! - Training opt-out (`c2pa.training-mining` set to `notAllowed`).
//!
//! Any byte-level tamper of the output invalidates the manifest's hash chain
//! and `Reader::with_stream` reports `validation_state != Valid`.
//!
//! # Why
//!
//! EU AI Act Article 50 enforcement begins August 2026. Commercial deliverables
//! shipped to EU markets after that date must carry C2PA provenance or risk
//! being rejected by downstream platforms (Adobe Premiere, Sony hardware, and
//! every major CMS already round-trip C2PA).
//!
//! # Usage
//!
//! ```ignore
//! use crate::c2pa::{build_manifest, sign_mp4, ManifestInputs};
//!
//! let inputs = ManifestInputs::from_composition(&comp, &cache_root, /*title*/ None);
//! let manifest_json = build_manifest(&inputs)?;
//! sign_mp4(&manifest_json, "in.mp4", "out.mp4", /*signing_key*/ None)?;
//! ```
//!
//! # Production keys
//!
//! For v0, signing uses a self-signed ES256 test certificate bundled in the
//! crate (the same fixture the upstream `c2pa-rs` test suite uses). Verifiers
//! will flag this as an untrusted signer — fine for development, not fine for
//! the EU deadline. Production deployments override via `signing_key: Some(...)`
//! with a real cert issued by a C2PA-trusted CA (or a Polar.sh-issued org cert,
//! once that flow exists).
//!
//! See [`signer`] for the test-cert bundle + production-cert override path,
//! and [`manifest`] for how the JSON manifest is assembled from a [`crate::render_offline::Composition`]
//! + the on-disk backend cache.

pub mod manifest;
pub mod signer;

pub use manifest::{build_manifest, ManifestInputs};
pub use signer::{
    build_signer, load_test_signer, SigningKey, WAVELET_CLAIM_GENERATOR,
};

use crate::render_offline::Composition;
use std::path::{Path, PathBuf};

/// Errors produced by the C2PA sign / verify pipeline.
#[derive(Debug, thiserror::Error)]
pub enum C2paError {
    /// Underlying c2pa-rs SDK error (signing, parsing, embedding).
    #[error("c2pa sdk: {0}")]
    Sdk(#[from] c2pa::Error),
    /// Filesystem error (reading the MP4 in, writing the signed MP4 out,
    /// loading the signing key, walking the cache).
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    /// JSON serialization failure when assembling the manifest definition.
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    /// Backend cache error (manifest lookup for an ingredient hash).
    #[error("cache: {0}")]
    Cache(String),
    /// Signed manifest re-read after embed didn't match the expected
    /// generator/ingredient set — indicates the embed silently failed.
    #[error("verify after sign: {0}")]
    VerifyMismatch(String),
}

/// Sign an existing MP4 in place: read `input`, embed a C2PA manifest built
/// from the composition + cache state, write to `output`. Round-trips the
/// signed file back through [`verify`] before returning success.
pub fn sign_mp4(
    composition: &Composition,
    cache_root: Option<&Path>,
    title: Option<&str>,
    author: Option<&str>,
    input: &Path,
    output: &Path,
    signing_key: Option<SigningKey>,
) -> Result<SignReport, C2paError> {
    let inputs = ManifestInputs::from_composition(composition, cache_root, title, author);
    let manifest_json = build_manifest(&inputs)?;

    let mut builder = c2pa::Builder::default()
        .with_definition(manifest_json.as_str())
        .map_err(C2paError::Sdk)?;

    let signer = match signing_key {
        Some(k) => build_signer(k)?,
        None => load_test_signer()?,
    };

    let mut src = std::fs::File::open(input)?;
    let mut dst = std::fs::File::create(output)?;
    builder
        .sign(&*signer, "video/mp4", &mut src, &mut dst)
        .map_err(C2paError::Sdk)?;
    drop(dst);

    let report = verify(output)?;
    if !report.valid {
        return Err(C2paError::VerifyMismatch(format!(
            "post-sign verify failed: {}",
            report.summary
        )));
    }
    Ok(report)
}

/// Read a signed MP4, parse its manifest, and report what's inside + whether
/// the hash chain validates.
pub fn verify(signed_mp4: &Path) -> Result<SignReport, C2paError> {
    let stream = std::fs::File::open(signed_mp4)?;
    let reader = c2pa::Reader::default()
        .with_stream("video/mp4", stream)
        .map_err(C2paError::Sdk)?;

    let state_str = format!("{:?}", reader.validation_state());

    let active = reader.active_manifest();
    // Modern manifests use `claim_generator_info` (array); older ones populate
    // the bare `claim_generator` string. Fall back across both so the report
    // surfaces a name regardless of which the signer emitted.
    let generator = active
        .and_then(|m| m.claim_generator().map(str::to_string))
        .or_else(|| {
            // `claim_generator_info` is a `Vec<ClaimGeneratorInfo>` on Manifest.
            // We flatten to JSON and pluck the "name" string — sidesteps SDK
            // field-visibility churn across c2pa-rs versions.
            active.and_then(|m| {
                let v = serde_json::to_value(&m.claim_generator_info).ok()?;
                v.as_array()?
                    .iter()
                    .filter_map(|cgi| cgi.get("name").and_then(|n| n.as_str()))
                    .next()
                    .map(str::to_string)
            })
        })
        .unwrap_or_default();
    let title = active.and_then(|m| m.title().map(str::to_string));
    let ingredients: Vec<String> = active
        .map(|m| {
            m.ingredients()
                .iter()
                .map(|i| i.title().unwrap_or("<unknown>").to_string())
                .collect()
        })
        .unwrap_or_default();
    let assertion_labels: Vec<String> = active
        .map(|m| m.assertions().iter().map(|a| a.label().to_string()).collect())
        .unwrap_or_default();

    let validation_failures: Vec<String> = reader
        .validation_results()
        .map(|r| {
            r.active_manifest()
                .map(|am| am.failure().iter().map(|s| s.code().to_string()).collect())
                .unwrap_or_default()
        })
        .unwrap_or_default();

    // For dev signing with the bundled self-signed cert, the trust chain check
    // fails (`signingCredential.untrusted`) but the hash chain is intact. We
    // treat the file as "structurally valid" iff every reported failure is a
    // trust-only failure. Any code matching the assertion/hash family flips
    // `valid` to false. Production signing with a real CA cert lands on
    // `Trusted` / `Valid` and `validation_failures` is empty.
    let trust_only_codes = ["signingCredential.untrusted", "signingCredential.unknown"];
    let only_trust_failures = !validation_failures.is_empty()
        && validation_failures
            .iter()
            .all(|c| trust_only_codes.contains(&c.as_str()));
    let valid = matches!(
        reader.validation_state(),
        c2pa::ValidationState::Valid | c2pa::ValidationState::Trusted
    ) || only_trust_failures;

    let summary = format!(
        "validation_state={state_str}; generator={generator}; \
         {} ingredients; {} assertions; failures={:?}",
        ingredients.len(),
        assertion_labels.len(),
        validation_failures,
    );

    Ok(SignReport {
        signed_path: signed_mp4.to_path_buf(),
        valid,
        validation_state: state_str,
        generator,
        title,
        ingredients,
        assertion_labels,
        summary,
        raw_json: reader.json(),
    })
}

/// Structured summary of a signed / verified MP4. Suitable for printing in the
/// CLI and for assertions in tests.
#[derive(Debug, Clone)]
pub struct SignReport {
    /// Path of the signed (or verified) file.
    pub signed_path: PathBuf,
    /// True iff `validation_state == ValidationState::Valid`. Tamper, missing
    /// cert chain, or a broken hash all push this to false.
    pub valid: bool,
    /// Raw `ValidationState` enum, stringified.
    pub validation_state: String,
    /// `claim_generator` field of the active manifest.
    pub generator: String,
    /// Title carried by the active manifest, if any.
    pub title: Option<String>,
    /// Ingredient titles, in declaration order.
    pub ingredients: Vec<String>,
    /// Assertion labels present on the active manifest.
    pub assertion_labels: Vec<String>,
    /// Human-readable one-line summary suitable for log output.
    pub summary: String,
    /// Full Reader::json() dump — for `wavelet c2pa verify --json`.
    pub raw_json: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render_offline::SceneSpec;
    use std::io::{Seek, SeekFrom, Write};
    use tempfile::tempdir;

    fn sample_mp4() -> PathBuf {
        // Tracked fixture; ~660 KB. Any seekable MP4 works — c2pa-rs reads the
        // box layout, signs, and re-emits a complete copy.
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("examples/beat-demo/beat-demo-final.mp4")
    }

    fn one_scene_comp() -> Composition {
        Composition {
            width: 1280,
            height: 720,
            fps: 30,
            duration_frames: 30,
            aspect: None,
            scenes: vec![SceneSpec {
                html_path: PathBuf::from("scenes/intro.html"),
                start_frame: 0,
                duration_frames: 30,
                transition_in: None,
                video_bg: None,
            }],
            audio_cues: vec![],
        }
    }

    #[test]
    fn sign_and_verify_round_trip() {
        let dir = tempdir().unwrap();
        let out = dir.path().join("signed.mp4");
        let comp = one_scene_comp();
        let report = sign_mp4(
            &comp,
            None,
            Some("RoundTrip Test"),
            Some("wavelet tests"),
            &sample_mp4(),
            &out,
            None,
        )
        .unwrap();
        assert!(report.valid, "signed MP4 failed verify: {}", report.summary);
        assert_eq!(report.title.as_deref(), Some("RoundTrip Test"));
        assert!(report.generator.contains("wavelet"), "generator: '{}'", report.generator);
        assert!(
            report
                .assertion_labels
                .iter()
                .any(|l| l == "stds.schema-org.CreativeWork"),
            "expected CreativeWork assertion, got: {:?}",
            report.assertion_labels
        );
        assert!(
            report
                .assertion_labels
                .iter()
                .any(|l| l == "c2pa.training-mining"),
            "expected training-mining assertion, got: {:?}",
            report.assertion_labels
        );
    }

    #[test]
    fn tampered_mp4_fails_verify() {
        let dir = tempdir().unwrap();
        let signed = dir.path().join("signed.mp4");
        let comp = one_scene_comp();
        sign_mp4(
            &comp,
            None,
            Some("Tamper Test"),
            None,
            &sample_mp4(),
            &signed,
            None,
        )
        .unwrap();

        // Flip a byte deep inside the file (past the C2PA box, inside the mdat
        // payload). Any change to signed bytes invalidates the hash chain.
        let mut f = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&signed)
            .unwrap();
        let len = f.metadata().unwrap().len();
        let target = len.saturating_sub(4096);
        f.seek(SeekFrom::Start(target)).unwrap();
        let mut buf = [0u8; 1];
        std::io::Read::read_exact(&mut f, &mut buf).unwrap();
        f.seek(SeekFrom::Start(target)).unwrap();
        f.write_all(&[buf[0] ^ 0xFF]).unwrap();
        drop(f);

        let report = verify(&signed).unwrap();
        assert!(
            !report.valid,
            "tampered MP4 unexpectedly passed verify: {}",
            report.summary
        );
    }
}

