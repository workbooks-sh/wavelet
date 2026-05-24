//! `wavelet verify` handler — structural lint of an HTML manifest.
//!
//! Routes by extension. The legacy comp.json verification path has been
//! retired alongside the JSON render input. For MP4 inspection, the
//! agent should use `wavelet lint <html> --mp4 <mp4>` (post-render
//! contrast + safe-zone + glyph-clip against the actual composited
//! frames) or system `ffprobe` for raw stream metadata.

use std::path::PathBuf;
use std::process::ExitCode;

use wavelet::compose::load_index_html;
use wavelet::verify::{verify, Level};

/// Run verify on an HTML manifest. Non-HTML inputs are rejected with
/// a fix-hint pointing at the right tool, so the agent doesn't waste
/// cycles re-running with the wrong file type.
pub fn run(comp_path: PathBuf, deep: bool) -> ExitCode {
    let ext = comp_path
        .extension()
        .and_then(|s| s.to_str())
        .map(|s| s.to_ascii_lowercase())
        .unwrap_or_default();

    if ext == "mp4" || ext == "mov" || ext == "webm" {
        eprintln!(
            "wavelet verify: REJECTED — `verify` is for HTML manifest \
             structural checks, not MP4 inspection.\n\
             \n\
             Your input: {}\n\
             \n\
             For MP4 quality checks (contrast, safe-zone, glyph-clip \
             against the rendered composite), use:\n\
                 wavelet lint <manifest>.html --mp4 {} --platform <p>\n\
             \n\
             For raw MP4 stream metadata (codec, duration, resolution):\n\
                 ffprobe -v quiet -print_format json -show_format -show_streams {}\n\
             \n\
             Do NOT retry `verify <foo.mp4>` — this command will not \
             change behaviour on retry.",
            comp_path.display(),
            comp_path.display(),
            comp_path.display(),
        );
        return ExitCode::from(3);
    }

    if ext != "html" && ext != "htm" {
        eprintln!(
            "wavelet verify: REJECTED — only HTML manifest inputs are \
             accepted (`.html` / `.htm`).\n\
             \n\
             Your input: {}\n\
             \n\
             Pass the deliverable's manifest HTML file (e.g. \
             `commercial.html`, `trailer.html`, `promo.html`). The \
             legacy comp.json verification path has been retired.",
            comp_path.display(),
        );
        return ExitCode::from(3);
    }

    let (comp, root_dir) = match load_index_html(&comp_path) {
        Ok(c) => {
            let dir = comp_path
                .parent()
                .map(|p| p.to_path_buf())
                .unwrap_or_else(|| std::path::Path::new(".").to_path_buf());
            (c, dir)
        }
        Err(e) => {
            eprintln!("error loading {}: {e}", comp_path.display());
            return ExitCode::from(3);
        }
    };
    let findings = verify(&comp, &root_dir, deep);
    let mut errors = 0usize;
    let mut warnings = 0usize;
    for f in &findings {
        let prefix = match f.level {
            Level::Error => {
                errors += 1;
                "ERROR"
            }
            Level::Warning => {
                warnings += 1;
                "WARN "
            }
        };
        println!("{prefix}  [{}] {}", f.origin, f.message);
    }
    println!();
    if findings.is_empty() {
        println!("✓ clean: 0 errors, 0 warnings");
        ExitCode::SUCCESS
    } else if errors == 0 {
        println!("◐ {} warning(s), 0 errors", warnings);
        ExitCode::SUCCESS
    } else {
        println!("✗ {} error(s), {} warning(s)", errors, warnings);
        ExitCode::from(1)
    }
}
