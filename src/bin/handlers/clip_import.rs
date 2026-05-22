//! `wavelet clip import` — backfill clip-refs from a legacy workdir
//! (wb-n33n.7).
//!
//! Walks `<workdir>/<cache>/<provider>/*.manifest.json` and synthesizes
//! one clip-ref per manifest. Idempotent: re-running on an already-
//! imported workdir doesn't duplicate refs (the underlying
//! `AssetCache::store_with_clip` is idempotent on `asset_hash`).
//!
//! Also expands a legacy `screenplay.json` into per-scene
//! `refs/screenplay-scene/*.clip.html` by re-running the wb-n33n.5
//! split pipeline against the original `.fountain` source when one is
//! present alongside.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use chrono::{DateTime, Utc};
use wavelet::backends::cache::{AssetCache, Manifest};
use wavelet::backends::clipref_emit::ClipEmitContext;
use wavelet::clipref::{walk_refs, ClipKind};
use wavelet::clipref::screenplay::{emit_scenes, split_scenes, EmitOptions};

pub fn run(workdir: Option<PathBuf>, cache_dir: PathBuf, dry_run: bool) -> ExitCode {
    let workdir = workdir.unwrap_or_else(|| PathBuf::from("."));
    let cache_root = if cache_dir.is_absolute() {
        cache_dir.clone()
    } else {
        workdir.join(&cache_dir)
    };

    let mut warnings: Vec<String> = Vec::new();
    let mut written = 0u32;
    let mut skipped = 0u32;

    if cache_root.exists() {
        let existing = walk_refs(&workdir).unwrap_or_default();
        let already: std::collections::HashSet<String> = existing
            .iter()
            .map(|w| w.clip.asset_hash.clone())
            .collect();
        let cache = AssetCache::new(&cache_root);
        for manifest in collect_manifests(&cache_root, &mut warnings) {
            if already.contains(&manifest.request_hash) {
                skipped += 1;
                continue;
            }
            let ctx = build_context(&workdir, &manifest, &mut warnings);
            if dry_run {
                eprintln!(
                    "would import {} {} ({})",
                    manifest.provider,
                    manifest.request_hash,
                    ctx.kind.as_kebab()
                );
                written += 1;
                continue;
            }
            match cache.store_with_clip(&manifest, ctx) {
                Ok(res) => {
                    eprintln!("wrote {}", res.clipref_path.display());
                    written += 1;
                }
                Err(e) => warnings.push(format!(
                    "import {} {}: {e}",
                    manifest.provider, manifest.request_hash
                )),
            }
        }
    } else {
        warnings.push(format!(
            "cache dir {} missing — nothing to import from cache",
            cache_root.display()
        ));
    }

    let fountain = find_fountain(&workdir);
    let screenplay_json = workdir.join("screenplay.json");
    if screenplay_json.exists() {
        if let Some(fountain_path) = fountain {
            match std::fs::read_to_string(&fountain_path) {
                Ok(src) => {
                    let chunks = split_scenes(&src);
                    let fountain_asset = PathBuf::from("../../").join(
                        fountain_path
                            .file_name()
                            .map(PathBuf::from)
                            .unwrap_or_else(|| PathBuf::from("script.fountain")),
                    );
                    let opts = EmitOptions {
                        workdir: &workdir,
                        fountain_asset: &fountain_asset,
                        parent: None,
                    };
                    if dry_run {
                        eprintln!(
                            "would expand screenplay.json → {} scenes",
                            chunks.iter().filter(|c| c.index > 0).count()
                        );
                    } else {
                        match emit_scenes(&chunks, &opts) {
                            Ok(emissions) => {
                                for e in &emissions {
                                    eprintln!("wrote {}", e.path.display());
                                }
                                written += emissions.len() as u32;
                            }
                            Err(e) => {
                                warnings.push(format!("expand screenplay: {e}"));
                            }
                        }
                    }
                }
                Err(e) => warnings
                    .push(format!("read {}: {e}", fountain_path.display())),
            }
        } else {
            warnings.push(
                "screenplay.json present but no sibling .fountain — skipping expansion"
                    .into(),
            );
        }
    }

    eprintln!(
        "{}: {} clip-ref(s) {}, {} skipped (already present), {} warning(s)",
        if dry_run { "dry-run" } else { "import" },
        written,
        if dry_run { "planned" } else { "written" },
        skipped,
        warnings.len()
    );
    for w in &warnings {
        eprintln!("warning: {w}");
    }
    ExitCode::SUCCESS
}

fn collect_manifests(cache_root: &Path, warnings: &mut Vec<String>) -> Vec<Manifest> {
    let mut out = Vec::new();
    let provider_dirs = match std::fs::read_dir(cache_root) {
        Ok(it) => it,
        Err(e) => {
            warnings.push(format!("read {}: {e}", cache_root.display()));
            return out;
        }
    };
    for entry in provider_dirs.flatten() {
        let ftype = match entry.file_type() {
            Ok(t) => t,
            Err(_) => continue,
        };
        if !ftype.is_dir() {
            continue;
        }
        let provider_dir = entry.path();
        let files = match std::fs::read_dir(&provider_dir) {
            Ok(it) => it,
            Err(e) => {
                warnings.push(format!("read {}: {e}", provider_dir.display()));
                continue;
            }
        };
        for f in files.flatten() {
            let p = f.path();
            if !p
                .file_name()
                .and_then(|s| s.to_str())
                .map(|s| s.ends_with(".manifest.json"))
                .unwrap_or(false)
            {
                continue;
            }
            let raw = match std::fs::read_to_string(&p) {
                Ok(s) => s,
                Err(e) => {
                    warnings.push(format!("read {}: {e}", p.display()));
                    continue;
                }
            };
            match serde_json::from_str::<Manifest>(&raw) {
                Ok(m) => out.push(m),
                Err(e) => warnings.push(format!("parse {}: {e}", p.display())),
            }
        }
    }
    out
}

fn build_context<'a>(
    workdir: &'a Path,
    manifest: &Manifest,
    warnings: &mut Vec<String>,
) -> ClipEmitContext<'a> {
    let kind = kind_from_cluster(&manifest.cluster).unwrap_or_else(|| {
        warnings.push(format!(
            "unknown cluster `{}` on {} → defaulting to Still",
            manifest.cluster, manifest.request_hash
        ));
        ClipKind::Still
    });
    let prompt = extract_prompt(&manifest.request).unwrap_or_else(|| {
        warnings.push(format!(
            "no prompt extractable from {} {} — using placeholder",
            manifest.provider, manifest.request_hash
        ));
        format!("<unknown — imported from cache manifest {}>",
            manifest.request_hash)
    });
    let model = manifest
        .request
        .get("model")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let _ = parse_created_at(&manifest.created_at);

    ClipEmitContext {
        workdir,
        kind,
        prompt,
        model,
        parent: None,
        edit_kind: None,
        edit_prompt: None,
        scene: None,
        tags: vec!["imported".into()],
    }
}

fn kind_from_cluster(cluster: &str) -> Option<ClipKind> {
    let c = cluster.to_ascii_lowercase();
    let c = c.as_str();
    if c.contains("i2v") || c.contains("t2v") || c.contains("img2vid") || c.contains("video") {
        Some(ClipKind::Shot)
    } else if c.contains("image") || c.contains("t2i") || c.contains("still") {
        Some(ClipKind::Still)
    } else if c.contains("music") {
        Some(ClipKind::Music)
    } else if c.contains("tts") || c.contains("voice") || c.contains("dialogue") {
        Some(ClipKind::Tts)
    } else if c.contains("caption") || c.contains("transcribe") {
        Some(ClipKind::Caption)
    } else {
        None
    }
}

fn extract_prompt(request: &serde_json::Value) -> Option<String> {
    for key in ["prompt", "text", "query", "input", "description"] {
        if let Some(v) = request.get(key).and_then(|v| v.as_str()) {
            if !v.is_empty() {
                return Some(v.to_string());
            }
        }
    }
    None
}

fn parse_created_at(s: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(s).ok().map(|d| d.with_timezone(&Utc))
}

fn find_fountain(workdir: &Path) -> Option<PathBuf> {
    let entries = std::fs::read_dir(workdir).ok()?;
    for e in entries.flatten() {
        let p = e.path();
        if p.extension().and_then(|s| s.to_str()) == Some("fountain") {
            return Some(p);
        }
    }
    None
}
