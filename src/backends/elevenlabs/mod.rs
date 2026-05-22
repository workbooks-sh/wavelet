//! ElevenLabs adapters — one vendor, many cluster trait impls.
//!
//! Per the `wavelet-backend-consolidation` memory, ElevenLabs is the
//! primary for every audio/voice cluster:
//! - `VoiceIdTts` — `tts` submodule (this commit)
//! - `StructuredMusicGen` — `music` submodule (planned)
//! - `SfxGen` — `sfx` submodule (planned)
//! - `VoiceConvert` — `voice_convert` submodule (planned)
//! - `Transcribe` — `transcribe` submodule (planned)
//!
//! All submodules share `ElevenLabsClient` for auth + HTTP + error
//! mapping. The client is constructed once from `ELEVENLABS_API_KEY`
//! and cloned cheaply into each adapter.

use crate::backends::BackendError;
use crate::backends::cache::AssetCache;
use std::path::PathBuf;

pub mod music;
pub mod tts;

pub use music::ElevenLabsMusicAdapter;
pub use tts::ElevenLabsTtsAdapter;

/// Env var the API key is read from.
pub const KEY_ENV: &str = "ELEVENLABS_API_KEY";

/// Base URL for the ElevenLabs v1 API.
pub const API_BASE: &str = "https://api.elevenlabs.io/v1";

/// Provider id used in cache keys + manifests across every ElevenLabs
/// cluster impl.
pub const PROVIDER: &str = "elevenlabs";

/// Shared client carrying credentials + the on-disk cache. Constructed
/// once and cloned into each cluster trait impl.
#[derive(Debug, Clone)]
pub struct ElevenLabsClient {
    api_key: String,
    cache: AssetCache,
}

impl ElevenLabsClient {
    /// Build from an explicit key + cache root. Useful for tests.
    pub fn with_key(api_key: impl Into<String>, cache_root: impl Into<PathBuf>) -> Self {
        Self {
            api_key: api_key.into(),
            cache: AssetCache::new(cache_root),
        }
    }

    /// Build from env. Returns `MissingCredential` when `ELEVENLABS_API_KEY`
    /// is unset or empty.
    pub fn from_env(cache_root: impl Into<PathBuf>) -> Result<Self, BackendError> {
        let key = std::env::var(KEY_ENV)
            .map_err(|_| BackendError::MissingCredential(KEY_ENV.into()))?;
        if key.trim().is_empty() {
            return Err(BackendError::MissingCredential(KEY_ENV.into()));
        }
        Ok(Self::with_key(key, cache_root))
    }

    /// Access the underlying API key (for adapter use only — don't
    /// log or print).
    pub(crate) fn api_key(&self) -> &str {
        &self.api_key
    }

    /// Access the shared cache.
    pub fn cache(&self) -> &AssetCache {
        &self.cache
    }
}

/// Truncate a response body for inclusion in an error message. Keeps
/// the first 512 chars + a trailing ellipsis.
pub(crate) fn truncate_body(body: &str) -> String {
    if body.len() <= 512 {
        body.to_string()
    } else {
        format!("{}…", &body[..512])
    }
}

/// Well-known ElevenLabs default voice ids. The agent can override via
/// `--voice <id>`; these are listed so the CLI's default is usable
/// without a separate voice-catalog call.
pub mod voices {
    /// Rachel — friendly American female (the documentation default).
    pub const RACHEL: &str = "21m00Tcm4TlvDq8ikWAM";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_env_var_is_explicit() {
        std::env::remove_var(KEY_ENV);
        let tmp = std::env::temp_dir().join("wavelet-elevenlabs-no-env");
        let err = ElevenLabsClient::from_env(&tmp).unwrap_err();
        match err {
            BackendError::MissingCredential(name) => assert_eq!(name, KEY_ENV),
            other => panic!("expected MissingCredential, got {other:?}"),
        }
    }

    #[test]
    fn explicit_key_does_not_read_env() {
        let tmp = std::env::temp_dir().join("wavelet-elevenlabs-explicit");
        let c = ElevenLabsClient::with_key("test-key", &tmp);
        assert_eq!(c.api_key(), "test-key");
    }

    #[test]
    fn well_known_voice_ids_are_documented() {
        // Lock the constants so accidental renames are caught.
        assert_eq!(voices::RACHEL.len(), 20);
        assert!(voices::RACHEL.chars().all(|c| c.is_ascii_alphanumeric()));
    }

    #[test]
    fn truncate_body_handles_short_and_long() {
        assert_eq!(truncate_body("short"), "short");
        let long: String = "x".repeat(1000);
        let truncated = truncate_body(&long);
        assert!(truncated.ends_with('…'));
        assert!(truncated.chars().count() <= 513); // 512 + ellipsis
    }
}
