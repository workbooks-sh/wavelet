//! Generic HTTP backend client — auth + `post_sync` + `fetch_asset` +
//! shared `AssetCache`, parameterized over an [`AuthScheme`].
//!
//! Each external HTTP vendor (Fal, Roboflow, Modal, OpenAI-shape,
//! …) plugs in via a thin newtype wrapper plus a `*_from_env`
//! convenience builder. The wire-level plumbing (URL composition,
//! header injection, error mapping, body truncation) lives here once.

use crate::backends::BackendError;
use crate::backends::cache::AssetCache;
use serde::de::DeserializeOwned;
use std::io::Read;
use std::path::PathBuf;

/// Env var Fal's API key is read from.
pub const FAL_KEY_ENV: &str = "FAL_KEY";

/// Env var Roboflow's API key is read from.
pub const ROBOFLOW_KEY_ENV: &str = "ROBOFLOW_API_KEY";

/// Authentication scheme for an HTTP-based backend.
#[derive(Debug, Clone)]
pub enum AuthScheme {
    /// `Authorization: Key <token>` — Fal's format. Token is the full
    /// `<id>:<secret>` string read from one env var.
    FalKey(String),
    /// `Authorization: Bearer <token>` — common pattern (OpenAI, Modal).
    Bearer(String),
    /// Two custom headers — e.g. Modal's `x-modal-token-id` +
    /// `x-modal-token-secret`.
    HeaderPair {
        /// First header name.
        name1: String,
        /// First header value.
        value1: String,
        /// Second header name.
        value2: String,
        /// Second header value.
        name2: String,
    },
    /// API key as a query parameter — Roboflow's `?api_key=…` pattern.
    QueryParam {
        /// Query parameter name.
        name: String,
        /// Query parameter value (the API key).
        value: String,
    },
}

/// Shared HTTP client. Cheap to clone — holds only the base URL, the
/// auth scheme, and an `AssetCache` (which is itself cheaply cloneable).
#[derive(Debug, Clone)]
pub struct HttpBackendClient {
    base_url: String,
    auth: AuthScheme,
    cache: AssetCache,
}

impl HttpBackendClient {
    /// Build from an explicit base URL + auth + cache root. Useful for
    /// tests and as the underlying constructor every vendor-specific
    /// builder delegates to.
    pub fn new(
        base_url: impl Into<String>,
        auth: AuthScheme,
        cache_root: impl Into<PathBuf>,
    ) -> Self {
        Self {
            base_url: base_url.into(),
            auth,
            cache: AssetCache::new(cache_root),
        }
    }

    /// Shared cache.
    pub fn cache(&self) -> &AssetCache {
        &self.cache
    }

    /// Auth scheme (test/debug visibility only — never log or print).
    #[cfg(test)]
    pub(crate) fn auth(&self) -> &AuthScheme {
        &self.auth
    }

    /// Base URL.
    #[cfg(test)]
    pub(crate) fn base_url(&self) -> &str {
        &self.base_url
    }

    /// Build a Fal client from `FAL_KEY`.
    pub fn fal_from_env(cache_root: impl Into<PathBuf>) -> Result<Self, BackendError> {
        let key = read_required_env(FAL_KEY_ENV)?;
        Ok(Self::new(
            "https://fal.run",
            AuthScheme::FalKey(key),
            cache_root,
        ))
    }

    /// Build a Roboflow client from `ROBOFLOW_API_KEY`. Roboflow auths
    /// via a `?api_key=…` query parameter rather than a header.
    pub fn roboflow_from_env(cache_root: impl Into<PathBuf>) -> Result<Self, BackendError> {
        let key = read_required_env(ROBOFLOW_KEY_ENV)?;
        Ok(Self::new(
            "https://infer.roboflow.com",
            AuthScheme::QueryParam {
                name: "api_key".into(),
                value: key,
            },
            cache_root,
        ))
    }

    /// POST a JSON body to `<base_url>/<path>` and decode the response.
    /// Adapters pick `R` to match the vendor's response shape.
    pub(crate) fn post_sync<B, R>(&self, path: &str, body: &B) -> Result<R, BackendError>
    where
        B: serde::Serialize,
        R: DeserializeOwned,
    {
        let url = compose_url(&self.base_url, path, &self.auth);
        let json_body = serde_json::to_string(body)
            .map_err(|e| BackendError::InvalidRequest(format!("serialize body: {e}")))?;

        let mut req = ureq::post(&url)
            .set("Accept", "application/json")
            .set("Content-Type", "application/json");
        req = apply_auth_headers(req, &self.auth);

        let resp = req.send_string(&json_body);
        match resp {
            Ok(r) => {
                let body = r
                    .into_string()
                    .map_err(|e| BackendError::Transport(e.to_string()))?;
                serde_json::from_str(&body)
                    .map_err(|e| BackendError::Decode(format!("{path}: {e}")))
            }
            Err(ureq::Error::Status(status, response)) => {
                let body = response.into_string().unwrap_or_default();
                Err(BackendError::HttpStatus {
                    status,
                    body: truncate_body(&body),
                })
            }
            Err(e) => Err(BackendError::Transport(e.to_string())),
        }
    }

    /// Upload bytes to fal-storage. Two-step: POST to
    /// `rest.alpha.fal.ai/storage/upload/initiate` for a signed PUT
    /// URL, then PUT the body. Returns the public `file_url`.
    ///
    /// Only valid on Fal-authenticated clients — other auth schemes
    /// fall through to `MissingCredential` semantics since the storage
    /// endpoint speaks Fal's `Key …` auth.
    pub(crate) fn fal_upload_bytes(
        &self,
        bytes: &[u8],
        content_type: &str,
        file_name: &str,
    ) -> Result<String, BackendError> {
        #[derive(serde::Serialize)]
        struct InitiateBody<'a> {
            content_type: &'a str,
            file_name: &'a str,
        }
        #[derive(serde::Deserialize)]
        struct InitiateResp {
            file_url: String,
            upload_url: String,
        }
        let initiate = ureq::post(
            "https://rest.alpha.fal.ai/storage/upload/initiate?storage_type=fal-cdn-v3",
        )
        .set("Accept", "application/json")
        .set("Content-Type", "application/json");
        let initiate = apply_auth_headers(initiate, &self.auth);
        let body = serde_json::to_string(&InitiateBody { content_type, file_name })
            .map_err(|e| BackendError::InvalidRequest(format!("serialize initiate: {e}")))?;
        let resp: InitiateResp = match initiate.send_string(&body) {
            Ok(r) => {
                let raw = r
                    .into_string()
                    .map_err(|e| BackendError::Transport(e.to_string()))?;
                serde_json::from_str(&raw)
                    .map_err(|e| BackendError::Decode(format!("storage initiate: {e}")))?
            }
            Err(ureq::Error::Status(status, response)) => {
                let raw = response.into_string().unwrap_or_default();
                return Err(BackendError::HttpStatus {
                    status,
                    body: truncate_body(&raw),
                });
            }
            Err(e) => return Err(BackendError::Transport(e.to_string())),
        };
        let put = ureq::put(&resp.upload_url).set("Content-Type", content_type);
        match put.send_bytes(bytes) {
            Ok(_) => Ok(resp.file_url),
            Err(ureq::Error::Status(status, response)) => {
                let raw = response.into_string().unwrap_or_default();
                Err(BackendError::HttpStatus {
                    status,
                    body: truncate_body(&raw),
                })
            }
            Err(e) => Err(BackendError::Transport(e.to_string())),
        }
    }

    /// Fetch a binary asset from a URL the backend returned (typically a
    /// signed, short-lived link to a generated artifact). No auth
    /// header is applied — the URL itself is the credential.
    pub(crate) fn fetch_asset(&self, url: &str) -> Result<Vec<u8>, BackendError> {
        let resp = ureq::get(url).call();
        match resp {
            Ok(r) => {
                let mut buf = Vec::with_capacity(64 * 1024);
                r.into_reader()
                    .read_to_end(&mut buf)
                    .map_err(|e| BackendError::Transport(format!("read asset: {e}")))?;
                if buf.is_empty() {
                    return Err(BackendError::Decode("empty asset response".into()));
                }
                Ok(buf)
            }
            Err(ureq::Error::Status(status, response)) => {
                let body = response.into_string().unwrap_or_default();
                Err(BackendError::HttpStatus {
                    status,
                    body: truncate_body(&body),
                })
            }
            Err(e) => Err(BackendError::Transport(e.to_string())),
        }
    }
}

/// Read an env var, returning [`BackendError::MissingCredential`] when
/// unset or whitespace-only.
fn read_required_env(name: &str) -> Result<String, BackendError> {
    let raw = std::env::var(name).map_err(|_| BackendError::MissingCredential(name.into()))?;
    if raw.trim().is_empty() {
        return Err(BackendError::MissingCredential(name.into()));
    }
    Ok(raw)
}

/// Compose `<base>/<path>`, appending the query-param auth pair if the
/// scheme uses one. The separator picks `?` vs `&` based on whether the
/// path already contains a `?`.
pub(crate) fn compose_url(base: &str, path: &str, auth: &AuthScheme) -> String {
    let trimmed_base = base.trim_end_matches('/');
    let trimmed_path = path.trim_start_matches('/');
    let mut url = format!("{trimmed_base}/{trimmed_path}");
    if let AuthScheme::QueryParam { name, value } = auth {
        let sep = if url.contains('?') { '&' } else { '?' };
        url.push(sep);
        url.push_str(name);
        url.push('=');
        url.push_str(value);
    }
    url
}

/// Apply auth-scheme headers to a `ureq` request. The query-param
/// variant adds no headers (its credential rides in the URL).
fn apply_auth_headers(req: ureq::Request, auth: &AuthScheme) -> ureq::Request {
    match auth {
        AuthScheme::FalKey(token) => req.set("Authorization", &format!("Key {token}")),
        AuthScheme::Bearer(token) => req.set("Authorization", &format!("Bearer {token}")),
        AuthScheme::HeaderPair {
            name1,
            value1,
            name2,
            value2,
        } => req.set(name1, value1).set(name2, value2),
        AuthScheme::QueryParam { .. } => req,
    }
}

/// Truncate a response body for inclusion in an error message.
pub(crate) fn truncate_body(body: &str) -> String {
    if body.len() <= 512 {
        body.to_string()
    } else {
        format!("{}…", &body[..512])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        std::env::temp_dir().join(name)
    }

    #[test]
    fn auth_scheme_variants_build() {
        let _fal = AuthScheme::FalKey("id:secret".into());
        let _bearer = AuthScheme::Bearer("tok".into());
        let _pair = AuthScheme::HeaderPair {
            name1: "x-modal-token-id".into(),
            value1: "id".into(),
            name2: "x-modal-token-secret".into(),
            value2: "secret".into(),
        };
        let _qp = AuthScheme::QueryParam {
            name: "api_key".into(),
            value: "abc".into(),
        };
    }

    #[test]
    fn url_composition_joins_with_single_slash() {
        let auth = AuthScheme::FalKey("k".into());
        assert_eq!(
            compose_url("https://fal.run", "fal-ai/flux/schnell", &auth),
            "https://fal.run/fal-ai/flux/schnell"
        );
        assert_eq!(
            compose_url("https://fal.run/", "/fal-ai/flux/schnell", &auth),
            "https://fal.run/fal-ai/flux/schnell"
        );
    }

    #[test]
    fn query_param_appends_with_question_mark_when_clean() {
        let auth = AuthScheme::QueryParam {
            name: "api_key".into(),
            value: "abc".into(),
        };
        let url = compose_url("https://infer.roboflow.com", "yolov8n/1", &auth);
        assert_eq!(url, "https://infer.roboflow.com/yolov8n/1?api_key=abc");
    }

    #[test]
    fn query_param_appends_with_ampersand_when_path_has_query() {
        let auth = AuthScheme::QueryParam {
            name: "api_key".into(),
            value: "abc".into(),
        };
        let url = compose_url("https://infer.roboflow.com", "yolov8n/1?confidence=0.5", &auth);
        assert_eq!(
            url,
            "https://infer.roboflow.com/yolov8n/1?confidence=0.5&api_key=abc"
        );
    }

    #[test]
    fn fal_key_yields_authorization_key_header() {
        let req = ureq::post("http://localhost/none");
        let updated = apply_auth_headers(req, &AuthScheme::FalKey("id:secret".into()));
        assert_eq!(
            updated.header("Authorization"),
            Some("Key id:secret")
        );
    }

    #[test]
    fn bearer_yields_authorization_bearer_header() {
        let req = ureq::post("http://localhost/none");
        let updated = apply_auth_headers(req, &AuthScheme::Bearer("tok123".into()));
        assert_eq!(updated.header("Authorization"), Some("Bearer tok123"));
    }

    #[test]
    fn header_pair_applies_both_headers() {
        let req = ureq::post("http://localhost/none");
        let updated = apply_auth_headers(
            req,
            &AuthScheme::HeaderPair {
                name1: "x-modal-token-id".into(),
                value1: "tid".into(),
                name2: "x-modal-token-secret".into(),
                value2: "tsec".into(),
            },
        );
        assert_eq!(updated.header("x-modal-token-id"), Some("tid"));
        assert_eq!(updated.header("x-modal-token-secret"), Some("tsec"));
    }

    #[test]
    fn query_param_does_not_set_authorization_header() {
        let req = ureq::post("http://localhost/none");
        let updated = apply_auth_headers(
            req,
            &AuthScheme::QueryParam {
                name: "api_key".into(),
                value: "abc".into(),
            },
        );
        assert!(updated.header("Authorization").is_none());
    }

    #[test]
    fn fal_from_env_errors_when_unset() {
        // Use a guarded key name to avoid clobbering the developer's
        // real FAL_KEY when the test happens to share a process.
        unsafe { std::env::remove_var(FAL_KEY_ENV) };
        let err = HttpBackendClient::fal_from_env(tmp("wavelet-http-fal-no-env")).unwrap_err();
        match err {
            BackendError::MissingCredential(n) => assert_eq!(n, FAL_KEY_ENV),
            other => panic!("expected MissingCredential, got {other:?}"),
        }
    }

    #[test]
    fn roboflow_from_env_errors_when_unset() {
        unsafe { std::env::remove_var(ROBOFLOW_KEY_ENV) };
        let err =
            HttpBackendClient::roboflow_from_env(tmp("wavelet-http-rf-no-env")).unwrap_err();
        match err {
            BackendError::MissingCredential(n) => assert_eq!(n, ROBOFLOW_KEY_ENV),
            other => panic!("expected MissingCredential, got {other:?}"),
        }
    }

    #[test]
    fn roboflow_from_env_reads_the_named_var() {
        unsafe { std::env::set_var(ROBOFLOW_KEY_ENV, "rf-test-key") };
        let c =
            HttpBackendClient::roboflow_from_env(tmp("wavelet-http-rf-set")).expect("env present");
        assert_eq!(c.base_url(), "https://infer.roboflow.com");
        match c.auth() {
            AuthScheme::QueryParam { name, value } => {
                assert_eq!(name, "api_key");
                assert_eq!(value, "rf-test-key");
            }
            other => panic!("expected QueryParam, got {other:?}"),
        }
        unsafe { std::env::remove_var(ROBOFLOW_KEY_ENV) };
    }

    #[test]
    fn truncate_body_caps_at_512() {
        let long: String = "x".repeat(1000);
        let t = truncate_body(&long);
        assert!(t.ends_with('…'));
        assert!(t.chars().count() <= 513);
    }
}
