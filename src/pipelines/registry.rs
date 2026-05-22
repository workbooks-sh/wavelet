//! Pipeline discovery — enumerate `*.yaml` files under a search dir.

use std::path::{Path, PathBuf};

use super::loader::{load_from_path, LoadError};
use super::schema::Pipeline;

/// One entry in the discovered pipeline list.
#[derive(Debug)]
pub struct PipelineEntry {
    /// Path to the YAML file.
    pub path: PathBuf,
    /// Either the loaded pipeline (if parse + validate succeeded) or
    /// the load error (so `pipelines list` can show broken files
    /// instead of silently dropping them).
    pub result: Result<Pipeline, LoadError>,
}

/// Discover every `*.yaml` and `*.yml` file directly inside `dir`,
/// returning one [`PipelineEntry`] per file. Sub-directories are not
/// recursed — pipelines live as a flat list.
pub fn discover(dir: &Path) -> Vec<PipelineEntry> {
    let mut entries = Vec::new();
    let read_dir = match std::fs::read_dir(dir) {
        Ok(rd) => rd,
        Err(_) => return entries,
    };
    let mut paths: Vec<PathBuf> = read_dir
        .filter_map(|r| r.ok())
        .map(|e| e.path())
        .filter(|p| {
            p.is_file()
                && p.extension()
                    .and_then(|s| s.to_str())
                    .map(|s| s == "yaml" || s == "yml")
                    .unwrap_or(false)
        })
        .collect();
    paths.sort();
    for path in paths {
        let result = load_from_path(&path);
        entries.push(PipelineEntry { path, result });
    }
    entries
}

/// Default search directory: `packages/wavelet/pipeline_defs/` relative
/// to the wavelet crate root. Resolved at runtime from the binary's
/// `CARGO_MANIFEST_DIR` baked at build time.
pub fn default_search_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("pipeline_defs")
}
