//! Hallucinated-attribute rule — flags `data-*` attributes that agents
//! commonly invent but the wavelet compose pre-pass does not process.
//!
//! Symptom (wb-a2z2, eval 008): agent generated `<section data-video-bg=
//! "shots/X.mp4" data-scene-href="scenes/01.html">` in commercial.html.
//! The compose pre-pass dropped both attributes silently. Result: $12+
//! of orphaned Veo gens, type-only output with no video underneath, agent
//! had no signal anything went wrong.
//!
//! Detection (intentionally lightweight regex scan, not a real HTML
//! parser):
//!   - Walk `commercial.html` and each `scenes/*.html`
//!   - Match any of HALLUCINATED_ATTRS as bare HTML attribute names
//!   - Emit one Error finding per occurrence, naming the file + the
//!     correct surface (wavelet-clip or raw <video> element)
//!
//! Scope: workdir-scoped, runs once per `wavelet lint` invocation
//! (mirrors audio-presence dispatch).

use super::report::{LintFinding, Severity};
use crate::query::Rect;
use std::path::{Path, PathBuf};

fn finding(scene_path: PathBuf, message: String, fix: String) -> LintFinding {
    LintFinding {
        rule: RULE.to_string(),
        severity: Severity::Error,
        scene_path,
        t_secs: 0.0,
        element_selector: String::from("html/"),
        element_bbox: Rect { x: 0.0, y: 0.0, w: 0.0, h: 0.0 },
        message,
        fix_hint: fix,
        subkind: None,
    }
}

/// Identifier emitted in `LintFinding.rule`.
pub const RULE: &str = "hallucinated-attrs";

/// Attribute names that agents have been observed inventing but wavelet's
/// compose pre-pass ignores. Each is paired with a one-line remediation
/// hint. Keep this list tight — only attributes confirmed to cause real
/// spend leaks belong here.
const HALLUCINATED_ATTRS: &[(&str, &str)] = &[
    (
        "data-video-bg",
        "use a raw <video src=\"...\" autoplay muted loop playsinline> element \
         inside the scene HTML (absolutely positioned), OR a <wavelet-clip src=\"....clip.html\"> reference",
    ),
    (
        "data-scene-href",
        "scenes are listed via the standard manifest pattern wavelet render recognizes — \
         don't add custom data-* routing attributes",
    ),
    (
        "data-shot",
        "shots are referenced via <wavelet-clip src=\"...\"> or a raw <video src=\"...\"> element, \
         not via data-shot",
    ),
    (
        "data-clip",
        "clips are referenced via <wavelet-clip src=\"...\">; data-clip is not processed",
    ),
];

/// What `run` discovered. Findings list mirrors other workdir-scoped
/// lint rules (audio-presence, color-grade-coherence).
pub struct Outcome {
    /// Per-occurrence findings — at most one per (file, hallucinated
    /// attribute) pair.
    pub findings: Vec<LintFinding>,
}

/// Scan `commercial.html` and `scenes/*.html` under `workdir` for
/// hallucinated wavelet-specific `data-*` attributes that the compose
/// pre-pass does not process. Returns an `Outcome` with one Error
/// finding per occurrence.
pub fn run(workdir: &Path) -> Result<Outcome, std::io::Error> {
    let mut findings = Vec::new();

    let mut targets: Vec<PathBuf> = Vec::new();
    let commercial = workdir.join("commercial.html");
    if commercial.exists() {
        targets.push(commercial);
    }
    let scenes_dir = workdir.join("scenes");
    if scenes_dir.is_dir() {
        for entry in std::fs::read_dir(&scenes_dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("html") {
                targets.push(path);
            }
        }
    }

    for file in &targets {
        let html = match std::fs::read_to_string(file) {
            Ok(s) => s,
            Err(_) => continue,
        };
        for (attr, hint) in HALLUCINATED_ATTRS {
            // Match the attribute as a word boundary on both sides — avoids
            // false positives like `data-video-bg-color` (legitimately custom).
            let needle = format!(" {attr}=");
            let needle_open = format!("\t{attr}=");
            if !html.contains(&needle) && !html.contains(&needle_open) {
                continue;
            }
            // Count occurrences crudely (one finding per file mention is
            // enough; we don't need a per-line report).
            let count = html.matches(&needle).count() + html.matches(&needle_open).count();
            let rel = file
                .strip_prefix(workdir)
                .unwrap_or(file)
                .display()
                .to_string();
            findings.push(finding(
                file.to_path_buf(),
                format!(
                    "hallucinated wavelet attribute `{attr}` in {rel} ({count}x). \
                     The compose pre-pass silently drops this — any referenced \
                     video/asset will be orphaned (no error, no warning, no render)."
                ),
                hint.to_string(),
            ));
        }
    }

    Ok(Outcome { findings })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(p: &Path, body: &str) {
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, body).unwrap();
    }

    #[test]
    fn flags_data_video_bg_on_section() {
        let tmp = tempfile::tempdir().unwrap();
        let workdir = tmp.path();
        write(
            &workdir.join("commercial.html"),
            r#"<!doctype html><html><body><section data-video-bg="shots/x.mp4"></section></body></html>"#,
        );
        let outcome = run(workdir).unwrap();
        assert_eq!(outcome.findings.len(), 1);
        let f = &outcome.findings[0];
        assert_eq!(f.severity, Severity::Error);
        assert!(f.message.contains("data-video-bg"));
        // The actionable fix lives in fix_hint, not message.
        assert!(f.fix_hint.contains("wavelet-clip") || f.fix_hint.contains("<video"));
    }

    #[test]
    fn flags_multiple_hallucinated_attrs_in_one_file() {
        let tmp = tempfile::tempdir().unwrap();
        let workdir = tmp.path();
        write(
            &workdir.join("commercial.html"),
            r#"<section data-video-bg="a.mp4" data-scene-href="s/01.html"></section>"#,
        );
        let outcome = run(workdir).unwrap();
        // One finding per distinct attribute name (not per occurrence).
        assert_eq!(outcome.findings.len(), 2);
    }

    #[test]
    fn does_not_flag_legitimate_data_attrs() {
        let tmp = tempfile::tempdir().unwrap();
        let workdir = tmp.path();
        write(
            &workdir.join("commercial.html"),
            r#"<div data-testid="foo" data-cue-id="music-1">ok</div>"#,
        );
        let outcome = run(workdir).unwrap();
        assert_eq!(outcome.findings.len(), 0);
    }

    #[test]
    fn scans_scene_html_files_too() {
        let tmp = tempfile::tempdir().unwrap();
        let workdir = tmp.path();
        write(&workdir.join("commercial.html"), "<html></html>");
        write(
            &workdir.join("scenes/01.html"),
            r#"<section data-shot="../shots/x.mp4"></section>"#,
        );
        let outcome = run(workdir).unwrap();
        assert_eq!(outcome.findings.len(), 1);
        assert!(outcome.findings[0]
            .scene_path
            .to_string_lossy()
            .contains("scenes/01.html"));
    }

    #[test]
    fn returns_empty_outcome_when_workdir_has_nothing_to_scan() {
        let tmp = tempfile::tempdir().unwrap();
        let outcome = run(tmp.path()).unwrap();
        assert!(outcome.findings.is_empty());
    }
}
