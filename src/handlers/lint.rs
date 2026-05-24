//! `wavelet lint` orchestrator. Resolves the input path to a list of
//! scene HTML files, runs each enabled rule against each scene, then
//! emits a `LintReport` in the requested format.

use crate::cli_args::LintOp;
use crate::lint::audio_presence as audio_presence_rule;
use crate::lint::baked_text_ocr as baked_text_ocr_rule;
use crate::lint::color_grade as color_grade_rule;
use crate::lint::glyph_clip as glyph_clip_rule;
use crate::lint::hallucinated_attrs as hallucinated_attrs_rule;
use crate::lint::layout_axis as layout_axis_rule;
use crate::lint::mp4_frames;
use crate::lint::report::{LintReport, Severity};
use crate::lint::static_frame_trim as static_frame_trim_rule;
use crate::lint::safe_zone as safe_zone_rule;
use crate::lint::safe_zones;
use crate::lint::text_on_subject as text_on_subject_rule;
use crate::lint::text_readability as text_readability_rule;
use crate::lint::text_readability_contrast as text_readability_contrast_rule;
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
    let (canvas_w, canvas_h) = infer_canvas(&op.aspect, &op.path, &scenes);
    let aspect_class = text_readability_rule::classify_aspect(
        op.aspect.as_deref(),
        canvas_w,
        canvas_h,
    );

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

    // When `--at` is unset we sample at one settled-midpoint time
    // per scene. The 3-sample multi-time scan (0.11, 0.5, 1.0) was
    // intended to catch animation-driven geometry changes (letter-
    // spacing keyframes, font-variation interpolation) but the
    // 008 post-mortem showed it inflated lint runtime to 145s for a
    // typical 8-scene commercial — agent burned half its budget on
    // lint iterations that never advanced because the cost was so
    // high. The vast majority of text overlays are static across
    // their scene window; a single t=1.0s sample is sufficient.
    // Callers who genuinely care about animation-start clip cases
    // can opt back in with multi-`--at` invocations or by extending
    // this list locally; the dedup logic handles overlap regardless.
    let sample_times: Vec<f32> = match op.at {
        Some(t) => vec![t],
        None => vec![1.0],
    };

    let mut contrast_cache = text_readability_contrast_rule::ContrastFrameCache::new();
    for scene_path in &scenes {
        let mut per_scene: Vec<crate::lint::report::LintFinding> = Vec::new();
        for &t_secs in &sample_times {
            let snap = FrameSnapshot::from_html(scene_path, canvas_w, canvas_h, t_secs);

            if rules_run.contains(&safe_zone_rule::RULE) {
                if let (Some(zone), Some(platform)) =
                    (scaled_zone.as_ref(), op.platform.as_deref())
                {
                    let mut fs = safe_zone_rule::run(&snap, scene_path, zone, platform);
                    per_scene.append(&mut fs);
                }
            }

            if rules_run.contains(&glyph_clip_rule::RULE) {
                let mut fs = glyph_clip_rule::run(&snap, scene_path);
                per_scene.append(&mut fs);
            }

            if rules_run.contains(&layout_axis_rule::RULE) {
                let mut fs = layout_axis_rule::run(&snap, scene_path);
                per_scene.append(&mut fs);
            }

            if rules_run.contains(&static_frame_trim_rule::RULE) {
                let mut fs = static_frame_trim_rule::run(scene_path);
                per_scene.append(&mut fs);
            }

            if rules_run.contains(&text_readability_rule::RULE) {
                let mut fs = text_readability_rule::run(&snap, scene_path, aspect_class);
                per_scene.append(&mut fs);
                // Contrast pass — independent of cap-height. Same rule
                // identifier; subkind distinguishes them in dedup.
                let mut fs2 = text_readability_contrast_rule::run(
                    &snap,
                    scene_path,
                    &mut contrast_cache,
                );
                per_scene.append(&mut fs2);
            }

            // text-on-subject: opt-in, depth-model-gated. Runs once per
            // sample time. Passes the MP4 path (if any) so the rule can
            // sample actual rendered video frames rather than the HTML
            // placeholder.
            if rules_run.contains(&text_on_subject_rule::RULE) {
                let mut fs = text_on_subject_rule::run(
                    &snap,
                    scene_path,
                    op.mp4.as_deref(),
                );
                per_scene.append(&mut fs);
            }
        }

        // Dedup: per (rule, element_selector, subkind) keep the
        // worst-severity finding. Sampling multiple times can surface
        // the same defect twice; the reader doesn't want both. The
        // subkind dimension keeps cap-height + contrast findings on
        // the same element from collapsing into each other.
        per_scene.sort_by(|a, b| {
            (a.rule.as_str(), a.element_selector.as_str(), a.subkind.as_deref().unwrap_or(""))
                .cmp(&(b.rule.as_str(), b.element_selector.as_str(), b.subkind.as_deref().unwrap_or("")))
                .then_with(|| severity_rank(b.severity).cmp(&severity_rank(a.severity)))
        });
        per_scene.dedup_by(|a, b| {
            a.rule == b.rule
                && a.element_selector == b.element_selector
                && a.subkind == b.subkind
        });
        report.findings.append(&mut per_scene);
    }

    // Post-render contrast pass — when an MP4 was provided, sample
    // frames from the actual composited output and run the halo-
    // contrast measurement against those pixels. This is the only
    // stage that sees the same pixels the viewer will: white text
    // over Veo-rendered countertops, scrim panels burned in by the
    // encoder, etc. Findings carry `subkind: "contrast-rendered"`
    // so they coexist with the lint-time HTML-render contrast pass.
    if let Some(mp4_path) = &op.mp4 {
        if rules_run.contains(&text_readability_rule::RULE) {
            let duration = mp4_frames::probe_duration_secs(mp4_path).unwrap_or(12.0);
            // 4 evenly-spaced samples avoid the first/last keyframe
            // edge cases. For a 12s spot: 1.5, 4.5, 7.5, 10.5.
            let n_samples = 4;
            for i in 0..n_samples {
                let frac = (i as f32 + 0.5) / n_samples as f32;
                let t = (duration * frac).max(0.0);
                let snap = FrameSnapshot::from_html(&op.path, canvas_w, canvas_h, t);
                let Some(frame) =
                    mp4_frames::sample_frame_rgba(mp4_path, t, canvas_w, canvas_h)
                else {
                    eprintln!(
                        "wavelet lint: ffmpeg sample failed at t={t:.2}s — skipping"
                    );
                    continue;
                };
                let mut fs = crate::lint::text_readability_contrast::run_against_frame(
                    &snap, &op.path, &frame,
                );
                for f in fs.iter_mut() {
                    f.subkind = Some("contrast-rendered".to_string());
                    f.message = format!(
                        "{}  (sampled from final MP4 at t={:.2}s)",
                        f.message, t
                    );
                }
                report.findings.append(&mut fs);
            }
        }
    }

    if rules_run.contains(&color_grade_rule::RULE) {
        match color_grade_rule::run(&op.path) {
            Ok(mut outcome) => report.findings.append(&mut outcome.findings),
            Err(e) => {
                eprintln!("wavelet lint: color-grade-coherence: {e}");
                return ExitCode::from(3);
            }
        }
    }

    if rules_run.contains(&audio_presence_rule::RULE) {
        match audio_presence_rule::run(&op.path) {
            Ok(mut outcome) => report.findings.append(&mut outcome.findings),
            Err(e) => {
                eprintln!("wavelet lint: audio-presence: {e}");
                return ExitCode::from(3);
            }
        }
    }

    // wb-a2z2: catch the hallucinated attribute names (data-video-bg,
    // data-scene-href, etc.) that the compose pre-pass silently drops.
    if rules_run.contains(&hallucinated_attrs_rule::RULE) {
        match hallucinated_attrs_rule::run(&op.path) {
            Ok(mut outcome) => report.findings.append(&mut outcome.findings),
            Err(e) => {
                eprintln!("wavelet lint: hallucinated-attrs: {e}");
                return ExitCode::from(3);
            }
        }
    }

    // Baked-text OCR pass — requires --mp4. Samples 4 frames from the final
    // composited MP4 and runs PaddleOCR v5 ONNX to catch garbled letterforms
    // that Veo or other AI generators sometimes produce. The `ocr` cargo
    // feature gate is inside baked_text_ocr::run; when the feature is off it
    // returns one Info finding describing how to opt in.
    if rules_run.contains(&baked_text_ocr_rule::RULE) {
        if let Some(mp4_path) = &op.mp4 {
            let html = std::fs::read_to_string(&op.path).unwrap_or_default();
            let expected_tokens = baked_text_ocr_rule::extract_brand_tokens(&html);
            let mut fs = baked_text_ocr_rule::run(
                mp4_path,
                &op.path,
                &expected_tokens,
                canvas_w,
                canvas_h,
            );
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

fn severity_rank(s: Severity) -> u8 {
    match s {
        Severity::Error => 3,
        Severity::Warn => 2,
        Severity::Info => 1,
    }
}

fn filter_rules(requested: &[String]) -> Vec<&'static str> {
    // `text-on-subject` is intentionally excluded from the default set —
    // it requires the `depth` feature + model download and is opt-in via
    // `--rules text-on-subject`.
    let available = [
        safe_zone_rule::RULE,
        glyph_clip_rule::RULE,
        layout_axis_rule::RULE,
        color_grade_rule::RULE,
        text_readability_rule::RULE,
        audio_presence_rule::RULE,
        hallucinated_attrs_rule::RULE,
        static_frame_trim_rule::RULE,
        text_on_subject_rule::RULE,
        baked_text_ocr_rule::RULE,
    ];
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

fn infer_canvas(aspect: &Option<String>, input: &Path, scenes: &[PathBuf]) -> (u32, u32) {
    if let Some(a) = aspect {
        if let Some(d) = aspect_to_canvas(a) {
            return d;
        }
    }
    // Canonical source: the manifest (`input`) almost always carries
    // the `<meta name="resolution">`. Check it FIRST — before falling
    // back to per-scene meta or walking the workdir — because the 008
    // post-mortem showed the walk-up logic mis-resolving canvas on
    // relative scene paths (`scenes/foo.html`) where `parent().parent()`
    // returns an empty path and `read_dir("")` fails silently.
    if let Some(d) = parse_meta_resolution(input) {
        return d;
    }
    // Per-scene meta (rare — scene HTMLs usually inherit from manifest).
    for scene in scenes {
        if let Some(d) = parse_meta_resolution(scene) {
            return d;
        }
    }
    // Last-resort directory walk: look for ANY .html sibling of the
    // first scene's workdir with a meta. Robust against relative-path
    // edge cases by canonicalizing the parent.
    if let Some(first) = scenes.first() {
        let parent = first
            .parent()
            .and_then(|p| p.parent())
            .filter(|p| !p.as_os_str().is_empty())
            .map(|p| p.to_path_buf())
            .or_else(|| Some(std::env::current_dir().ok()?));
        if let Some(dir) = parent {
            if let Ok(read) = std::fs::read_dir(&dir) {
                for entry in read.flatten() {
                    let p = entry.path();
                    if p.extension().and_then(|s| s.to_str()) == Some("html") {
                        if let Some(d) = parse_meta_resolution(&p) {
                            return d;
                        }
                    }
                }
            }
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
        let has_bbox = f.element_bbox.w > 0.0 && f.element_bbox.h > 0.0;
        if has_bbox {
            println!("       element: {}", f.element_selector);
            println!(
                "       bbox: x={} y={} w={} h={}",
                f.element_bbox.x as i32,
                f.element_bbox.y as i32,
                f.element_bbox.w as i32,
                f.element_bbox.h as i32,
            );
        }
        if f.message.contains('\n') {
            println!("       detail: {}", f.message);
        } else {
            println!("       detail: {}", f.message);
        }
        if !f.fix_hint.is_empty() {
            println!("       fix: {}", f.fix_hint);
        }
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
