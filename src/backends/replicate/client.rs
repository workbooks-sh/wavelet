//! Replicate (api.replicate.com) HTTP client.
//!
//! Auth: `Authorization: Token <token>` read from `REPLICATE_API_TOKEN`.
//! Base: `https://api.replicate.com/v1`.
//!
//! Lifecycle:
//! - `POST /predictions {version, input}` → `{id, status:'starting', urls:{get}}`
//! - `GET /predictions/<id>` (or `urls.get`) until `status` is `succeeded`
//!   or `failed`. On succeeded, `output` carries the result (typically a
//!   URL or a list of URLs for video models).
//! - Asset URLs are short-lived signed `replicate.delivery` paths — fetch
//!   immediately and cache locally.

use std::path::PathBuf;
use std::time::Duration;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::backends::cache::AssetCache;
use crate::backends::BackendError;

/// Env var the Replicate API token is read from.
pub const REPLICATE_TOKEN_ENV: &str = "REPLICATE_API_TOKEN";

/// API root.
pub const BASE_URL: &str = "https://api.replicate.com/v1";

/// Default max wall-time per prediction. Video models routinely take
/// 60–120s; we give them 6 minutes before failing.
pub const DEFAULT_POLL_TIMEOUT_SECS: u64 = 360;

/// Interval between successive polls.
pub const POLL_INTERVAL_SECS: u64 = 4;

/// Replicate client. Cheaply cloneable.
#[derive(Debug, Clone)]
pub struct ReplicateClient {
    token: String,
    cache: AssetCache,
}

impl ReplicateClient {
    /// Build with an explicit token + cache root.
    pub fn with_token(token: impl Into<String>, cache_root: impl Into<PathBuf>) -> Self {
        Self {
            token: token.into(),
            cache: AssetCache::new(cache_root),
        }
    }

    /// Read `REPLICATE_API_TOKEN` from the environment.
    pub fn from_env(cache_root: impl Into<PathBuf>) -> Result<Self, BackendError> {
        let token = std::env::var(REPLICATE_TOKEN_ENV)
            .map_err(|_| BackendError::MissingCredential(REPLICATE_TOKEN_ENV.into()))?;
        if token.trim().is_empty() {
            return Err(BackendError::MissingCredential(REPLICATE_TOKEN_ENV.into()));
        }
        Ok(Self::with_token(token, cache_root))
    }

    /// Shared asset cache.
    pub fn cache(&self) -> &AssetCache {
        &self.cache
    }

    /// Submit a prediction and poll until done. `R` is the model-specific
    /// output type (string, vec of strings, struct — Replicate is
    /// permissive). Returns the parsed prediction record on success.
    pub(crate) fn run_prediction<I, O>(
        &self,
        version: &str,
        input: &I,
    ) -> Result<Prediction<O>, BackendError>
    where
        I: Serialize,
        O: DeserializeOwned,
    {
        let body = StartBody { version, input };
        let json_body = serde_json::to_string(&body)
            .map_err(|e| BackendError::InvalidRequest(format!("serialize body: {e}")))?;
        let url = format!("{BASE_URL}/predictions");
        let resp = ureq::post(&url)
            .set("Authorization", &format!("Token {}", self.token))
            .set("Content-Type", "application/json")
            .set("Accept", "application/json")
            .set("Prefer", "wait=1")
            .send_string(&json_body);
        let started: Prediction<O> = send_decode(resp)?;
        if matches!(started.status.as_deref(), Some("succeeded") | Some("failed") | Some("canceled")) {
            return Ok(started);
        }
        self.poll(&started.id)
    }

    fn poll<O>(&self, id: &str) -> Result<Prediction<O>, BackendError>
    where
        O: DeserializeOwned,
    {
        let url = format!("{BASE_URL}/predictions/{id}");
        let deadline = std::time::Instant::now()
            + Duration::from_secs(DEFAULT_POLL_TIMEOUT_SECS);
        loop {
            let resp = ureq::get(&url)
                .set("Authorization", &format!("Token {}", self.token))
                .set("Accept", "application/json")
                .call();
            let p: Prediction<O> = send_decode(resp)?;
            match p.status.as_deref() {
                Some("succeeded") | Some("failed") | Some("canceled") => return Ok(p),
                _ => {}
            }
            if std::time::Instant::now() >= deadline {
                return Err(BackendError::Transport(format!(
                    "prediction {id} did not complete within {DEFAULT_POLL_TIMEOUT_SECS}s"
                )));
            }
            std::thread::sleep(Duration::from_secs(POLL_INTERVAL_SECS));
        }
    }

    /// Fetch arbitrary bytes (typically the model output URL).
    pub(crate) fn fetch_asset(&self, url: &str) -> Result<Vec<u8>, BackendError> {
        let resp = ureq::get(url)
            .call()
            .map_err(|e| BackendError::Transport(format!("fetch asset: {e}")))?;
        let mut buf = Vec::new();
        resp.into_reader()
            .read_to_end(&mut buf)
            .map_err(|e| BackendError::Transport(format!("read asset body: {e}")))?;
        Ok(buf)
    }
}

#[derive(Debug, Serialize)]
struct StartBody<'a, I: Serialize> {
    version: &'a str,
    input: &'a I,
}

/// A Replicate prediction record. The shape every model returns.
#[derive(Debug, Deserialize)]
pub(crate) struct Prediction<O> {
    /// Prediction id (`f0mhhafxyhrmr0cy855s83v6qw`-style).
    pub id: String,
    /// Lifecycle status: `starting`, `processing`, `succeeded`, `failed`, `canceled`.
    #[serde(default)]
    pub status: Option<String>,
    /// Model output. Shape varies per model; caller picks `O` to match.
    /// `None` while the prediction is still running or on failure.
    #[serde(default = "default_none")]
    pub output: Option<O>,
    /// Error message when `status == "failed"`.
    #[serde(default)]
    pub error: Option<String>,
}

fn default_none<O>() -> Option<O> {
    None
}

fn send_decode<R: DeserializeOwned>(
    resp: Result<ureq::Response, ureq::Error>,
) -> Result<R, BackendError> {
    match resp {
        Ok(r) => r
            .into_json()
            .map_err(|e| BackendError::Decode(format!("decode response: {e}"))),
        Err(ureq::Error::Status(code, r)) => {
            let body = r.into_string().unwrap_or_default();
            Err(BackendError::Transport(format!("HTTP {code}: {body}")))
        }
        Err(e) => Err(BackendError::Transport(e.to_string())),
    }
}

use std::io::Read;
