//! # `wavelet::clipref` — clip-ref schema (wb-n33n.1)
//!
//! A **clip-ref** is a lineage-tracked HTML file (`.clip.html`) emitted by
//! every wavelet producer (shot / image / music / tts) AND optionally
//! hand-authored. The schema in this module is the source of truth for the
//! whole epic (wb-n33n).
//!
//! ## Format
//!
//! ```text
//! ---
//! clip: 01JQX9NXFVR2D5JBQGFCWQHZNX
//! kind: shot
//! asset: assets/shot-3.mp4
//! asset-hash: a1b2c3d4...
//! provider: google-veo-3.1
//! prompt: hand pouring water in a slow steady stream
//! created-at: "2026-05-20T14:30:00Z"
//! ---
//!
//! <video src="../../assets/shot-3.mp4" controls></video>
//! ```
//!
//! ## Authoring symmetry
//!
//! Same struct, same parser, same writer for hand-authored and procedurally
//! generated clip-refs. An `Overlay` written by a human round-trips
//! identically to a `Shot` emitted by Veo.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use ulid::Ulid;

mod path;
pub mod screenplay;
mod types;
mod walk;

pub use path::{default_path, slug_for};
pub use types::{ClipKind, EditKind};
pub use walk::{walk_refs, WalkedRef};

/// Errors produced by clip-ref parse / write / validate.
#[derive(Debug, thiserror::Error)]
pub enum ClipRefError {
    /// Input did not start with a `---` YAML front-matter delimiter.
    #[error("front matter missing or malformed")]
    FrontMatterMissing,
    /// Closing `---` delimiter not found after the opening one.
    #[error("body missing front-matter delimiter")]
    BodyMissingDelimiter,
    /// YAML decode failed.
    #[error("yaml decode: {0}")]
    YamlDecode(#[from] serde_yaml::Error),
    /// `parent`/`edit_kind` pairing constraint violated. Either both are
    /// present or both are absent — never one without the other.
    #[error("paired fields mismatch: parent and edit_kind must be set together")]
    PairedFieldsMismatch,
    /// ULID could not be parsed (only triggered when ULID arrives as a
    /// string in `extra` and is later promoted — `serde_yaml` already
    /// rejects malformed ULIDs on the primary `clip` / `parent` fields).
    #[error("invalid ulid: {0}")]
    InvalidUlid(String),
    /// Filesystem error on `write`.
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}

/// A clip-ref: HTML file with YAML front matter describing one wavelet clip.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub struct ClipRef {
    /// Primary identifier. ULIDs sort lexicographically by creation time.
    pub clip: Ulid,
    /// What this clip represents.
    pub kind: ClipKind,
    /// Path to the asset bytes, relative to the wavelet workdir.
    pub asset: PathBuf,
    /// sha256 of the asset bytes. Matches `backends::cache::Manifest::request_hash`
    /// when the clip-ref was emitted by a producer.
    pub asset_hash: String,
    /// Provider that produced this clip. `"manual"` for hand-authored.
    pub provider: String,
    /// Natural-language description. May be empty for `Overlay` /
    /// `ScreenplayScene` where prompt isn't applicable.
    pub prompt: String,
    /// ISO-8601 UTC creation time.
    pub created_at: DateTime<Utc>,

    /// Provider-specific model slug (e.g. `veo-3.1-generate-preview`).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub model: Option<String>,

    /// Cost in USD reported by the provider.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub cost_usd: Option<f32>,

    /// The clip-ref this was derived from. Forms the edit chain.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub parent: Option<Ulid>,

    /// Set iff `parent` is set.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub edit_kind: Option<EditKind>,

    /// The edit instruction (e.g. "make the sky bluer").
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub edit_prompt: Option<String>,

    /// Free-form tags.
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub tags: Vec<String>,

    /// Scene slug for `ScreenplayScene` and `Shot`.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub scene: Option<String>,

    /// Forward-compat catch-all. Unknown YAML keys land here and survive
    /// round-trip. Kebab-cased keys are preserved as-is.
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_yaml::Value>,
}

impl ClipRef {
    /// Parse a `.clip.html` file. Returns the `ClipRef` and the HTML body
    /// (everything after the closing `---`, including the leading newline).
    pub fn parse(input: &str) -> Result<(ClipRef, String), ClipRefError> {
        let rest = input
            .strip_prefix("---\n")
            .or_else(|| input.strip_prefix("---\r\n"))
            .ok_or(ClipRefError::FrontMatterMissing)?;

        let (yaml, body) = split_on_closing_delimiter(rest)
            .ok_or(ClipRefError::BodyMissingDelimiter)?;

        let clip_ref: ClipRef = serde_yaml::from_str(yaml)?;
        clip_ref.validate()?;
        Ok((clip_ref, body.to_string()))
    }

    /// Write a `.clip.html` file. Front matter is serialized YAML; body is
    /// appended verbatim. The body's leading whitespace is preserved.
    pub fn write(&self, path: &Path, body: &str) -> Result<(), ClipRefError> {
        self.validate()?;
        let yaml = serde_yaml::to_string(self)?;
        let mut out = String::with_capacity(yaml.len() + body.len() + 16);
        out.push_str("---\n");
        out.push_str(&yaml);
        if !yaml.ends_with('\n') {
            out.push('\n');
        }
        out.push_str("---");
        out.push_str(body);

        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, out)?;
        Ok(())
    }

    /// Cross-field validation. Runs after parse and before write.
    fn validate(&self) -> Result<(), ClipRefError> {
        if self.parent.is_some() != self.edit_kind.is_some() {
            return Err(ClipRefError::PairedFieldsMismatch);
        }
        Ok(())
    }
}

/// Locate the closing `---` delimiter. The delimiter must sit on its own
/// line. Returns `(yaml_before, body_after_including_leading_newline)`.
fn split_on_closing_delimiter(input: &str) -> Option<(&str, &str)> {
    let mut search_from = 0usize;
    while let Some(idx) = input[search_from..].find("---") {
        let abs = search_from + idx;
        let starts_line = abs == 0 || input.as_bytes()[abs - 1] == b'\n';
        let after = abs + 3;
        let ends_line = after == input.len()
            || matches!(input.as_bytes()[after], b'\n' | b'\r');
        if starts_line && ends_line {
            return Some((&input[..abs], &input[after..]));
        }
        search_from = abs + 3;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    fn sample() -> ClipRef {
        ClipRef {
            clip: Ulid::from_str("01JQX9NXFVR2D5JBQGFCWQHZNX").unwrap(),
            kind: ClipKind::Shot,
            asset: PathBuf::from("assets/shot-3.mp4"),
            asset_hash: "a1b2c3d4e5f6".into(),
            provider: "google-veo-3.1".into(),
            prompt: "hand pouring water in a slow steady stream".into(),
            created_at: DateTime::parse_from_rfc3339("2026-05-20T14:30:00Z")
                .unwrap()
                .with_timezone(&Utc),
            model: Some("veo-3.1-generate-preview".into()),
            cost_usd: Some(0.20),
            parent: None,
            edit_kind: None,
            edit_prompt: None,
            tags: vec![],
            scene: Some("INT. KITCHEN - DAY".into()),
            extra: BTreeMap::new(),
        }
    }

    #[test]
    fn round_trip() {
        let clip = sample();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sample.clip.html");
        let body = "\n\n<video src=\"../../assets/shot-3.mp4\" controls></video>\n";
        clip.write(&path, body).unwrap();

        let raw = fs::read_to_string(&path).unwrap();
        let (parsed, parsed_body) = ClipRef::parse(&raw).unwrap();
        assert_eq!(parsed, clip);
        assert_eq!(parsed_body, body);
    }

    #[test]
    fn unknown_keys_preserved() {
        let raw = "---\n\
            clip: 01JQX9NXFVR2D5JBQGFCWQHZNX\n\
            kind: overlay\n\
            asset: assets/title.html\n\
            asset-hash: deadbeef\n\
            provider: manual\n\
            prompt: \"\"\n\
            created-at: \"2026-05-20T14:30:00Z\"\n\
            extra-future-field: 42\n\
            another-unknown: hello\n\
            ---\n\
            <p>card</p>\n";

        let (clip, _body) = ClipRef::parse(raw).unwrap();
        assert_eq!(
            clip.extra.get("extra-future-field"),
            Some(&serde_yaml::Value::Number(42.into()))
        );
        assert_eq!(
            clip.extra.get("another-unknown"),
            Some(&serde_yaml::Value::String("hello".into()))
        );

        let yaml = serde_yaml::to_string(&clip).unwrap();
        assert!(yaml.contains("extra-future-field: 42"));
        assert!(yaml.contains("another-unknown: hello"));
    }

    #[test]
    fn paired_fields_mismatch_errors() {
        let raw = "---\n\
            clip: 01JQX9NXFVR2D5JBQGFCWQHZNX\n\
            kind: shot\n\
            asset: assets/shot-3.mp4\n\
            asset-hash: a1b2c3d4\n\
            provider: google-veo-3.1\n\
            prompt: x\n\
            created-at: \"2026-05-20T14:30:00Z\"\n\
            parent: 01JQX0000000000000000000AA\n\
            ---\n\
            body\n";

        match ClipRef::parse(raw) {
            Err(ClipRefError::PairedFieldsMismatch) => {}
            other => panic!("expected PairedFieldsMismatch, got {other:?}"),
        }
    }

    #[test]
    fn slug_stability() {
        let a = slug_for(
            ClipKind::Shot,
            Some("INT. KITCHEN - DAY"),
            "hand pouring water in a slow steady stream",
            "a1b2c3d4e5f6",
        );
        let b = slug_for(
            ClipKind::Shot,
            Some("INT. KITCHEN - DAY"),
            "hand pouring water in a slow steady stream",
            "a1b2c3d4e5f6",
        );
        assert_eq!(a, b, "same inputs should produce same slug");

        let c = slug_for(
            ClipKind::Shot,
            Some("INT. KITCHEN - DAY"),
            "a totally different prompt entirely",
            "a1b2c3d4e5f6",
        );
        assert_ne!(a, c, "different prompt should change slug");
        assert!(a.ends_with("a1b2c3"), "slug ends in 6-char asset-hash prefix: {a}");
    }

    #[test]
    fn default_path_layout() {
        let p = default_path(Path::new("/work"), ClipKind::ScreenplayScene, "my-slug");
        assert_eq!(p, PathBuf::from("/work/refs/screenplay-scene/my-slug.clip.html"));
    }

    #[test]
    fn empty_prompt_ok_for_overlay() {
        let mut clip = sample();
        clip.kind = ClipKind::Overlay;
        clip.prompt = String::new();
        clip.provider = "manual".into();
        clip.scene = None;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("overlay.clip.html");
        clip.write(&path, "\n<p>hi</p>\n").unwrap();
        let raw = fs::read_to_string(&path).unwrap();
        let (parsed, _) = ClipRef::parse(&raw).unwrap();
        assert_eq!(parsed.kind, ClipKind::Overlay);
        assert!(parsed.prompt.is_empty());
    }
}
