//! Gemini Omni in-place pixel edit — NOT YET SHIPPED as of 2026-05-19.
//!
//! Probing `v1beta/models` for `gemini-omni*` returns nothing as of
//! the time this verb landed. We return a clear "unavailable" error
//! that includes any probed model id so an operator can swap it in
//! without code changes.

use std::path::Path;

use crate::edit::EditError;

/// Reject with a clear, descriptive error. When `probe_omni_model`
/// finds a matching slug at run-time, the error includes it so the
/// operator can see the exact identifier to wire in.
pub fn unavailable(input_path: &Path, instruction: &str, api_key: &str) -> EditError {
    let live = crate::edit::gemini::probe_omni_model(api_key);
    let detail = match live {
        Some(name) => format!(
            "Gemini Omni model `{name}` was discovered live but the wavelet edit executor doesn't wire it yet. Override via env: `WAVELET_OMNI_MODEL={name}` once the adapter ships."
        ),
        None => "no models matching `gemini-omni*` exist on the live Gemini surface yet (probed 2026-05-19). Re-check periodically.".into(),
    };
    EditError::OmniUnavailable {
        input: input_path.display().to_string(),
        instruction: instruction.into(),
        detail,
    }
}
