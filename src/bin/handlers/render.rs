//! `wavelet render` handler — comp.json → MP4 (+ optional C2PA sign).

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
) -> ExitCode {
    let render_opts = RenderOptions { frame_budget_secs };
    let is_html = comp_path
        .extension()
        .and_then(|s| s.to_str())
        .map(|s| s.eq_ignore_ascii_case("html") || s.eq_ignore_ascii_case("htm"))
        .unwrap_or(false);

    let (comp, root_dir) = if is_html {
        match load_index_html(&comp_path) {
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
        }
    } else {
        match Composition::from_json_path(&comp_path) {
            Ok(pair) => pair,
            Err(e) => {
                eprintln!("error loading {}: {e}", comp_path.display());
                return ExitCode::from(2);
            }
        }
    };

    if aspects.is_empty() {
        let out_path = out.unwrap_or_else(|| comp_path.with_extension("mp4"));
        return render_one(&comp, &root_dir, &comp_path, &out_path, &c2pa_opts, &render_opts);
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
