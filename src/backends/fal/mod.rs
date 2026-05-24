//! Fal AI adapters — one client, image/video utility clusters.
//!
//! Fal is the host for utility-tier image clusters (background removal,
//! segmentation, OCR, Flux Schnell text-to-image, Flux Kontext Max instruction
//! edit, SUPIR upscale, CLIP similarity, pairwise compare, vision verify) plus
//! Whisper-words, Kokoro TTS, Veo 3 Standard + Fast text-to-video
//! (`fal/veo.rs`), and Veo 3.1 reference-to-video (`fal/veo_ref.rs`).
//! Hero text-to-video can now route through either Fal Veo (`--backend
//! fal-veo3`) or Google AI Studio directly. Character-consistent clips use
//! `--backend fal-veo3-ref` with one or more `--reference` images;
//! reference-conditioned stills route through Google Nano-Banana 3.
//!
//! All submodules share `FalClient` for auth + HTTP + error mapping.
//! `FalClient` is itself a thin newtype over the generic
//! [`HttpBackendClient`] — the auth/transport/cache plumbing is shared
//! with every other HTTP-based backend (Roboflow, future Modal, etc).
//!
//! ## Auth
//!
//! Fal uses `Authorization: Key <id>:<secret>` with the full key string
//! (`<id>:<secret>`) read from the `FAL_KEY` environment variable.
//!
//! ## Sync vs queue
//!
//! Fal exposes both a sync endpoint (`https://fal.run/<model>`) that
//! blocks until the gen completes, and a queue endpoint
//! (`https://queue.fal.run/<model>`) that returns a request id you poll.
//! This client uses the sync endpoint — simpler, and short-form audio
//! gens (≤30s) typically complete inside the sync window.

use crate::backends::BackendError;
use crate::backends::cache::AssetCache;
use crate::backends::http_client::{AuthScheme, FAL_KEY_ENV, HttpBackendClient};
use serde::de::DeserializeOwned;
use std::path::PathBuf;

pub mod birefnet;
pub mod clip_similarity;
pub mod evf_sam;
pub mod flux_schnell;
pub mod kokoro;
pub mod kontext_max;
pub mod pairwise_compare;
pub mod supir;
pub mod veo;
pub mod veo_ref;
pub mod vision_verify;
pub mod whisper_words;

pub use birefnet::FalBirefnetAdapter;
pub use clip_similarity::FalClipSimilarityAdapter;
pub use evf_sam::FalEvfSamAdapter;
pub use flux_schnell::FalFluxSchnellAdapter;
pub use kokoro::FalKokoroAdapter;
pub use kontext_max::FalKontextMaxAdapter;
pub use pairwise_compare::FalPairwiseCompareAdapter;
pub use supir::FalSupirAdapter;
pub use veo::{FalVeoAdapter, FalVeoModel};
pub use veo_ref::{FalVeoRefAdapter, FalVeoRefModel};
pub use vision_verify::FalVisionVerifyAdapter;
pub use whisper_words::FalWhisperWordsAdapter;

/// Env var the API key is read from. Re-exported for legacy callers;
/// the canonical constant now lives on [`http_client`].
pub const KEY_ENV: &str = FAL_KEY_ENV;

/// Sync endpoint base. Each gen completes inside the HTTP response.
pub const SYNC_BASE: &str = "https://fal.run";

/// Provider id prefix used in cache keys + manifests. Per-model
/// adapters extend this with their model path (e.g. `fal-flux-schnell`).
pub const PROVIDER_PREFIX: &str = "fal";

/// Shared Fal client. Thin newtype over [`HttpBackendClient`] so every
/// existing adapter keeps working unchanged.
#[derive(Debug, Clone)]
pub struct FalClient(HttpBackendClient);

impl FalClient {
    /// Build from an explicit key + cache root. Useful for tests.
    pub fn with_key(api_key: impl Into<String>, cache_root: impl Into<PathBuf>) -> Self {
        Self(HttpBackendClient::new(
            SYNC_BASE,
            AuthScheme::FalKey(api_key.into()),
            cache_root,
        ))
    }

    /// Build from `FAL_KEY`. Returns `MissingCredential` when unset or
    /// empty.
    pub fn from_env(cache_root: impl Into<PathBuf>) -> Result<Self, BackendError> {
        HttpBackendClient::fal_from_env(cache_root).map(Self)
    }

    /// Shared cache.
    pub fn cache(&self) -> &AssetCache {
        self.0.cache()
    }

    /// POST to the sync endpoint for `model_path` (e.g. `"fal-ai/flux-schnell"`).
    pub(crate) fn post_sync<B, R>(&self, model_path: &str, body: &B) -> Result<R, BackendError>
    where
        B: serde::Serialize,
        R: DeserializeOwned,
    {
        self.0.post_sync(model_path, body)
    }

    /// Fetch a binary asset from a URL Fal returned in its response.
    pub(crate) fn fetch_asset(&self, url: &str) -> Result<Vec<u8>, BackendError> {
        self.0.fetch_asset(url)
    }

    /// Upload local bytes to fal-storage (the CDN that hosts user
    /// inputs for endpoints that don't accept `data:` URLs). Returns
    /// the public `file_url` you pass to the model.
    ///
    /// Used by the Whisper-words adapter — Whisper rejects `data:`
    /// audio URLs with `422 Unsupported data URL`, so we initiate an
    /// upload, PUT the bytes, and pass the returned URL.
    pub(crate) fn upload_bytes(
        &self,
        bytes: &[u8],
        content_type: &str,
        file_name: &str,
    ) -> Result<String, BackendError> {
        self.0.fal_upload_bytes(bytes, content_type, file_name)
    }
}

/// Roboflow client — thin newtype over [`HttpBackendClient`] using the
/// query-parameter auth scheme. Used by upcoming object-detection
/// adapters (epic wb-f2ul, issues B1/B2).
#[derive(Debug, Clone)]
pub struct RoboflowClient(HttpBackendClient);

impl RoboflowClient {
    /// Build from an explicit key + cache root.
    pub fn with_key(api_key: impl Into<String>, cache_root: impl Into<PathBuf>) -> Self {
        Self(HttpBackendClient::new(
            "https://infer.roboflow.com",
            AuthScheme::QueryParam {
                name: "api_key".into(),
                value: api_key.into(),
            },
            cache_root,
        ))
    }

    /// Build from `ROBOFLOW_API_KEY`.
    pub fn from_env(cache_root: impl Into<PathBuf>) -> Result<Self, BackendError> {
        HttpBackendClient::roboflow_from_env(cache_root).map(Self)
    }

    /// Shared cache.
    pub fn cache(&self) -> &AssetCache {
        self.0.cache()
    }

    /// POST to a model endpoint.
    #[allow(dead_code)]
    pub(crate) fn post_sync<B, R>(&self, path: &str, body: &B) -> Result<R, BackendError>
    where
        B: serde::Serialize,
        R: DeserializeOwned,
    {
        self.0.post_sync(path, body)
    }

    /// Fetch a binary asset by URL.
    #[allow(dead_code)]
    pub(crate) fn fetch_asset(&self, url: &str) -> Result<Vec<u8>, BackendError> {
        self.0.fetch_asset(url)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_env_var_is_explicit() {
        unsafe { std::env::remove_var(KEY_ENV) };
        let tmp = std::env::temp_dir().join("wavelet-fal-no-env");
        let err = FalClient::from_env(&tmp).unwrap_err();
        match err {
            BackendError::MissingCredential(name) => assert_eq!(name, KEY_ENV),
            other => panic!("expected MissingCredential, got {other:?}"),
        }
    }

    #[test]
    fn explicit_key_builds_a_client() {
        let tmp = std::env::temp_dir().join("wavelet-fal-explicit");
        let _c = FalClient::with_key("id:secret", &tmp);
    }

    #[test]
    fn roboflow_with_key_builds_a_client() {
        let tmp = std::env::temp_dir().join("wavelet-rf-explicit");
        let _c = RoboflowClient::with_key("rf-key", &tmp);
    }
}
