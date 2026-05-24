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
use wavelet::screenplay::duration_fit;

/// Extract and display the canonical character registry for a Fountain
/// screenplay. With `--json` or `--pretty` emits structured JSON;
/// otherwise renders a compact table to stdout.
pub fn characters(path: PathBuf, json: bool, pretty: bool) -> ExitCode {
    let src = match std::fs::read_to_string(&path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("read {}: {e}", path.display());
            return ExitCode::from(2);
        }
    };
    let screenplay = match fountain::parse(&src) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("parse {}: {e}", path.display());
            return ExitCode::from(2);
        }
    };
    let registry = fountain::screenplay_characters(&screenplay);

    if json || pretty {
        let out = if pretty {
            serde_json::to_string_pretty(&registry)
        } else {
            serde_json::to_string(&registry)
        };
        match out {
            Ok(s) => println!("{s}"),
            Err(e) => {
                eprintln!("serialize: {e}");
                return ExitCode::from(2);
            }
        }
        return ExitCode::SUCCESS;
    }

    // Pretty table output.
    if registry.is_empty() {
        println!("0 characters found");
        return ExitCode::SUCCESS;
    }

    // Determine column widths.
    let name_w = registry.iter().map(|c| c.canonical.len()).max().unwrap_or(4).max(9);

    for entry in &registry {
        let cue_label = if entry.cue_count == 1 { "cue " } else { "cues" };
        let scenes_str = if entry.scenes.is_empty() {
            "[]".to_string()
        } else {
            format!(
                "[{}]",
                entry
                    .scenes
                    .iter()
                    .map(|s| s.to_string())
                    .collect::<Vec<_>>()
                    .join(",")
            )
        };
        let ext_str = if entry.extensions.is_empty() {
            "[]".to_string()
        } else {
            format!("[{}]", entry.extensions.join(", "))
        };
        println!(
            "{:<name_w$}  {:>3} {}  {:>5} words  {:>12}  {}",
            entry.canonical,
            entry.cue_count,
            cue_label,
            entry.word_count,
            scenes_str,
            ext_str,
            name_w = name_w,
        );
    }

    // Count distinct scenes across all characters.
    let all_scenes: std::collections::BTreeSet<u32> =
        registry.iter().flat_map(|c| c.scenes.iter().copied()).collect();
    let scene_count = all_scenes.len();

    println!(
        "{} character{} across {} scene{}",
        registry.len(),
        if registry.len() == 1 { "" } else { "s" },
        scene_count,
        if scene_count == 1 { "" } else { "s" },
    );

    ExitCode::SUCCESS
}

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

/// Validate copy density against a declared duration. Emits a JSON
/// `Report` to stdout; exits non-zero on `over_budget`.
///
/// **Idempotent on identical content**: hashes (file content + duration)
/// and caches the result under `.wavelet-cache/screenplay-validate/`.
/// A second call within the same workdir on an unchanged file emits
/// the cached report + a stderr note "already validated Ns ago, no
/// changes detected". This short-circuits the 005 v5 pattern where
/// the agent ran `screenplay validate` 3 times in 6 seconds with
/// identical args.
pub fn validate(path: PathBuf, duration: f32, pretty: bool) -> ExitCode {
    let src = match std::fs::read_to_string(&path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("read {}: {e}", path.display());
            return ExitCode::from(2);
        }
    };

    let cache_key = validate_cache_key(&src, duration);
    let cache_path = std::path::Path::new(".wavelet-cache/screenplay-validate")
        .join(format!("{cache_key}.json"));

    if let Some(cached) = read_cache(&cache_path) {
        let age = cache_age_secs(&cache_path);
        eprintln!(
            "wavelet screenplay validate: already validated {:.0}s ago, no changes detected — emitting cached report",
            age
        );
        // Re-pretty-print so the --pretty flag is honored on cache hits too.
        if let Ok(report) = serde_json::from_str::<duration_fit::Report>(&cached) {
            let json = if pretty {
                serde_json::to_string_pretty(&report)
            } else {
                serde_json::to_string(&report)
            };
            if let Ok(s) = json {
                println!("{s}");
            }
            return if report.blocks() {
                ExitCode::from(3)
            } else {
                ExitCode::SUCCESS
            };
        }
        // Cache parse failure → fall through to fresh evaluation.
    }

    let screenplay = match fountain::parse(&src) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("parse {}: {e}", path.display());
            return ExitCode::from(2);
        }
    };
    let report = duration_fit::evaluate(&screenplay, duration);
    let json = if pretty {
        serde_json::to_string_pretty(&report)
    } else {
        serde_json::to_string(&report)
    };
    let canonical = match serde_json::to_string(&report) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("serialize: {e}");
            return ExitCode::from(2);
        }
    };
    write_cache(&cache_path, &canonical);
    match json {
        Ok(s) => println!("{s}"),
        Err(e) => {
            eprintln!("serialize: {e}");
            return ExitCode::from(2);
        }
    }
    if report.blocks() {
        // Exit 3 — same convention as `wavelet lint` for hard fails.
        ExitCode::from(3)
    } else {
        ExitCode::SUCCESS
    }
}

/// Stable cache key over (fountain content + duration). Uses sha1 of
/// the canonical inputs so any whitespace change in the source busts
/// the cache.
fn validate_cache_key(src: &str, duration: f32) -> String {
    use std::hash::{Hash, Hasher};
    // xxHash-equivalent stable hash — siphash via std::hash is fine
    // here; the cache only needs to be content-deterministic within a
    // workdir, not adversarial-collision-resistant.
    let mut h = std::collections::hash_map::DefaultHasher::new();
    src.hash(&mut h);
    duration.to_bits().hash(&mut h);
    format!("{:016x}", h.finish())
}

fn read_cache(path: &std::path::Path) -> Option<String> {
    let s = std::fs::read_to_string(path).ok()?;
    Some(s)
}

fn write_cache(path: &std::path::Path, body: &str) {
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(path, body);
}

fn cache_age_secs(path: &std::path::Path) -> f64 {
    let meta = match std::fs::metadata(path) {
        Ok(m) => m,
        Err(_) => return 0.0,
    };
    let mtime = match meta.modified() {
        Ok(t) => t,
        Err(_) => return 0.0,
    };
    std::time::SystemTime::now()
        .duration_since(mtime)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
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
