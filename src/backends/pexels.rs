//! Pexels stock-search adapter.
//!
//! Wraps the Pexels Videos API (<https://www.pexels.com/api/documentation/>):
//! a free, no-charge tier with a per-API-key rate limit. The free tier
//! is the reason this is our primary stock backend.
//!
//! ## Credentials
//!
//! Reads the API key from the `PEXELS_API_KEY` environment variable.
//!
//! ## Wire shape
//!
//! `GET https://api.pexels.com/videos/search?query=<q>&per_page=<n>&page=<p>&orientation=<o>&min_duration=<min>&max_duration=<max>`
//! Headers: `Authorization: <api_key>`.
//!
//! Response is the canonical Pexels JSON. We extract a stable subset
//! into `StockSearchResult` and ignore the rest (full body lives in
//! the cache manifest if anyone needs to drill into it).

use crate::backends::cache::{utc_now_iso8601, AssetCache, Manifest};
use crate::backends::stock::{
    check_budget, Orientation, StockItem, StockSearchBackend, StockSearchRequest,
    StockSearchResult, CLUSTER,
};
use crate::backends::{
    mode_label, BackendCallOutcome, BackendError, CostEstimate, RunMode,
};
use serde::Deserialize;
use std::path::PathBuf;

/// Provider id used in cache keys + manifests.
pub const PROVIDER: &str = "pexels";

/// Env var the API key is read from.
pub const KEY_ENV: &str = "PEXELS_API_KEY";

const API_BASE: &str = "https://api.pexels.com/videos/search";

/// Pexels adapter. Holds the API key (loaded once at construction) and
/// an `AssetCache` for response caching.
#[derive(Debug, Clone)]
pub struct PexelsAdapter {
    api_key: String,
    cache: AssetCache,
}

impl PexelsAdapter {
    /// Construct from an explicit key + cache root. Useful for tests.
    pub fn with_key(api_key: impl Into<String>, cache_root: impl Into<PathBuf>) -> Self {
        Self {
            api_key: api_key.into(),
            cache: AssetCache::new(cache_root),
        }
    }

    /// Construct from env (`PEXELS_API_KEY`). Returns `MissingCredential`
    /// when the var isn't set.
    pub fn from_env(cache_root: impl Into<PathBuf>) -> Result<Self, BackendError> {
        let key =
            std::env::var(KEY_ENV).map_err(|_| BackendError::MissingCredential(KEY_ENV.into()))?;
        if key.trim().is_empty() {
            return Err(BackendError::MissingCredential(KEY_ENV.into()));
        }
        Ok(Self::with_key(key, cache_root))
    }
}

impl StockSearchBackend for PexelsAdapter {
    fn name(&self) -> &'static str {
        PROVIDER
    }

    fn estimate_cost(&self, _: &StockSearchRequest) -> CostEstimate {
        CostEstimate {
            provider: PROVIDER.into(),
            cost_usd: 0.0,
            explanation: "Pexels is free; rate-limit costs are not modeled here.".into(),
        }
    }

    fn search(
        &self,
        request: &StockSearchRequest,
        mode: RunMode,
    ) -> Result<BackendCallOutcome<StockSearchResult>, BackendError> {
        let estimate = self.estimate_cost(request);
        check_budget(&estimate, mode)?;

        let request_hash = AssetCache::request_hash(PROVIDER, CLUSTER, request)?;

        // Cache hit short-circuits the network entirely.
        if let Some(manifest) = self.cache.hit(PROVIDER, &request_hash)? {
            let response: StockSearchResult =
                serde_json::from_value(manifest.response.clone()).map_err(|e| {
                    BackendError::Cache(format!("decode cached response: {e}"))
                })?;
            return Ok(BackendCallOutcome {
                response,
                provider: PROVIDER.into(),
                request_hash,
                cached: true,
                cost_estimate_usd: 0.0,
                mode: mode_label(mode),
            });
        }

        // Dry-run: synthesize an empty response describing what we would
        // have asked for. Don't store to cache (so the next live call
        // still goes through).
        if !mode.is_live() {
            let response = StockSearchResult {
                provider: PROVIDER.into(),
                items: Vec::new(),
                total_hits: None,
                page: request.page,
            };
            return Ok(BackendCallOutcome {
                response,
                provider: PROVIDER.into(),
                request_hash,
                cached: false,
                cost_estimate_usd: estimate.cost_usd,
                mode: mode_label(mode),
            });
        }

        // Live: hit the API.
        let url = build_url(request);
        let resp = ureq::get(&url)
            .set("Authorization", &self.api_key)
            .call();
        let body = match resp {
            Ok(r) => r
                .into_string()
                .map_err(|e| BackendError::Transport(e.to_string()))?,
            Err(ureq::Error::Status(status, response)) => {
                let body = response.into_string().unwrap_or_default();
                return Err(BackendError::HttpStatus {
                    status,
                    body: truncate_body(&body),
                });
            }
            Err(e) => return Err(BackendError::Transport(e.to_string())),
        };

        let parsed: PexelsResponse = serde_json::from_str(&body)
            .map_err(|e| BackendError::Decode(format!("pexels: {e}")))?;
        let response = parsed.into_result(request.page);

        // Store the canonical response in the cache for next time.
        let manifest = Manifest {
            version: 1,
            provider: PROVIDER.into(),
            cluster: CLUSTER.into(),
            request_hash: request_hash.clone(),
            request: serde_json::to_value(request).map_err(|e| {
                BackendError::Cache(format!("serialize request for cache: {e}"))
            })?,
            response: serde_json::to_value(&response).map_err(|e| {
                BackendError::Cache(format!("serialize response for cache: {e}"))
            })?,
            cost_estimate_usd: 0.0,
            asset_path: None,
            created_at: utc_now_iso8601(),
        };
        self.cache.store(&manifest)?;

        Ok(BackendCallOutcome {
            response,
            provider: PROVIDER.into(),
            request_hash,
            cached: false,
            cost_estimate_usd: estimate.cost_usd,
            mode: mode_label(mode),
        })
    }
}

/// Build the Pexels search URL from a `StockSearchRequest`. Factored out
/// so tests can verify the URL without making HTTP calls.
fn build_url(req: &StockSearchRequest) -> String {
    let per_page = req.per_page.clamp(1, 80);
    let page = req.page.max(1);
    let mut url = format!(
        "{API_BASE}?query={q}&per_page={per_page}&page={page}",
        q = urlencode(&req.query),
    );
    if let Some(o) = req.orientation {
        url.push_str("&orientation=");
        url.push_str(match o {
            Orientation::Landscape => "landscape",
            Orientation::Portrait => "portrait",
            Orientation::Square => "square",
        });
    }
    if let Some(min) = req.min_duration_secs {
        url.push_str(&format!("&min_duration={min}"));
    }
    if let Some(max) = req.max_duration_secs {
        url.push_str(&format!("&max_duration={max}"));
    }
    url
}

fn urlencode(s: &str) -> String {
    // Minimal percent-encoder for query-string segment: encode anything
    // that isn't unreserved per RFC 3986. Spaces become %20 (the API
    // accepts both `+` and `%20`; %20 keeps the encoding consistent).
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        let is_safe = b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~');
        if is_safe {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{:02X}", b));
        }
    }
    out
}

fn truncate_body(body: &str) -> String {
    if body.len() <= 512 {
        body.to_string()
    } else {
        format!("{}…", &body[..512])
    }
}

// === Pexels wire types (subset) ===

#[derive(Debug, Clone, Deserialize)]
struct PexelsResponse {
    videos: Vec<PexelsVideo>,
    #[serde(default)]
    total_results: Option<u32>,
}

#[derive(Debug, Clone, Deserialize)]
struct PexelsVideo {
    id: u64,
    width: u32,
    height: u32,
    duration: u32,
    #[serde(default)]
    image: Option<String>,
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    user: Option<PexelsUser>,
    #[serde(default)]
    video_files: Vec<PexelsVideoFile>,
}

#[derive(Debug, Clone, Deserialize)]
struct PexelsUser {
    #[serde(default)]
    name: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct PexelsVideoFile {
    #[serde(default)]
    quality: Option<String>,
    #[serde(default)]
    width: Option<u32>,
    #[serde(default)]
    height: Option<u32>,
    link: String,
}

impl PexelsResponse {
    fn into_result(self, page: u32) -> StockSearchResult {
        let items = self
            .videos
            .into_iter()
            .map(|v| {
                let file = pick_best_file(&v.video_files);
                StockItem {
                    id: v.id.to_string(),
                    url: file
                        .map(|f| f.link.clone())
                        .unwrap_or_else(|| v.url.clone().unwrap_or_default()),
                    thumb_url: v.image.clone(),
                    width: file.and_then(|f| f.width).unwrap_or(v.width),
                    height: file.and_then(|f| f.height).unwrap_or(v.height),
                    duration_secs: Some(v.duration),
                    author: v.user.as_ref().and_then(|u| u.name.clone()),
                    license: Some("pexels".into()),
                    source_page: v.url,
                }
            })
            .collect();
        StockSearchResult {
            provider: PROVIDER.into(),
            items,
            total_hits: self.total_results,
            page,
        }
    }
}

/// Pick the best video file from the Pexels variants. Prefer the
/// `hd`-quality entry, otherwise the largest by area, otherwise the
/// first one.
fn pick_best_file(files: &[PexelsVideoFile]) -> Option<&PexelsVideoFile> {
    if files.is_empty() {
        return None;
    }
    let hd = files.iter().find(|f| f.quality.as_deref() == Some("hd"));
    if let Some(hd) = hd {
        return Some(hd);
    }
    files.iter().max_by_key(|f| {
        f.width.unwrap_or(0) as u64 * f.height.unwrap_or(0) as u64
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_includes_query_per_page_page() {
        let mut req = StockSearchRequest::new("highway desert");
        req.per_page = 7;
        req.page = 3;
        let url = build_url(&req);
        assert!(url.contains("query=highway%20desert"));
        assert!(url.contains("per_page=7"));
        assert!(url.contains("page=3"));
    }

    #[test]
    fn url_optional_filters() {
        let mut req = StockSearchRequest::new("ocean");
        req.orientation = Some(Orientation::Portrait);
        req.min_duration_secs = Some(5);
        req.max_duration_secs = Some(20);
        let url = build_url(&req);
        assert!(url.contains("orientation=portrait"));
        assert!(url.contains("min_duration=5"));
        assert!(url.contains("max_duration=20"));
    }

    #[test]
    fn per_page_is_clamped_to_pexels_max() {
        let mut req = StockSearchRequest::new("ocean");
        req.per_page = 9_999;
        let url = build_url(&req);
        assert!(url.contains("per_page=80"));
    }

    #[test]
    fn urlencode_escapes_special_chars() {
        assert_eq!(urlencode("a b/c?d"), "a%20b%2Fc%3Fd");
        assert_eq!(urlencode("ocean"), "ocean");
    }

    #[test]
    fn pick_best_file_prefers_hd() {
        let files = vec![
            PexelsVideoFile {
                quality: Some("sd".into()),
                width: Some(640),
                height: Some(360),
                link: "sd".into(),
            },
            PexelsVideoFile {
                quality: Some("hd".into()),
                width: Some(1280),
                height: Some(720),
                link: "hd".into(),
            },
        ];
        assert_eq!(pick_best_file(&files).unwrap().link, "hd");
    }

    #[test]
    fn pick_best_file_falls_back_to_largest() {
        let files = vec![
            PexelsVideoFile {
                quality: Some("sd".into()),
                width: Some(640),
                height: Some(360),
                link: "small".into(),
            },
            PexelsVideoFile {
                quality: Some("sd".into()),
                width: Some(1920),
                height: Some(1080),
                link: "big".into(),
            },
        ];
        assert_eq!(pick_best_file(&files).unwrap().link, "big");
    }

    #[test]
    fn pexels_response_decodes_minimal_payload() {
        let body = r#"{
            "videos": [{
                "id": 1234,
                "width": 1920,
                "height": 1080,
                "duration": 12,
                "image": "https://images.example/1234.jpg",
                "url": "https://www.pexels.com/video/1234",
                "user": {"name": "Some Author"},
                "video_files": [
                    {"quality": "sd", "width": 640, "height": 360, "link": "https://v.example/sd.mp4"},
                    {"quality": "hd", "width": 1920, "height": 1080, "link": "https://v.example/hd.mp4"}
                ]
            }],
            "total_results": 42
        }"#;
        let parsed: PexelsResponse = serde_json::from_str(body).unwrap();
        let result = parsed.into_result(1);
        assert_eq!(result.items.len(), 1);
        let item = &result.items[0];
        assert_eq!(item.id, "1234");
        assert_eq!(item.url, "https://v.example/hd.mp4");
        assert_eq!(item.width, 1920);
        assert_eq!(item.height, 1080);
        assert_eq!(item.duration_secs, Some(12));
        assert_eq!(item.author.as_deref(), Some("Some Author"));
        assert_eq!(item.license.as_deref(), Some("pexels"));
        assert_eq!(result.total_hits, Some(42));
    }

    #[test]
    fn dry_run_returns_empty_response_with_no_cache_write() {
        let tmp = std::env::temp_dir().join(format!(
            "wavelet-pexels-dryrun-{}",
            AssetCache::request_hash("seed", "seed", &"x").unwrap()
        ));
        let _ = std::fs::remove_dir_all(&tmp);
        let adapter = PexelsAdapter::with_key("fake-key", &tmp);
        let req = StockSearchRequest::new("ocean");
        let out = adapter.search(&req, RunMode::DryRun).unwrap();
        assert!(out.response.items.is_empty());
        assert_eq!(out.provider, "pexels");
        assert_eq!(out.mode, "dry-run");
        assert!(!out.cached);
        // Cache must still be empty.
        assert!(adapter
            .cache
            .hit(PROVIDER, &out.request_hash)
            .unwrap()
            .is_none());
    }

    #[test]
    fn missing_env_var_is_explicit() {
        // SAFETY: we own this var name for the test; clear it deliberately.
        std::env::remove_var(KEY_ENV);
        let tmp = std::env::temp_dir().join("wavelet-pexels-no-env");
        let err = PexelsAdapter::from_env(&tmp).unwrap_err();
        match err {
            BackendError::MissingCredential(name) => assert_eq!(name, KEY_ENV),
            other => panic!("expected MissingCredential, got {other:?}"),
        }
    }
}
