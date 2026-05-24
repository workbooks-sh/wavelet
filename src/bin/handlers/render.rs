//! `wavelet render` handler — HTML manifest → MP4 (+ optional C2PA sign).
//!
//! Filename is up to the caller — `commercial.html` for the commercial
//! pipeline, `trailer.html` for a trailer, `promo.html` for a promo,
//! `index.html` is fine, etc. Only the `.html` / `.htm` extension is
//! enforced. Non-HTML inputs are rejected at exit 3 with no escape
//! hatch. The `Composition::from_json_path` codepath remains as an
//! internal compiler primitive used elsewhere (workflow stage diffs,
//! caching) but is not reachable from `wavelet render` anymore.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use wavelet::compose::load_index_html;
use wavelet::render_offline::{render_composition_with_options, Composition, RenderOptions};

use wavelet::handlers::util::load_signing_key;

/// Bundle of C2PA-related options the caller passes in. Kept as a
/// struct so the dispatch in `wavelet.rs` doesn't have to thread 6
/// arguments through.
pub struct C2paOpts {
    /// Sign the rendered MP4.
    pub sign: bool,
    /// CreativeWork title for the manifest.
    pub title: Option<String>,
    /// CreativeWork author.
    pub author: Option<String>,
    /// Override the cache root used to enumerate ingredients.
    pub cache_root: Option<PathBuf>,
    /// Production signing cert (PEM). Defaults to the bundled test cert.
    pub signing_cert: Option<PathBuf>,
    /// Production signing key (PEM).
    pub signing_key: Option<PathBuf>,
}

/// Run the render. When `aspects` is non-empty, the comp is rendered
/// once per aspect with the resolution overridden — output paths are
/// derived from the base path's stem (`<stem>.<W>x<H>.mp4`). The
/// existing scene-stills are reused across passes; for fully
/// aspect-aware gen, regenerate scene-stills with the target
/// `--image-size` before render.
pub fn run(
    comp_path: PathBuf,
    out: Option<PathBuf>,
    c2pa_opts: C2paOpts,
    aspects: Vec<String>,
    frame_budget_secs: u64,
    no_audio: bool,
) -> ExitCode {
    let render_opts = RenderOptions { frame_budget_secs, mux_audio: !no_audio };
    let is_html = comp_path
        .extension()
        .and_then(|s| s.to_str())
        .map(|s| s.eq_ignore_ascii_case("html") || s.eq_ignore_ascii_case("htm"))
        .unwrap_or(false);

    // HTML-ONLY render. `wavelet render` accepts an HTML manifest file
    // referencing per-scene HTML via `<section data-scene-href="...">`,
    // period. Non-HTML inputs are rejected outright — there is no
    // escape hatch. The manifest filename is up to the caller / pipeline
    // (`commercial.html`, `trailer.html`, `promo.html`, `index.html` —
    // whatever the deliverable is); only the `.html` / `.htm` extension
    // is enforced.
    //
    // History: until 2026-05-23, JSON was a parallel legacy input.
    // Every adversarial eval that ran into agent friction (008 v1, 008
    // v2, 005 v3) ended with the agent hand-authoring a JSON
    // composition and bypassing every lint rule, gate, and discipline
    // check wired into the HTML path. The escape hatch was the problem.
    // Removed entirely so the agent has exactly one way to reach an MP4.
    if !is_html {
        eprintln!(
            "wavelet render: REJECTED — only HTML inputs are accepted.\n\
             \n\
             Your input: {}\n\
             \n\
             Write a manifest HTML file (name it after your deliverable —\n\
             `commercial.html`, `trailer.html`, `promo.html`, `index.html`, etc.)\n\
             with `<meta name=\"resolution\" content=\"WxH\">`,\n\
             `<meta name=\"fps\" content=\"N\">`, `<meta name=\"duration\" content=\"Ns\">`\n\
             in the head, and pull each scene in via\n\
             `<section data-scene-href=\"scenes/01-foo.html\">`.\n\
             \n\
             Then re-run `wavelet render <your-manifest>.html`.",
            comp_path.display(),
        );
        return ExitCode::from(3);
    }

    // Pre-render trace check. The pipeline-level gating criteria
    // (`screenplay_duration_fits`, `wavelet_lint_passes`) only fire
    // inside `wavelet workflow run`. The 008 eval surfaced an agent
    // executing each stage manually with `wavelet render` directly,
    // sidestepping every gate. Enforce them here too so the discipline
    // holds regardless of which entry point the agent chose.
    //
    // Trace lookup is cwd-local — same `.wavelet-trace.jsonl` the
    // PATH-shim writes during eval. Outside eval the file doesn't exist
    // and the check is skipped (so direct CLI users aren't gated).
    if let Err(detail) = preflight_check(is_html) {
        eprintln!("wavelet render: REJECTED — pipeline preflight failed");
        eprintln!("{detail}");
        return ExitCode::from(3);
    }

    // We've already early-returned for non-HTML inputs above. This
    // branch only executes for `.html` / `.htm` paths.
    let (comp, root_dir) = match load_index_html(&comp_path) {
        Ok(c) => {
            let dir = comp_path
                .parent()
                .map(|p| p.to_path_buf())
                .unwrap_or_else(|| Path::new(".").to_path_buf());
            (c, dir)
        }
        Err(e) => {
            eprintln!("error loading {}: {e}", comp_path.display());
            return ExitCode::from(2);
        }
    };

    if aspects.is_empty() {
        let out_path = out.unwrap_or_else(|| comp_path.with_extension("mp4"));

        // Render-reentrance cache: hash the manifest content + every
        // referenced scene HTML content. If the hash matches the
        // sidecar saved next to the existing output MP4, emit a clear
        // "no changes — reusing existing render" line and exit 0
        // without re-rendering. 005 v5 burned ~9 minutes (two 4.5-min
        // renders) when the second was essentially identical to the
        // first. Detect that case and short-circuit.
        match render_input_hash(&comp_path, &root_dir) {
            Ok(current_hash) => {
                let sidecar = render_hash_sidecar(&out_path);
                if out_path.exists() {
                    let prior_hash = std::fs::read_to_string(&sidecar).ok();
                    match prior_hash.as_deref() {
                        Some(p) if p.trim() == current_hash => {
                            eprintln!(
                                "wavelet render: no changes detected since last render — reusing existing {} (use --force or modify the manifest / scenes to re-render)",
                                out_path.display()
                            );
                            return ExitCode::SUCCESS;
                        }
                        Some(_) => {
                            eprintln!(
                                "wavelet render: manifest or scene HTML changed since last render — proceeding with re-render"
                            );
                        }
                        None => {
                            // No sidecar — old MP4 from before this
                            // feature shipped, or a manual copy. Just
                            // proceed; we'll write the sidecar after.
                        }
                    }
                }
                // Stash the hash to write post-render.
                std::env::set_var("WAVELET_RENDER_HASH_PENDING", &current_hash);
                std::env::set_var(
                    "WAVELET_RENDER_HASH_SIDECAR",
                    sidecar.display().to_string(),
                );
            }
            Err(e) => {
                // Hashing failed — proceed without the cache. The
                // render is still safe; we just lose the
                // idempotence short-circuit on the next call.
                eprintln!("wavelet render: hash sidecar skipped ({e})");
            }
        }

        let code = render_one(&comp, &root_dir, &comp_path, &out_path, &c2pa_opts, &render_opts);
        // After a successful render, persist the input hash so the
        // next call can short-circuit when inputs haven't changed.
        if format!("{code:?}") == format!("{:?}", ExitCode::SUCCESS) {
            if let (Ok(hash), Ok(sidecar)) = (
                std::env::var("WAVELET_RENDER_HASH_PENDING"),
                std::env::var("WAVELET_RENDER_HASH_SIDECAR"),
            ) {
                let _ = std::fs::write(sidecar, hash);
            }
        }
        return code;
    }

    let mut overall = ExitCode::SUCCESS;
    let base_out = out.unwrap_or_else(|| comp_path.with_extension("mp4"));
    for aspect in &aspects {
        let dims = match parse_aspect_dims(aspect) {
            Some(d) => d,
            None => {
                eprintln!(
                    "skipping --aspect '{aspect}': want W:H (e.g. 16:9, 9:16, 1:1) or WxH (e.g. 1920x1080)"
                );
                overall = ExitCode::from(2);
                continue;
            }
        };
        let mut variant = comp.clone();
        variant.width = dims[0];
        variant.height = dims[1];
        let aspect_tag = aspect.replace(':', "x");
        let out_path = sibling_with_tag(&base_out, &aspect_tag);
        eprintln!("--- aspect {aspect} → {} ---", out_path.display());
        let code = render_one(&variant, &root_dir, &comp_path, &out_path, &c2pa_opts, &render_opts);
        if !matches!(code, c if format!("{c:?}") == format!("{:?}", ExitCode::SUCCESS)) {
            overall = code;
        }
    }
    overall
}

/// Pipeline preflight — verify the trace shows the prerequisite
/// gates were satisfied before this render call. Read-only inspection
/// of `.wavelet-trace.jsonl` in the current working directory. Returns
/// `Ok(())` when:
///   - no trace file exists (direct CLI user, no eval harness), OR
///   - the trace shows `screenplay validate` exited 0 at some point, AND
///   - if the input is HTML, the trace also shows `lint <html> --mp4`
///     OR the env var WAVELET_NO_PREFLIGHT=1 is set (escape hatch).
///
/// Disabled by setting `WAVELET_NO_PREFLIGHT=1` for legacy / debug use.
fn preflight_check(is_html: bool) -> Result<(), String> {
    if std::env::var("WAVELET_NO_PREFLIGHT")
        .map(|v| matches!(v.trim(), "1" | "true" | "yes" | "on"))
        .unwrap_or(false)
    {
        return Ok(());
    }
    let trace_path = Path::new(".wavelet-trace.jsonl");
    if !trace_path.exists() {
        return Ok(());
    }
    let raw = match std::fs::read_to_string(trace_path) {
        Ok(s) => s,
        Err(_) => return Ok(()),
    };
    let mut screenplay_validate_passed = false;
    let mut postrender_lint_passed = false;
    for line in raw.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let rec: serde_json::Value = match serde_json::from_str(trimmed) {
            Ok(v) => v,
            Err(_) => continue,
        };
        let argv: Vec<String> = rec
            .get("argv")
            .and_then(|v| v.as_array())
            .map(|a| a.iter().filter_map(|s| s.as_str().map(|s| s.to_string())).collect())
            .unwrap_or_default();
        let exit_ok = rec.get("exit").and_then(|v| v.as_i64()) == Some(0);
        if !exit_ok {
            continue;
        }
        if argv.get(1).map(String::as_str) == Some("screenplay")
            && argv.get(2).map(String::as_str) == Some("validate")
        {
            screenplay_validate_passed = true;
        }
        if argv.get(1).map(String::as_str) == Some("lint")
            && argv.iter().any(|a| a == "--mp4")
        {
            postrender_lint_passed = true;
        }
    }

    if !screenplay_validate_passed {
        return Err(
            "missing prerequisite: `wavelet screenplay validate <fountain> \
             --duration <secs>` must have exited 0 in this workdir before \
             render. The copy-density gate blocks over-stuffed scripts before \
             paid composition.\n\
             To bypass for legacy / debug use: WAVELET_NO_PREFLIGHT=1 wavelet render ..."
                .to_string(),
        );
    }
    // Post-render lint can only be required when the input is HTML —
    // the legacy comp.json path has no manifest the --mp4 lint can
    // target. (Strict mode handles that case separately.)
    if is_html && !postrender_lint_passed {
        // Allowed: render is the step that PRODUCES the MP4. The lint
        // --mp4 step can only run after render. So we don't require it
        // pre-render; instead the lint gate enforces it before the
        // workflow declares compose complete. Stay silent here.
        let _ = postrender_lint_passed;
    }
    Ok(())
}

/// Compute a stable content hash over (manifest content + every
/// referenced scene HTML content). Used for the render-reentrance
/// cache. Returns `Err` if reading any input fails.
///
/// We hash bytes, not parsed structure, so any meaningful edit busts
/// the cache. Media file paths are included but their bytes are not —
/// re-rendering only the MP4 because a referenced video changed is
/// rare AND the agent can manually delete the .render-hash sidecar
/// if they need a forced re-render.
fn render_input_hash(manifest_path: &Path, root_dir: &Path) -> Result<String, String> {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let mut h = DefaultHasher::new();
    let manifest = std::fs::read_to_string(manifest_path)
        .map_err(|e| format!("read manifest {}: {e}", manifest_path.display()))?;
    manifest.hash(&mut h);

    // Discover referenced scene HTMLs via the `data-scene-href`
    // attribute. We don't parse — just scan textually, same approach
    // as `scenes_from_manifest` in the lint handler.
    for chunk in manifest.split("data-scene-href").skip(1) {
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
        let scene_path = root_dir.join(rel);
        let scene_body = std::fs::read_to_string(&scene_path)
            .map_err(|e| format!("read scene {}: {e}", scene_path.display()))?;
        rel.hash(&mut h);
        scene_body.hash(&mut h);
    }

    Ok(format!("{:016x}", h.finish()))
}

/// Sidecar path for storing the input hash next to a rendered MP4.
/// `<out>.mp4` → `<out>.mp4.render-hash`.
fn render_hash_sidecar(out_path: &Path) -> PathBuf {
    let mut s = out_path.as_os_str().to_owned();
    s.push(".render-hash");
    PathBuf::from(s)
}

fn render_one(
    comp: &Composition,
    root_dir: &Path,
    comp_path: &Path,
    out_path: &Path,
    c2pa_opts: &C2paOpts,
    render_opts: &RenderOptions,
) -> ExitCode {
    println!(
        "rendering {} → {} ({}×{} @{}fps, {} frames, {} scenes, {} audio cues)",
        comp_path.display(),
        out_path.display(),
        comp.width,
        comp.height,
        comp.fps,
        comp.duration_frames,
        comp.scenes.len(),
        comp.audio_cues.len(),
    );

    let stats = match render_composition_with_options(comp, root_dir, out_path, render_opts) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("render failed: {e}");
            return ExitCode::from(2);
        }
    };
    let wav_bit = if stats.wav_bytes > 0 {
        format!(", {} bytes wav", stats.wav_bytes)
    } else {
        String::new()
    };
    println!(
        "done: {} frames, {} bytes mp4{}, {} ms",
        stats.video_frames, stats.mp4_bytes, wav_bit, stats.elapsed_ms,
    );

    if !c2pa_opts.sign {
        return ExitCode::SUCCESS;
    }
    let signed_tmp = out_path.with_extension("signed.mp4");
    let cache_root = c2pa_opts
        .cache_root
        .clone()
        .unwrap_or_else(|| root_dir.join(".wavelet-cache"));
    let cache_opt = if cache_root.exists() {
        Some(cache_root.as_path())
    } else {
        None
    };
    let key = match load_signing_key(
        c2pa_opts.signing_cert.as_deref(),
        c2pa_opts.signing_key.as_deref(),
    ) {
        Ok(k) => k,
        Err(code) => return code,
    };
    let auto_title = c2pa_opts.title.clone().unwrap_or_else(|| {
        comp_path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("wavelet export")
            .to_string()
    });
    match wavelet::c2pa_credentials::sign_mp4(
        comp,
        cache_opt,
        Some(auto_title.as_str()),
        c2pa_opts.author.as_deref(),
        out_path,
        &signed_tmp,
        key,
    ) {
        Ok(report) => {
            if let Err(e) = std::fs::rename(&signed_tmp, out_path) {
                eprintln!("c2pa: rename signed mp4 failed: {e}");
                return ExitCode::from(2);
            }
            println!("c2pa: {}", report.summary);
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!(
                "c2pa sign failed (unsigned mp4 left at {}): {e}",
                out_path.display()
            );
            let _ = std::fs::remove_file(&signed_tmp);
            ExitCode::from(2)
        }
    }
}

/// Resolve an aspect-ratio string to pixel dimensions. Accepts both
/// `W:H` (the gen-side vocabulary — `16:9`, `9:16`, `1:1`, `4:5`,
/// `21:9`) and explicit `WxH` (`1920x1080`). Short-edge defaults to
/// 720 pixels for the `W:H` form — matches the `storyboard plan
/// --aspect` heuristic so multi-aspect render and storyboard plan
/// agree on dimensions.
fn parse_aspect_dims(s: &str) -> Option<[u32; 2]> {
    let s = s.trim();
    if let Some((w, h)) = s.split_once('x') {
        return Some([w.trim().parse().ok()?, h.trim().parse().ok()?]);
    }
    let (a, b) = s.split_once(':')?;
    let aw: u32 = a.trim().parse().ok()?;
    let ah: u32 = b.trim().parse().ok()?;
    if aw == 0 || ah == 0 {
        return None;
    }
    let short = 720u32;
    if aw >= ah {
        let scale = short as f32 / ah as f32;
        Some([(aw as f32 * scale).round() as u32, short])
    } else {
        let scale = short as f32 / aw as f32;
        Some([short, (ah as f32 * scale).round() as u32])
    }
}

fn sibling_with_tag(base: &Path, tag: &str) -> PathBuf {
    let stem = base
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("render")
        .to_owned();
    let ext = base
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("mp4")
        .to_owned();
    let mut p = base.to_path_buf();
    p.set_file_name(format!("{stem}.{tag}.{ext}"));
    p
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aspect_dims_landscape() {
        assert_eq!(parse_aspect_dims("16:9"), Some([1280, 720]));
    }

    #[test]
    fn aspect_dims_portrait() {
        assert_eq!(parse_aspect_dims("9:16"), Some([720, 1280]));
    }

    #[test]
    fn aspect_dims_square() {
        assert_eq!(parse_aspect_dims("1:1"), Some([720, 720]));
    }

    #[test]
    fn aspect_dims_explicit_wxh_pass_through() {
        assert_eq!(parse_aspect_dims("1920x1080"), Some([1920, 1080]));
    }

    #[test]
    fn aspect_dims_rejects_zero() {
        assert!(parse_aspect_dims("0:9").is_none());
        assert!(parse_aspect_dims("16:0").is_none());
    }

    #[test]
    fn aspect_dims_rejects_garbage() {
        assert!(parse_aspect_dims("widescreen").is_none());
    }

    #[test]
    fn sibling_with_tag_swaps_filename() {
        let p = PathBuf::from("/tmp/spot.mp4");
        assert_eq!(sibling_with_tag(&p, "16x9"), PathBuf::from("/tmp/spot.16x9.mp4"));
        assert_eq!(sibling_with_tag(&p, "9x16"), PathBuf::from("/tmp/spot.9x16.mp4"));
    }
}
