//! Veo regeneration tool — re-roll the entire shot with a new prompt.
//!
//! Thin wrapper around the existing `GoogleVeoAdapter` so the edit
//! loop doesn't duplicate its long-running-op handling.

use std::path::{Path, PathBuf};

use crate::edit::EditError;

/// Run a single Veo 3.1 Fast text-to-video request and write the
/// resulting MP4 to `out_path`.
pub fn run_veo_regen(
    prompt: &str,
    duration_secs: f32,
    aspect: &str,
    max_cost_usd: f32,
    cache_root: &Path,
    out_path: &Path,
) -> Result<PathBuf, EditError> {
    use crate::backends::google::{GoogleAiClient, GoogleVeoAdapter, VeoModel};
    use crate::backends::video::{Txt2VidGenBackend, Txt2VidRequest};
    use crate::backends::RunMode;

    let client = GoogleAiClient::from_env(cache_root.to_path_buf())
        .map_err(|_| EditError::NoKey)?;
    let adapter = GoogleVeoAdapter::new(client, VeoModel::Fast);
    let req = Txt2VidRequest {
        prompt: prompt.to_string(),
        negative_prompt: None,
        apply_default_negatives: true,
        duration_secs,
        aspect_ratio: aspect.to_string(),
        seed: None,
    };
    let mode = RunMode::Live { max_cost_usd };
    let outcome = adapter
        .generate(&req, mode)
        .map_err(|e| EditError::Transport(format!("veo regen: {e}")))?;
    // Copy the cached MP4 to the requested out_path.
    std::fs::copy(&outcome.response.video_path, out_path)
        .map_err(|e| EditError::Transport(format!("copy veo output: {e}")))?;
    Ok(out_path.to_path_buf())
}
