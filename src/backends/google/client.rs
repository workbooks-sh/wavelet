//! Google AI Studio (generativelanguage.googleapis.com) HTTP client.
//!
//! Thin wrapper around the existing [`HttpBackendClient`] that knows
//! Google's URL conventions:
//!
//! - Base: `https://generativelanguage.googleapis.com/v1beta`
//! - Auth: `?key=<api-key>` on every request
//! - Sync calls: `POST .../models/<model>:<method>?key=…`
//! - Long-running operations: `POST` returns `{ "name": "models/<model>/operations/<id>" }`;
//!   poll `GET .../<operation-name>?key=…` until `done: true`.

use std::path::PathBuf;
use std::time::Duration;

use serde::de::DeserializeOwned;

use crate::backends::cache::AssetCache;
use crate::backends::http_client::{compose_url, AuthScheme, HttpBackendClient};
use crate::backends::BackendError;

/// Env var the Google AI Studio API key is read from.
pub const GOOGLE_API_KEY_ENV: &str = "GOOGLE_API_KEY";

/// Base URL — the v1beta surface hosts Veo + the rest of Gemini's API.
pub const BASE_URL: &str = "https://generativelanguage.googleapis.com/v1beta";

/// Default max wall-time for a Veo gen poll loop. Veo 3.1 fast usually
/// returns in 30–60s; standard can run 90–120s. We give it 5 minutes
/// before failing — anything longer is a real outage, not slow path.
pub const DEFAULT_POLL_TIMEOUT_SECS: u64 = 300;

/// Interval between successive polls of a long-running operation.
pub const POLL_INTERVAL_SECS: u64 = 5;

/// Google AI Studio client.
#[derive(Debug, Clone)]
pub struct GoogleAiClient {
    inner: HttpBackendClient,
    api_key: String,
}

impl GoogleAiClient {
    /// Build with an explicit key + cache root. Cache root is the same
    /// pattern every backend uses for response-manifests + asset bytes.
    pub fn with_key(api_key: impl Into<String>, cache_root: impl Into<PathBuf>) -> Self {
        let key = api_key.into();
        let inner = HttpBackendClient::new(
            BASE_URL,
            AuthScheme::QueryParam {
                name: "key".into(),
                value: key.clone(),
            },
            cache_root,
        );
        Self { inner, api_key: key }
    }

    /// Read `GOOGLE_API_KEY` from the environment.
    pub fn from_env(cache_root: impl Into<PathBuf>) -> Result<Self, BackendError> {
        let key = std::env::var(GOOGLE_API_KEY_ENV)
            .map_err(|_| BackendError::MissingCredential(GOOGLE_API_KEY_ENV.into()))?;
        if key.trim().is_empty() {
            return Err(BackendError::MissingCredential(GOOGLE_API_KEY_ENV.into()));
        }
        Ok(Self::with_key(key, cache_root))
    }

    /// Shared cache.
    pub fn cache(&self) -> &AssetCache {
        self.inner.cache()
    }

    /// POST a JSON body to `models/<model>:<method>` and decode the
    /// response. Used both for the initial long-running-op POST and any
    /// future sync surfaces.
    pub(crate) fn post_sync<B, R>(
        &self,
        model: &str,
        method: &str,
        body: &B,
    ) -> Result<R, BackendError>
    where
        B: serde::Serialize,
        R: DeserializeOwned,
    {
        let path = format!("models/{model}:{method}");
        let url = compose_url(BASE_URL, &path, &auth(&self.api_key));
        let json_body = serde_json::to_string(body)
            .map_err(|e| BackendError::InvalidRequest(format!("serialize body: {e}")))?;
        let req = ureq::post(&url)
            .set("Accept", "application/json")
            .set("Content-Type", "application/json");
        send_decode(req.send_string(&json_body))
    }

    /// GET an operation by full name (`models/<model>/operations/<id>`).
    pub(crate) fn get_operation<R: DeserializeOwned>(
        &self,
        operation_name: &str,
    ) -> Result<R, BackendError> {
        let url = compose_url(BASE_URL, operation_name, &auth(&self.api_key));
        let req = ureq::get(&url).set("Accept", "application/json");
        send_decode(req.call())
    }

    /// Poll a long-running operation until done or wall-time exceeded.
    /// Generic over the operation's response shape — caller decides
    /// what `R` decodes to.
    pub(crate) fn poll_until_done<R>(&self, operation_name: &str) -> Result<R, BackendError>
    where
        R: DeserializeOwned + IsOperationDone,
    {
        let deadline = std::time::Instant::now()
            + Duration::from_secs(DEFAULT_POLL_TIMEOUT_SECS);
        loop {
            let op: R = self.get_operation(operation_name)?;
            if op.is_done() {
                return Ok(op);
            }
            if std::time::Instant::now() >= deadline {
                return Err(BackendError::Decode(format!(
                    "operation {operation_name} did not complete within {DEFAULT_POLL_TIMEOUT_SECS}s"
                )));
            }
            std::thread::sleep(Duration::from_secs(POLL_INTERVAL_SECS));
        }
    }

    /// Fetch arbitrary asset bytes from a Google-side URL. Used to pull
    /// the rendered MP4 once an op is done. The URL already carries
    /// `?key=…` when returned from the operation; for the rare case it
    /// doesn't, we append.
    pub(crate) fn fetch_asset(&self, url: &str) -> Result<Vec<u8>, BackendError> {
        let url = if url.contains("key=") {
            url.to_string()
        } else if url.contains('?') {
            format!("{url}&key={}", self.api_key)
        } else {
            format!("{url}?key={}", self.api_key)
        };
        let resp = ureq::get(&url)
            .call()
            .map_err(|e| BackendError::Transport(format!("fetch asset: {e}")))?;
        let mut buf = Vec::new();
        resp.into_reader()
            .read_to_end(&mut buf)
            .map_err(|e| BackendError::Transport(format!("read asset body: {e}")))?;
        Ok(buf)
    }
}

/// Implemented by response shapes that have a `done` flag — the
/// polling helper reads this to decide when to stop.
pub(crate) trait IsOperationDone {
    /// `true` once the long-running operation completed (regardless of
    /// success/failure — caller inspects payload).
    fn is_done(&self) -> bool;
}

fn auth(key: &str) -> AuthScheme {
    AuthScheme::QueryParam {
        name: "key".into(),
        value: key.into(),
    }
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
