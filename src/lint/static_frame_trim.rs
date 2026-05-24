//! Lint rule that walks scene HTML for `<video src="…">` references
//! and runs `freezedetect` against each clip. Surfaces findings when
//! a referenced clip has untrimmed leading or trailing static frames.
//!
//! Backstop for the automatic trim path in `wavelet shot txt2vid`:
//! when an agent imports a clip outside the normal ingest workflow
//! (manual `cp`, third-party Veo run, legacy cache replay), this lint
//! catches the case where the static frames slipped past auto-trim.

use crate::clip::trim_static::{analyze, DetectParams};
use crate::lint::report::{LintFinding, Severity};
use std::path::{Path, PathBuf};

/// Identifier emitted in `LintFinding.rule`.
pub const RULE: &str = "static-frame-trim";

/// Seconds of detected leading/trailing static at which we promote
/// from `Warn` to `Error`. 0.5s is enough to read as a stutter at
/// the cut point.
const ERROR_STATIC_SECS: f32 = 0.5;

/// Run the rule against every clip referenced by `scene_path`.
/// Returns one finding per untrimmed clip. Returns an empty vec when
/// the scene has no video refs OR ffmpeg is unavailable.
pub fn run(scene_path: &Path) -> Vec<LintFinding> {
    let Ok(html) = std::fs::read_to_string(scene_path) else {
        return Vec::new();
    };
    let base = scene_path.parent().unwrap_or(Path::new("."));
    let refs = video_refs(&html, base);
    let mut findings = Vec::new();
    let params = DetectParams::default();
    for clip in refs {
        let Ok(report) = analyze(&clip, params) else {
            // ffmpeg missing or unreadable clip — skip silently.
            // Don't fail the lint over an environment issue.
            continue;
        };
        let static_total = report.leading_freeze_s + report.trailing_freeze_s;
        if static_total < 0.1 {
            continue;
        }
        let severity = if report.unusable {
            Severity::Error
        } else if static_total >= ERROR_STATIC_SECS {
            Severity::Error
        } else {
            Severity::Warn
        };
        let detail = build_detail(&report);
        let fix_hint = build_fix_hint(&report, &clip);
        findings.push(LintFinding {
            rule: RULE.to_string(),
            severity,
            scene_path: scene_path.to_path_buf(),
            t_secs: 0.0,
            element_selector: clip
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("(clip)")
                .to_string(),
            element_bbox: crate::query::Rect { x: 0.0, y: 0.0, w: 0.0, h: 0.0 },
            message: detail,
            fix_hint,
            subkind: None,
        });
    }
    findings
}

fn build_detail(r: &crate::clip::trim_static::TrimReport) -> String {
    if r.unusable {
        return format!(
            "clip duration {:.2}s — only {:.2}s of detected motion ({:.2}s leading + \
             {:.2}s trailing static). Re-roll: too short to use after trim.",
            r.input_duration_s, r.motion_duration_s, r.leading_freeze_s, r.trailing_freeze_s,
        );
    }
    let mut parts: Vec<String> = Vec::new();
    if r.leading_freeze_s >= 0.1 {
        parts.push(format!("{:.2}s leading freeze", r.leading_freeze_s));
    }
    if r.trailing_freeze_s >= 0.1 {
        parts.push(format!("{:.2}s trailing freeze", r.trailing_freeze_s));
    }
    format!(
        "untrimmed static frames detected ({}) — clip duration {:.2}s, motion span \
         {:.2}s. Trim before compositing to avoid stutter at cut points.",
        parts.join(" + "),
        r.input_duration_s,
        r.motion_duration_s,
    )
}

fn build_fix_hint(r: &crate::clip::trim_static::TrimReport, clip: &Path) -> String {
    if r.unusable {
        return format!(
            "re-roll the clip — `wavelet shot txt2vid` with a fresh seed. The current \
             generation has too little motion to use."
        );
    }
    format!(
        "run `wavelet clip trim-static {} --out <trimmed.mp4>` (trims [{:.2}s, {:.2}s]) \
         and reference the trimmed file from the scene instead",
        clip.display(),
        r.trim_start_s,
        r.trim_end_s,
    )
}

/// Best-effort extraction of `<video src="…">` paths from a scene's
/// HTML. Resolves relative paths against `base`. Returns absolute
/// paths that exist on disk; missing files are skipped.
fn video_refs(html: &str, base: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    // Look for `src="…"` and `data-veo-src="…"` after a `<video` tag
    // OR a `<source` inside `<video>`. We're not parsing — just
    // scanning for src attributes whose values look like MP4 paths.
    for needle in ["<video", "<source"] {
        let mut search = html;
        while let Some(pos) = search.find(needle) {
            let after = &search[pos..];
            // Bounded scan: only look within this tag (until the next '>').
            let tag_end = after.find('>').unwrap_or(after.len());
            let tag = &after[..tag_end];
            if let Some(src) = extract_src(tag) {
                if looks_like_video(&src) {
                    let path = if Path::new(&src).is_absolute() {
                        PathBuf::from(&src)
                    } else {
                        base.join(&src)
                    };
                    if path.exists() && !out.contains(&path) {
                        out.push(path);
                    }
                }
            }
            search = &after[tag_end.min(after.len())..];
            if search.is_empty() {
                break;
            }
            // Step past the '>' we just looked at to avoid an infinite
            // loop when find() points to the same '<video'.
            if let Some(next) = search.find('>') {
                search = &search[next + 1..];
            } else {
                break;
            }
        }
    }
    out
}

fn extract_src(tag_chunk: &str) -> Option<String> {
    for attr in ["src=\"", "src='"] {
        if let Some(start) = tag_chunk.find(attr) {
            let rest = &tag_chunk[start + attr.len()..];
            let quote = attr.chars().last().unwrap();
            let end = rest.find(quote)?;
            return Some(rest[..end].to_string());
        }
    }
    None
}

fn looks_like_video(s: &str) -> bool {
    let lower = s.to_ascii_lowercase();
    lower.ends_with(".mp4") || lower.ends_with(".mov") || lower.ends_with(".webm")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_video_src_from_simple_tag() {
        let html = r#"<video src="shots/01.mp4" autoplay></video>"#;
        let refs = video_refs(html, Path::new("/tmp/nonexistent-base"));
        // Files don't exist so refs is empty — but extract_src in
        // isolation should find the path.
        assert_eq!(extract_src(r#"<video src="shots/01.mp4" autoplay"#), Some("shots/01.mp4".into()));
        // And the scan should at least try (path didn't exist so empty).
        assert_eq!(refs.len(), 0);
    }

    #[test]
    fn extracts_source_inside_video() {
        let chunk = r#"<source src='clips/04.mp4' type="video/mp4""#;
        assert_eq!(extract_src(chunk), Some("clips/04.mp4".into()));
    }

    #[test]
    fn skips_non_video_src() {
        assert!(!looks_like_video("logo.png"));
        assert!(looks_like_video("shot.MP4"));
        assert!(looks_like_video("clip.webm"));
    }
}
