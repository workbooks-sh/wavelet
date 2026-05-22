//! Composite tool — concat / overlay multiple source clips via
//! `ffmpeg`. Deferred for v1 — only stubbed so the dispatch enum is
//! exhaustive.

use std::path::{Path, PathBuf};

use crate::edit::EditError;

/// Stub. Returns an "unimplemented" error pointing at the deferral.
pub fn run_composite(_sources: &[(PathBuf, f32, f32)], _out: &Path) -> Result<PathBuf, EditError> {
    Err(EditError::Transport(
        "Composite/Splice not implemented in v1 — tracked at wb-ft0o follow-up.".into(),
    ))
}
