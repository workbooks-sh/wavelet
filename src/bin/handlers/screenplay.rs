//! `wavelet screenplay parse` + `reassemble` handlers (wb-n33n.5).
//!
//! Default: per-scene `.clip.html` files of kind=screenplay-scene under
//! `<workdir>/refs/screenplay-scene/`. `--legacy-json` falls back to the
//! original single-blob `screenplay.json` emit.

use std::path::PathBuf;
use std::process::ExitCode;

use wavelet::clipref::screenplay::{
    emit_scenes, load_scenes, reassemble as reassemble_chunks, split_scenes, EmitOptions,
};

/// Parse a Fountain screenplay.
pub fn run(
    path: PathBuf,
    workdir: Option<PathBuf>,
    legacy_json: bool,
    pretty: bool,
    out: Option<PathBuf>,
) -> ExitCode {
    let src = match std::fs::read_to_string(&path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("read {}: {e}", path.display());
            return ExitCode::from(2);
        }
    };

    if legacy_json {
        return run_legacy_json(&path, &src, pretty, out);
    }

    let workdir = workdir
        .or_else(|| path.parent().map(|p| p.to_path_buf()))
        .unwrap_or_else(|| PathBuf::from("."));
    let fountain_asset = relative_asset(&workdir, &path);

    let chunks = split_scenes(&src);
    let opts = EmitOptions {
        workdir: &workdir,
        fountain_asset: &fountain_asset,
        parent: None,
    };
    let emissions = match emit_scenes(&chunks, &opts) {
        Ok(e) => e,
        Err(e) => {
            eprintln!("emit: {e}");
            return ExitCode::from(2);
        }
    };
    for e in &emissions {
        eprintln!("wrote {}", e.path.display());
    }
    ExitCode::SUCCESS
}

/// Concatenate per-scene clip-refs back into a fountain.
pub fn reassemble(workdir: PathBuf, out: Option<PathBuf>) -> ExitCode {
    let chunks = match load_scenes(&workdir) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("load {}: {e}", workdir.display());
            return ExitCode::from(2);
        }
    };
    let body = reassemble_chunks(chunks);
    match out {
        Some(p) => {
            if let Err(e) = std::fs::write(&p, &body) {
                eprintln!("write {}: {e}", p.display());
                return ExitCode::from(2);
            }
            eprintln!("wrote {}", p.display());
            ExitCode::SUCCESS
        }
        None => {
            print!("{body}");
            ExitCode::SUCCESS
        }
    }
}

fn run_legacy_json(
    path: &std::path::Path,
    src: &str,
    pretty: bool,
    out: Option<PathBuf>,
) -> ExitCode {
    let screenplay = match fountain::parse(src) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("parse {}: {e}", path.display());
            return ExitCode::from(2);
        }
    };
    let json = if pretty {
        serde_json::to_string_pretty(&screenplay)
    } else {
        serde_json::to_string(&screenplay)
    };
    match (json, out) {
        (Ok(s), Some(p)) => {
            if let Err(e) = std::fs::write(&p, &s) {
                eprintln!("write {}: {e}", p.display());
                return ExitCode::from(2);
            }
            eprintln!("wrote {}", p.display());
            ExitCode::SUCCESS
        }
        (Ok(s), None) => {
            println!("{s}");
            ExitCode::SUCCESS
        }
        (Err(e), _) => {
            eprintln!("serialize: {e}");
            ExitCode::from(2)
        }
    }
}

/// Compute the path the clip-ref's `asset:` field should hold. We want
/// it relative to `<workdir>/refs/screenplay-scene/`. Caller passes the
/// workdir; the clip-ref module writes into the subdirectory itself.
fn relative_asset(workdir: &std::path::Path, fountain: &std::path::Path) -> PathBuf {
    let scene_dir = workdir.join("refs/screenplay-scene");
    let target = std::fs::canonicalize(fountain).unwrap_or_else(|_| fountain.to_path_buf());
    let base = std::fs::canonicalize(&scene_dir).unwrap_or(scene_dir.clone());
    pathdiff(&target, &base)
}

fn pathdiff(target: &std::path::Path, base: &std::path::Path) -> PathBuf {
    use std::path::Component;
    let t: Vec<_> = target.components().collect();
    let b: Vec<_> = base.components().collect();
    let mut i = 0;
    while i < t.len() && i < b.len() && t[i] == b[i] {
        i += 1;
    }
    let mut out = PathBuf::new();
    for _ in i..b.len() {
        out.push("..");
    }
    for c in &t[i..] {
        if let Component::Normal(s) = c {
            out.push(s);
        }
    }
    if out.as_os_str().is_empty() {
        out.push(".");
    }
    out
}
