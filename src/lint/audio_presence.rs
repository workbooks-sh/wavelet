//! Audio-presence rule — flags commercials shipped silent. A motion
//! commercial without music or voiceover almost always reads as broken
//! to the recipient; this lint catches the case where the agent
//! considers a render done despite having no audio anywhere.
//!
//! Scope: workdir-scoped, runs once per `wavelet lint` invocation
//! (like color-grade-coherence). Dispatched outside the per-scene loop
//! in `handlers/lint.rs`.
//!
//! Detection:
//! 1. Walk `commercial.html` and every `scenes/*.html` for `<audio src>`
//!    references. Resolve refs against the workdir root.
//! 2. Scan `music/` and `voiceover/` (and `vo/`) for `.wav` / `.mp3`
//!    / `.flac` / `.ogg` / `.m4a` files.
//! 3. Notice sidecar `commercial.wav` next to `commercial.mp4`.
//! 4. If `commercial.mp4` exists, probe it via rsmpeg for audio streams.
//!
//! Severity gradient (worst case wins):
//!   * No reference + no asset + no mp4 stream → Error
//!   * Reference present but file missing → Error
//!   * Asset on disk but mp4 has 0 audio streams → Warn (mux dropped)
//!   * Asset present and mp4 has >= 1 audio stream → Pass (no finding)

use super::report::{LintFinding, Severity};
use crate::query::Rect;
use rsmpeg::avformat::AVFormatContextInput;
use rsmpeg::ffi;
use std::ffi::CString;
use std::path::{Path, PathBuf};

/// Identifier emitted in `LintFinding.rule`.
pub const RULE: &str = "audio-presence";

/// Audio file extensions we recognise on disk.
const AUDIO_EXTS: &[&str] = &["wav", "mp3", "flac", "ogg", "m4a", "aac"];

/// Directories searched for music/voiceover assets, relative to workdir.
const AUDIO_DIRS: &[&str] = &["music", "voiceover", "vo"];

/// What `run` discovered. Useful for tests; the orchestrator only
/// consumes `findings`.
#[derive(Debug, Clone)]
pub struct AudioPresenceOutcome {
    /// The workdir that was inspected (parent of `commercial.html` or
    /// the directory argument itself).
    pub workdir: PathBuf,
    /// `<audio src="...">` references discovered, resolved to absolute
    /// paths against the workdir.
    pub html_refs: Vec<PathBuf>,
    /// Audio asset files discovered on disk under AUDIO_DIRS or as
    /// sidecars next to `commercial.mp4`.
    pub disk_assets: Vec<PathBuf>,
    /// References that point at a file that doesn't exist on disk.
    pub missing_refs: Vec<PathBuf>,
    /// Number of audio streams in `commercial.mp4`. `None` when the
    /// file is absent.
    pub mp4_audio_streams: Option<usize>,
    /// Whether `commercial.mp4` exists.
    pub mp4_present: bool,
    /// Findings to merge into the parent `LintReport`. At most one
    /// finding is emitted per run.
    pub findings: Vec<LintFinding>,
}

/// Resolve the workdir given a `<PATH>` arg.
///
/// - file → its parent dir
/// - dir → itself
pub fn discover_workdir(path: &Path) -> Option<PathBuf> {
    if path.is_file() {
        return path.parent().map(PathBuf::from);
    }
    if path.is_dir() {
        return Some(path.to_path_buf());
    }
    None
}

/// Run the rule against a workdir-or-manifest path.
pub fn run(path: &Path) -> Result<AudioPresenceOutcome, String> {
    let Some(workdir) = discover_workdir(path) else {
        return Ok(AudioPresenceOutcome {
            workdir: path.to_path_buf(),
            html_refs: Vec::new(),
            disk_assets: Vec::new(),
            missing_refs: Vec::new(),
            mp4_audio_streams: None,
            mp4_present: false,
            findings: vec![informational(path, "path not resolvable — audio-presence not applicable")],
        });
    };

    let mut html_files: Vec<PathBuf> = Vec::new();
    let commercial = workdir.join("commercial.html");
    if commercial.is_file() {
        html_files.push(commercial);
    }
    let scenes_dir = workdir.join("scenes");
    if scenes_dir.is_dir() {
        if let Ok(rd) = std::fs::read_dir(&scenes_dir) {
            let mut scene_html: Vec<PathBuf> = rd
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.extension().and_then(|s| s.to_str()) == Some("html"))
                .collect();
            scene_html.sort();
            html_files.extend(scene_html);
        }
    }

    let mut html_refs: Vec<PathBuf> = Vec::new();
    let mut missing_refs: Vec<PathBuf> = Vec::new();
    let mut refs_per_file: Vec<(PathBuf, usize)> = Vec::new();
    for html in &html_files {
        let body = match std::fs::read_to_string(html) {
            Ok(b) => b,
            Err(_) => continue,
        };
        let base = html.parent().unwrap_or(&workdir);
        let mut n_here = 0usize;
        for src in extract_audio_srcs(&body) {
            let resolved = base.join(&src);
            n_here += 1;
            if resolved.exists() {
                html_refs.push(resolved);
            } else {
                missing_refs.push(resolved);
            }
        }
        refs_per_file.push((html.clone(), n_here));
    }

    let mut disk_assets = scan_audio_dirs(&workdir);
    let sidecar = workdir.join("commercial.wav");
    if sidecar.is_file() {
        disk_assets.push(sidecar);
    }

    let mp4_path = workdir.join("commercial.mp4");
    let mp4_present = mp4_path.is_file();
    let mp4_audio_streams = if mp4_present {
        Some(count_audio_streams(&mp4_path).unwrap_or(0))
    } else {
        None
    };

    let findings = build_findings(
        &workdir,
        &html_files,
        &refs_per_file,
        &html_refs,
        &missing_refs,
        &disk_assets,
        mp4_present,
        mp4_audio_streams,
    );

    Ok(AudioPresenceOutcome {
        workdir,
        html_refs,
        disk_assets,
        missing_refs,
        mp4_audio_streams,
        mp4_present,
        findings,
    })
}

fn informational(scope: &Path, message: &str) -> LintFinding {
    LintFinding {
        rule: RULE.to_string(),
        severity: Severity::Info,
        scene_path: scope.to_path_buf(),
        t_secs: 0.0,
        element_selector: String::from("workdir/"),
        element_bbox: Rect { x: 0.0, y: 0.0, w: 0.0, h: 0.0 },
        message: message.to_string(),
        fix_hint: String::new(),
        subkind: None,
    }
}

/// Extract `src` values from every `<audio ... src="...">` occurrence in
/// the HTML body. Cheap string scan — the FrameSnapshot pipeline doesn't
/// retain `<audio>` (it's not a layout-bearing element) so a substring
/// pass is the right tool.
pub fn extract_audio_srcs(body: &str) -> Vec<String> {
    let mut out = Vec::new();
    let lc = body.to_ascii_lowercase();
    let mut cursor = 0;
    while let Some(pos) = lc[cursor..].find("<audio") {
        let start = cursor + pos;
        let tag_end = match lc[start..].find('>') {
            Some(p) => start + p,
            None => break,
        };
        let tag = &body[start..tag_end];
        if let Some(src) = extract_src_attr(tag) {
            if !src.is_empty() {
                out.push(src);
            }
        }
        cursor = tag_end + 1;
    }
    out
}

fn extract_src_attr(tag: &str) -> Option<String> {
    let lc = tag.to_ascii_lowercase();
    let key_pos = lc.find("src")?;
    let after = &tag[key_pos + 3..];
    let eq = after.find('=')?;
    let rest = after[eq + 1..].trim_start();
    let quote = rest.chars().next()?;
    if quote != '"' && quote != '\'' {
        return None;
    }
    let body = &rest[1..];
    let end = body.find(quote)?;
    Some(body[..end].to_string())
}

/// Walk every AUDIO_DIRS subdir of workdir, returning every file with a
/// recognised audio extension.
pub fn scan_audio_dirs(workdir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for dir_name in AUDIO_DIRS {
        let dir = workdir.join(dir_name);
        if !dir.is_dir() {
            continue;
        }
        let Ok(rd) = std::fs::read_dir(&dir) else { continue };
        for entry in rd.flatten() {
            let p = entry.path();
            let Some(ext) = p.extension().and_then(|s| s.to_str()) else {
                continue;
            };
            if AUDIO_EXTS.iter().any(|e| ext.eq_ignore_ascii_case(e)) {
                out.push(p);
            }
        }
    }
    out.sort();
    out
}

/// Count audio streams in an MP4 via rsmpeg. Matches the pattern used
/// by `decode_rgba_frames` — open the container, walk streams, count
/// `AVMEDIA_TYPE_AUDIO`. Returns Err on probe failure; the caller
/// converts that to "0 streams" so a corrupt mp4 still gets flagged.
pub fn count_audio_streams(path: &Path) -> Result<usize, String> {
    let path_c =
        CString::new(path.to_string_lossy().into_owned()).map_err(|e| format!("path: {e}"))?;
    let input_ctx =
        AVFormatContextInput::open(&path_c).map_err(|e| format!("open: {e}"))?;
    let n = input_ctx
        .streams()
        .iter()
        .filter(|s| s.codecpar().codec_type == ffi::AVMEDIA_TYPE_AUDIO)
        .count();
    Ok(n)
}

fn finding(workdir: &Path, sev: Severity, msg: String, fix: &str) -> LintFinding {
    LintFinding {
        rule: RULE.to_string(),
        severity: sev,
        scene_path: workdir.to_path_buf(),
        t_secs: 0.0,
        element_selector: String::from("workdir/"),
        element_bbox: Rect { x: 0.0, y: 0.0, w: 0.0, h: 0.0 },
        message: msg.trim_end().to_string(),
        fix_hint: fix.to_string(),
        subkind: None,
    }
}

fn build_findings(
    workdir: &Path,
    html_files: &[PathBuf],
    refs_per_file: &[(PathBuf, usize)],
    html_refs: &[PathBuf],
    missing_refs: &[PathBuf],
    disk_assets: &[PathBuf],
    mp4_present: bool,
    mp4_audio_streams: Option<usize>,
) -> Vec<LintFinding> {
    let any_ref = !html_refs.is_empty();
    let any_asset = !disk_assets.is_empty();
    let mp4_has_audio = matches!(mp4_audio_streams, Some(n) if n > 0);
    let summary = scan_summary(workdir, html_files, refs_per_file, mp4_present, mp4_audio_streams);

    // Pass: at least one asset on disk AND the final mp4 carries audio.
    // When there's no mp4 yet, having an asset+ref is enough.
    if mp4_present {
        if any_asset && mp4_has_audio && missing_refs.is_empty() {
            return Vec::new();
        }
    } else if any_asset && missing_refs.is_empty() {
        return Vec::new();
    }

    if !missing_refs.is_empty() {
        let mut msg = String::from("audio reference points at a missing file\n");
        for r in missing_refs {
            msg.push_str(&format!("         missing: {}\n", display_rel(workdir, r)));
        }
        msg.push_str(&summary);
        return vec![finding(
            workdir,
            Severity::Error,
            msg,
            "fix the <audio src> path so it resolves to an existing \
             file under the workdir, or generate the missing track via \
             `wavelet music gen --prompt \"<style>\" --duration <secs> \
             --out music/track.wav`",
        )];
    }

    if !any_ref && !any_asset && !mp4_has_audio {
        let msg = format!(
            "no music or voiceover found anywhere in the workdir\n{summary}"
        );
        return vec![finding(
            workdir,
            Severity::Error,
            msg,
            "generate a music track via `wavelet music gen --prompt \
             \"<style>\" --duration <secs> --out music/track.wav`, then \
             add an `<audio src=\"music/track.wav\">` to commercial.html. \
             Voiceover is optional and brief-dependent; music is not.",
        )];
    }

    if any_asset && mp4_present && !mp4_has_audio {
        let lead = &disk_assets[0];
        let size = std::fs::metadata(lead).map(|m| m.len()).unwrap_or(0);
        let msg = format!(
            "{} exists ({}) but commercial.mp4 has no audio stream — the renderer dropped the audio\n{}",
            display_rel(workdir, lead),
            human_size(size),
            summary.trim_end(),
        );
        return vec![finding(
            workdir,
            Severity::Warn,
            msg,
            "re-render with `wavelet render commercial.html --out \
             commercial.mp4` and confirm rsmpeg muxes the wav",
        )];
    }

    if any_ref && !any_asset && !mp4_has_audio {
        let msg = format!(
            "an <audio> reference is declared but no on-disk asset was located\n{}",
            summary.trim_end(),
        );
        return vec![finding(
            workdir,
            Severity::Warn,
            msg,
            "produce the referenced asset (e.g. `wavelet music gen --out \
             music/track.wav`) before rendering",
        )];
    }

    Vec::new()
}

fn scan_summary(
    workdir: &Path,
    html_files: &[PathBuf],
    refs_per_file: &[(PathBuf, usize)],
    mp4_present: bool,
    mp4_audio_streams: Option<usize>,
) -> String {
    let is_named = |p: &Path, name: &str| p.file_name().and_then(|s| s.to_str()) == Some(name);
    let in_scenes = |p: &Path| {
        p.parent()
            .and_then(|pp| pp.file_name())
            .and_then(|s| s.to_str())
            == Some("scenes")
    };
    let commercial_refs: usize = refs_per_file
        .iter()
        .filter(|(p, _)| is_named(p, "commercial.html"))
        .map(|(_, n)| *n)
        .sum();
    let scene_refs: usize = refs_per_file
        .iter()
        .filter(|(p, _)| in_scenes(p))
        .map(|(_, n)| *n)
        .sum();
    let scenes_count = html_files.iter().filter(|p| in_scenes(p)).count();
    let plur = |n: usize, w: &str| {
        format!("{n} {w}{}", if n == 1 { "" } else { "s" })
    };
    let music = workdir.join("music");
    let vo = workdir.join("voiceover");
    let vo_alt = workdir.join("vo");
    let music_line = if music.is_dir() {
        plur(count_in(&music), "audio file")
    } else {
        String::from("does not exist")
    };
    let vo_line = if vo.is_dir() {
        plur(count_in(&vo), "audio file")
    } else if vo_alt.is_dir() {
        format!("vo/ {}", plur(count_in(&vo_alt), "audio file"))
    } else {
        String::from("does not exist")
    };
    let mp4_line = if mp4_present {
        format!("audio streams: {}", mp4_audio_streams.unwrap_or(0))
    } else {
        String::from("not rendered yet")
    };
    format!(
        "         scanned: commercial.html ({} <audio> {})\n\
         \x20                 scenes/*.html ({} across {})\n\
         \x20                 music/ ({})\n\
         \x20                 voiceover/ ({})\n\
         \x20                 commercial.mp4 ({})",
        commercial_refs,
        if commercial_refs == 1 { "ref" } else { "refs" },
        plur(scene_refs, "ref"),
        plur(scenes_count, "file"),
        music_line,
        vo_line,
        mp4_line,
    )
}

fn count_in(dir: &Path) -> usize {
    let Ok(rd) = std::fs::read_dir(dir) else { return 0 };
    rd.flatten()
        .filter(|e| {
            let p = e.path();
            p.extension()
                .and_then(|s| s.to_str())
                .map(|ext| AUDIO_EXTS.iter().any(|a| ext.eq_ignore_ascii_case(a)))
                .unwrap_or(false)
        })
        .count()
}

fn display_rel(workdir: &Path, path: &Path) -> String {
    path.strip_prefix(workdir)
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| path.display().to_string())
}

fn human_size(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = 1024 * 1024;
    if bytes >= MB {
        format!("{:.1} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.1} KB", bytes as f64 / KB as f64)
    } else {
        format!("{bytes} B")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mk(dir: &Path) -> PathBuf {
        let _ = std::fs::remove_dir_all(dir);
        std::fs::create_dir_all(dir).unwrap();
        dir.to_path_buf()
    }

    #[test]
    fn extract_audio_srcs_finds_double_quoted() {
        let html = r#"<!doctype html><html><body>
<audio src="music/track.wav" data-volume="0.85"></audio>
</body></html>"#;
        let srcs = extract_audio_srcs(html);
        assert_eq!(srcs, vec!["music/track.wav"]);
    }

    #[test]
    fn extract_audio_srcs_finds_single_quoted_and_multiple() {
        let html = r#"<audio src='a.wav'></audio><div></div><audio src="b.mp3"></audio>"#;
        let srcs = extract_audio_srcs(html);
        assert_eq!(srcs, vec!["a.wav", "b.mp3"]);
    }

    #[test]
    fn extract_audio_srcs_ignores_unrelated_src() {
        let html = r#"<img src="hero.png"><audio src="music/track.wav"></audio>"#;
        let srcs = extract_audio_srcs(html);
        assert_eq!(srcs, vec!["music/track.wav"]);
    }

    #[test]
    fn scan_audio_dirs_picks_up_music_wav() {
        let dir = mk(&std::env::temp_dir().join("wavelet-lint-audio-music"));
        std::fs::create_dir_all(dir.join("music")).unwrap();
        std::fs::write(dir.join("music/track.wav"), b"riff").unwrap();
        let assets = scan_audio_dirs(&dir);
        assert_eq!(assets.len(), 1);
        assert!(assets[0].ends_with("music/track.wav"));
    }

    #[test]
    fn scan_audio_dirs_walks_vo_alias() {
        let dir = mk(&std::env::temp_dir().join("wavelet-lint-audio-vo-alias"));
        std::fs::create_dir_all(dir.join("vo")).unwrap();
        std::fs::write(dir.join("vo/line-1.mp3"), b"id3").unwrap();
        let assets = scan_audio_dirs(&dir);
        assert_eq!(assets.len(), 1);
    }

    #[test]
    fn no_audio_anywhere_emits_error() {
        let dir = mk(&std::env::temp_dir().join("wavelet-lint-audio-silent"));
        std::fs::write(
            dir.join("commercial.html"),
            "<!doctype html><html><body></body></html>",
        )
        .unwrap();
        let outcome = run(&dir.join("commercial.html")).unwrap();
        assert_eq!(outcome.findings.len(), 1);
        assert_eq!(outcome.findings[0].severity, Severity::Error);
        assert!(outcome.findings[0]
            .message
            .contains("no music or voiceover found"));
        assert!(outcome.findings[0].fix_hint.contains("wavelet music gen"));
    }

    #[test]
    fn ref_to_missing_file_emits_error() {
        let dir = mk(&std::env::temp_dir().join("wavelet-lint-audio-missing"));
        std::fs::write(
            dir.join("commercial.html"),
            r#"<!doctype html><audio src="music/missing.wav"></audio>"#,
        )
        .unwrap();
        let outcome = run(&dir.join("commercial.html")).unwrap();
        assert_eq!(outcome.findings.len(), 1);
        assert_eq!(outcome.findings[0].severity, Severity::Error);
        assert!(outcome.findings[0].message.contains("missing"));
        assert_eq!(outcome.missing_refs.len(), 1);
    }

    #[test]
    fn asset_present_no_mp4_passes() {
        let dir = mk(&std::env::temp_dir().join("wavelet-lint-audio-prerender"));
        std::fs::create_dir_all(dir.join("music")).unwrap();
        std::fs::write(dir.join("music/track.wav"), b"riff").unwrap();
        std::fs::write(
            dir.join("commercial.html"),
            r#"<!doctype html><audio src="music/track.wav"></audio>"#,
        )
        .unwrap();
        let outcome = run(&dir.join("commercial.html")).unwrap();
        assert!(outcome.findings.is_empty(), "expected pass, got {:?}", outcome.findings);
    }

    #[test]
    fn discover_workdir_handles_file_and_dir() {
        let dir = mk(&std::env::temp_dir().join("wavelet-lint-audio-discover"));
        std::fs::write(dir.join("commercial.html"), "<!doctype html>").unwrap();
        assert_eq!(discover_workdir(&dir.join("commercial.html")).unwrap(), dir);
        assert_eq!(discover_workdir(&dir).unwrap(), dir);
    }

    #[test]
    fn scenes_directory_audio_refs_are_walked() {
        let dir = mk(&std::env::temp_dir().join("wavelet-lint-audio-scenes"));
        std::fs::create_dir_all(dir.join("scenes")).unwrap();
        std::fs::create_dir_all(dir.join("music")).unwrap();
        std::fs::write(dir.join("music/track.wav"), b"riff").unwrap();
        std::fs::write(
            dir.join("commercial.html"),
            "<!doctype html><html><body></body></html>",
        )
        .unwrap();
        std::fs::write(
            dir.join("scenes/01.html"),
            r#"<audio src="../music/track.wav"></audio>"#,
        )
        .unwrap();
        let outcome = run(&dir.join("commercial.html")).unwrap();
        assert_eq!(outcome.html_refs.len(), 1);
        assert!(outcome.findings.is_empty());
    }
}
