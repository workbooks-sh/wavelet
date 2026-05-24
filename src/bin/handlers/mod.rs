//! Per-verb CLI handler modules. Each verb's `run` function lives in
//! its own file. The Cmd / *Op enum definitions stay in `wavelet.rs`
//! because clap's derive macros + the central dispatch belong with
//! the binary's entry point.
//!
//! Submodules access their `*Op` enum via `super::<EnumName>` — child
//! modules see the parent's items without explicit visibility on the
//! enum itself.
//!
//! Adding a verb: define the `Cmd::Foo` variant + `FooOp` enum in
//! `wavelet.rs`, route `Cmd::Foo { op } => handlers::foo::run(op)` in
//! the main dispatch, add `pub mod foo;` here.

use std::path::{Path, PathBuf};

use wavelet::pipelines::{default_search_dir, discover, load_from_path, Pipeline};

pub mod brief;
pub mod c2pa;
pub mod captions;
pub mod character;
pub mod clip;
pub mod clip_import;
pub mod continuity;
pub mod dialogue;
pub mod diff;
pub mod director;
pub mod lipsync;
pub mod music;
pub mod pipelines;
pub mod render;
pub mod screenplay;
pub mod transitions;
pub mod velocity;
pub mod verify;
pub mod workflow;

/// Resolve a pipeline by name (search the registry) or by direct
/// path. Shared by every verb that takes a pipeline reference.
pub(super) fn resolve_pipeline(
    name_or_path: &str,
    dir: Option<&Path>,
) -> Result<(PathBuf, Pipeline), String> {
    let p = Path::new(name_or_path);
    if p.is_file() {
        return load_from_path(p)
            .map(|pl| (p.to_path_buf(), pl))
            .map_err(|e| e.to_string());
    }
    let search = dir
        .map(|d| d.to_path_buf())
        .unwrap_or_else(default_search_dir);
    for entry in discover(&search) {
        if let Ok(pl) = &entry.result {
            if pl.name == name_or_path {
                return Ok((entry.path, pl.clone()));
            }
        }
    }
    Err(format!(
        "no pipeline named `{name_or_path}` under {}",
        search.display()
    ))
}
