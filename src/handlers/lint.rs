//! `wavelet lint` orchestrator. Resolves the input path to a list of
//! scene HTML files, runs each enabled rule against each scene, then
//! emits a `LintReport` in the requested format.

use crate::cli_args::LintOp;
use crate::lint::glyph_clip as glyph_clip_rule;
use crate::lint::report::{LintReport, Severity};
use crate::lint::safe_zone as safe_zone_rule;
use crate::lint::safe_zones;
use crate::query::FrameSnapshot;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

/// Entry point dispatched from `wavelet lint`.
pub fn run(op: LintOp) -> ExitCode {
    let scenes = match resolve_scenes(&op.path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("wavelet lint: {e}");
            return ExitCode::from(3);
        }
    };
    if scenes.is_empty() {
        eprintln!(
            "wavelet lint: no scenes found at {}",
            op.path.display()
        );
        return ExitCode::from(3);
    }

    let rules_run = filter_rules(&op.rules);
    let (canvas_w, canvas_h) = infer_canvas(&op.aspect, &scenes);

    let mut report = LintReport {
        scenes_checked: scenes.len(),
        rules_run: rules_run.iter().map(|r| r.to_string()).collect(),
        platform: op.platform.clone(),
        findings: Vec::new(),
    };

    let safe_zone_table = if rules_run.contains(&safe_zone_rule::RULE) {
        match safe_zones::load_table() {
            Ok(t) => Some(t),
            Err(e) => {
                eprintln!("wavelet lint: failed to load safe-zone table: {e}");
                return ExitCode::from(3);
            }
        }
    } else {
        None
    };

    let scaled_zone = match (&op.platform, &safe_zone_table) {
        (Some(p), Some(t)) => match t.get(p) {
            Ok(Some(z)) => Some(t.scaled(z, canvas_w as f32, canvas_h as f32)),
            Ok(None) => None,
            Err(e) => {
                eprintln!("wavelet lint: {e}");
                return ExitCode::from(3);
            }
        },
        _ => None,
    };

    for scene_path in &scenes {
        let t_secs = op.at.unwrap_or(1.0);
        let snap = FrameSnapshot::from_html(scene_path, canvas_w, canvas_h, t_secs);

        if rules_run.contains(&safe_zone_rule::RULE) {
            if let (Some(zone), Some(platform)) = (scaled_zone.as_ref(), op.platform.as_deref()) {
                let mut fs = safe_zone_rule::run(&snap, scene_path, zone, platform);
                report.findings.append(&mut fs);
            }
        }

        if rules_run.contains(&glyph_clip_rule::RULE) {
            let mut fs = glyph_clip_rule::run(&snap, scene_path);
            report.findings.append(&mut fs);
        }
    }

    match op.format.as_str() {
        "json" => emit_json(&report),
        _ => emit_text(&report),
    }

    if report.exit_code() == 0 {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(report.exit_code())
    }
}

fn filter_rules(requested: &[String]) -> Vec<&'static str> {
    let available = [safe_zone_rule::RULE, glyph_clip_rule::RULE];
    let mut out = Vec::new();
    for r in requested {
        let want = r.trim();
        if want.is_empty() {
            continue;
        }
        if let Some(known) = available.iter().find(|k| **k == want) {
            out.push(*known);
        } else {
            eprintln!(
                "wavelet lint: skipping unknown rule `{want}` (available: {})",
                available.join(", ")
            );
        }
    }
    if out.is_empty() {
        out.extend_from_slice(&available);
    }
    out
}

fn infer_canvas(aspect: &Option<String>, scenes: &[PathBuf]) -> (u32, u32) {
    if let Some(a) = aspect {
        if let Some(d) = aspect_to_canvas(a) {
            return d;
        }
    }
    for scene in scenes {
        if let Some(d) = parse_meta_resolution(scene) {
            return d;
        }
    }
    (1080, 1920)
}

fn aspect_to_canvas(a: &str) -> Option<(u32, u32)> {
    match a.trim() {
        "9:16" => Some((1080, 1920)),
        "16:9" => Some((1920, 1080)),
        "1:1" => Some((1080, 1080)),
        "4:5" => Some((1080, 1350)),
        _ => None,
    }
}

fn parse_meta_resolution(path: &Path) -> Option<(u32, u32)> {
    let txt = std::fs::read_to_string(path).ok()?;
    let needle = "name=\"resolution\"";
    let pos = txt.find(needle)?;
    let after = &txt[pos..];
    let content_pos = after.find("content=")?;
    let rest = &after[content_pos + 8..];
    let quote = rest.chars().next()?;
    let end = rest[1..].find(quote)?;
    let val = &rest[1..1 + end];
    let (w, h) = val.split_once('x')?;
    Some((w.trim().parse().ok()?, h.trim().parse().ok()?))
}

/// Resolve `<PATH>` to a flat list of scene HTML files.
fn resolve_scenes(path: &Path) -> Result<Vec<PathBuf>, String> {
    if path.is_dir() {
        return scenes_in_dir(path);
    }
    if !path.exists() {
        return Err(format!("path not found: {}", path.display()));
    }
    if let Some(scenes) = scenes_from_manifest(path)? {
        return Ok(scenes);
    }
    Ok(vec![path.to_path_buf()])
}

fn scenes_in_dir(dir: &Path) -> Result<Vec<PathBuf>, String> {
    let mut out: Vec<PathBuf> = Vec::new();
    for entry in std::fs::read_dir(dir).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let p = entry.path();
        if p.extension().and_then(|s| s.to_str()) == Some("html") {
            out.push(p);
        }
    }
    out.sort();
    Ok(out)
}

/// Detect the `commercial.html`-style manifest format and return the
/// referenced scene paths (resolved relative to the manifest file).
/// Returns `Ok(None)` when the file isn't a manifest — i.e. it's just
/// a regular scene HTML file the caller should lint directly.
fn scenes_from_manifest(path: &Path) -> Result<Option<Vec<PathBuf>>, String> {
    let html = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    if !html.contains("data-scene-href") {
        return Ok(None);
    }
    let base_dir = path.parent().unwrap_or(Path::new("."));
    let mut out = Vec::new();
    for chunk in html.split("data-scene-href").skip(1) {
        let after_eq = match chunk.split_once('=') {
            Some((_, rest)) => rest.trim_start(),
            None => continue,
        };
        let quote = match after_eq.chars().next() {
            Some(c) if c == '"' || c == '\'' => c,
            _ => continue,
        };
        let body = &after_eq[1..];
        let end = match body.find(quote) {
            Some(i) => i,
            None => continue,
        };
        let rel = &body[..end];
        out.push(base_dir.join(rel));
    }
    Ok(Some(out))
}

fn emit_json(report: &LintReport) {
    #[derive(serde::Serialize)]
    struct Wire<'a> {
        scenes_checked: usize,
        rules_run: &'a [String],
        platform: &'a Option<String>,
        findings: &'a [crate::lint::report::LintFinding],
        exit_code: u8,
    }
    let wire = Wire {
        scenes_checked: report.scenes_checked,
        rules_run: &report.rules_run,
        platform: &report.platform,
        findings: &report.findings,
        exit_code: report.exit_code(),
    };
    match serde_json::to_string_pretty(&wire) {
        Ok(s) => println!("{s}"),
        Err(e) => eprintln!("wavelet lint: failed to serialize report: {e}"),
    }
}

fn emit_text(report: &LintReport) {
    let header_rules = report.rules_run.join(", ");
    let header_platform = report
        .platform
        .as_deref()
        .map(|p| format!(", platform={p}"))
        .unwrap_or_default();
    println!(
        "  {} scenes checked, {} rule{} run ({}){}",
        report.scenes_checked,
        report.rules_run.len(),
        if report.rules_run.len() == 1 { "" } else { "s" },
        header_rules,
        header_platform,
    );
    println!();

    for f in &report.findings {
        println!(
            "{}  {}  {} @ t={:.1}s",
            f.severity.label(),
            f.rule,
            f.scene_path.display(),
            f.t_secs,
        );
        println!("       element: {}", f.element_selector);
        println!(
            "       bbox: x={} y={} w={} h={}",
            f.element_bbox.x as i32,
            f.element_bbox.y as i32,
            f.element_bbox.w as i32,
            f.element_bbox.h as i32,
        );
        println!("       detail: {}", f.message);
        println!("       fix: {}", f.fix_hint);
        println!();
    }

    let errors = report.error_count();
    let warns = report.warn_count();
    let infos = report.findings.len() - errors - warns;
    println!(
        "Summary: {} error{}, {} warning{}{} across {} scene{}.",
        errors,
        if errors == 1 { "" } else { "s" },
        warns,
        if warns == 1 { "" } else { "s" },
        if infos > 0 {
            format!(", {infos} info")
        } else {
            String::new()
        },
        report.scenes_checked,
        if report.scenes_checked == 1 { "" } else { "s" },
    );
    if errors > 0 {
        println!("Exit code 1 (any error fails the lint).");
    } else {
        let _ = Severity::Error;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn write(path: &Path, body: &str) {
        let mut f = std::fs::File::create(path).unwrap();
        f.write_all(body.as_bytes()).unwrap();
    }

    #[test]
    fn manifest_parses_scene_refs() {
        let dir = std::env::temp_dir().join("wavelet-lint-manifest");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let manifest = dir.join("commercial.html");
        write(
            &manifest,
            r#"<!doctype html><html><body>
<section data-scene-href="scenes/01.html"></section>
<section data-scene-href='scenes/02.html'></section>
</body></html>"#,
        );
        let scenes = scenes_from_manifest(&manifest).unwrap().unwrap();
        assert_eq!(scenes.len(), 2);
        assert!(scenes[0].ends_with("scenes/01.html"));
        assert!(scenes[1].ends_with("scenes/02.html"));
    }

    #[test]
    fn non_manifest_returns_none() {
        let dir = std::env::temp_dir().join("wavelet-lint-plain");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let scene = dir.join("a.html");
        write(
            &scene,
            "<!doctype html><html><body><h1>hello</h1></body></html>",
        );
        assert!(scenes_from_manifest(&scene).unwrap().is_none());
    }

    #[test]
    fn aspect_maps_known_ratios() {
        assert_eq!(aspect_to_canvas("9:16"), Some((1080, 1920)));
        assert_eq!(aspect_to_canvas("16:9"), Some((1920, 1080)));
        assert_eq!(aspect_to_canvas("1:1"), Some((1080, 1080)));
        assert_eq!(aspect_to_canvas("4:5"), Some((1080, 1350)));
        assert!(aspect_to_canvas("garbage").is_none());
    }

    #[test]
    fn meta_resolution_parses() {
        let dir = std::env::temp_dir().join("wavelet-lint-meta");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("a.html");
        write(
            &p,
            r#"<!doctype html><html><head><meta name="resolution" content="720x1280"></head><body></body></html>"#,
        );
        assert_eq!(parse_meta_resolution(&p), Some((720, 1280)));
    }
}
