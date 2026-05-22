//! Content-addressed cache for backend responses.
//!
//! Every backend call goes through `AssetCache`. The cache key is the
//! request hash — a stable hash over `(provider, kind, request_json)`.
//! Re-running the same request returns the cached response instead of
//! re-billing the backend.
//!
//! Layout on disk (rooted at `<cache_root>/`):
//!
//! ```text
//! <cache_root>/
//!   <provider>/
//!     <request_hash>.manifest.json
//!     <request_hash>.<ext>           (optional binary blob — image, audio, video)
//! ```
//!
//! The manifest carries provenance metadata so the agent can audit any
//! cached entry: provider, original request, cost estimate at fetch
//! time, response payload (or a pointer to the binary blob), and an
//! RFC3339 timestamp.

use crate::backends::BackendError;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::hash::Hasher;
use std::io::Read;
use std::path::{Path, PathBuf};

/// One on-disk cache entry. Stored as `<provider>/<request_hash>.manifest.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Manifest {
    /// Schema version of this manifest. Bump on breaking changes.
    pub version: u32,
    /// Backend identifier (`pexels`, `suno`, …).
    pub provider: String,
    /// Cluster trait the backend implements (`stock_search`, etc.) —
    /// helps the agent reason about capability availability across
    /// cached entries.
    pub cluster: String,
    /// Stable hash of the original request.
    pub request_hash: String,
    /// Original request payload as JSON. The agent can read this to
    /// audit *what was asked for*.
    pub request: serde_json::Value,
    /// Backend response payload as JSON.
    pub response: serde_json::Value,
    /// Estimated cost incurred at fetch time, USD.
    pub cost_estimate_usd: f32,
    /// Optional relative path (from the cache root) to a binary blob
    /// associated with this entry (e.g. a downloaded video).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub asset_path: Option<String>,
    /// RFC3339-ish timestamp (`YYYY-MM-DDTHH:MM:SSZ`) of when the entry
    /// was written. UTC.
    pub created_at: String,
}

/// On-disk cache rooted at a single directory. Cheap to construct;
/// every method takes `&self` and is thread-safe at the OS level (each
/// op is a single file read or write).
#[derive(Debug, Clone)]
pub struct AssetCache {
    root: PathBuf,
}

impl AssetCache {
    /// Construct a cache rooted at `root`. The directory is created on
    /// first write; `new` is infallible.
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// Cache root directory.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Compute the request hash for an arbitrary `Serialize` payload.
    /// The hash includes the provider name + cluster + JSON-serialized
    /// request, so identical requests across providers don't collide.
    pub fn request_hash<R: Serialize>(
        provider: &str,
        cluster: &str,
        request: &R,
    ) -> Result<String, BackendError> {
        let payload = serde_json::to_vec(request)
            .map_err(|e| BackendError::Cache(format!("hash serialize: {e}")))?;
        let mut h = twox_hash::XxHash64::with_seed(0);
        h.write(provider.as_bytes());
        h.write(b"\x00");
        h.write(cluster.as_bytes());
        h.write(b"\x00");
        h.write(&payload);
        Ok(format!("{:016x}", h.finish()))
    }

    /// Look up an existing manifest by request hash. Returns `None` if
    /// the cache hasn't seen this request before. I/O errors are mapped
    /// to `BackendError::Cache`.
    pub fn hit(&self, provider: &str, request_hash: &str) -> Result<Option<Manifest>, BackendError> {
        let path = self.manifest_path(provider, request_hash);
        if !path.exists() {
            return Ok(None);
        }
        let src = std::fs::read_to_string(&path)
            .map_err(|e| BackendError::Cache(format!("read {}: {e}", path.display())))?;
        let manifest: Manifest = serde_json::from_str(&src)
            .map_err(|e| BackendError::Cache(format!("parse {}: {e}", path.display())))?;
        Ok(Some(manifest))
    }

    /// Store a manifest under `<root>/<provider>/<request_hash>.manifest.json`.
    /// Creates the provider subdir if needed.
    pub fn store(&self, manifest: &Manifest) -> Result<PathBuf, BackendError> {
        let dir = self.root.join(&manifest.provider);
        std::fs::create_dir_all(&dir)
            .map_err(|e| BackendError::Cache(format!("mkdir {}: {e}", dir.display())))?;
        let path = self.manifest_path(&manifest.provider, &manifest.request_hash);
        let json = serde_json::to_string_pretty(manifest)
            .map_err(|e| BackendError::Cache(format!("serialize: {e}")))?;
        std::fs::write(&path, json)
            .map_err(|e| BackendError::Cache(format!("write {}: {e}", path.display())))?;
        Ok(path)
    }

    /// Resolve the absolute path for a manifest, whether or not it
    /// exists. Used by both `hit` and `store`.
    pub fn manifest_path(&self, provider: &str, request_hash: &str) -> PathBuf {
        self.root
            .join(provider)
            .join(format!("{request_hash}.manifest.json"))
    }

    /// Resolve the path a binary asset should live at, side-by-side with
    /// its manifest. `ext` is the file extension *without* the dot.
    pub fn asset_path(&self, provider: &str, request_hash: &str, ext: &str) -> PathBuf {
        self.root
            .join(provider)
            .join(format!("{request_hash}.{ext}"))
    }

    /// Write a binary asset alongside its manifest entry. Creates the
    /// provider subdir if needed. Returns the absolute path written.
    pub fn write_asset(
        &self,
        provider: &str,
        request_hash: &str,
        ext: &str,
        bytes: &[u8],
    ) -> Result<PathBuf, BackendError> {
        let dir = self.root.join(provider);
        std::fs::create_dir_all(&dir)
            .map_err(|e| BackendError::Cache(format!("mkdir {}: {e}", dir.display())))?;
        let path = self.asset_path(provider, request_hash, ext);
        std::fs::write(&path, bytes)
            .map_err(|e| BackendError::Cache(format!("write {}: {e}", path.display())))?;
        Ok(path)
    }
}

/// Current UTC timestamp formatted as RFC3339-ish (`YYYY-MM-DDTHH:MM:SSZ`).
/// Avoids the chrono dep by formatting from `SystemTime` directly.
pub fn utc_now_iso8601() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    // Convert to broken-down date/time without pulling chrono.
    let (year, month, day, hour, minute, second) = ts_to_ymdhms(secs);
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z")
}

/// Convert Unix seconds (UTC) to `(year, month, day, hour, minute, second)`.
/// Pure arithmetic — adapted from the Howard Hinnant date algorithm. No
/// timezone, no leap-second handling.
fn ts_to_ymdhms(secs: u64) -> (i32, u32, u32, u32, u32, u32) {
    let day = (secs / 86400) as i64;
    let sec_of_day = (secs % 86400) as u32;
    let hour = sec_of_day / 3600;
    let minute = (sec_of_day % 3600) / 60;
    let second = sec_of_day % 60;

    // Civil-from-days (Hinnant). Day 0 is 1970-01-01.
    let z = day + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = doy - (153 * mp + 2) / 5 + 1; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 }; // [1, 12]
    let year = if m <= 2 { y + 1 } else { y };

    (year as i32, m as u32, d as u32, hour, minute, second)
}

/// URL→file cache errors. Separate from [`BackendError`] because
/// `cache_url` runs outside any specific backend adapter — callers (the
/// brand-asset bridge, agent tools) consume the URL→path mapping as a
/// general utility, not as one provider's response.
#[derive(Debug, thiserror::Error)]
pub enum CacheError {
    /// Network or transport failure while fetching `url`.
    #[error("fetch {url}: {source}")]
    Http {
        /// The URL that failed to fetch.
        url: String,
        /// Underlying transport / HTTP error.
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    },
    /// Filesystem error while reading or writing the cache.
    #[error("io {path}: {source}")]
    Io {
        /// Path the I/O was attempted against.
        path: String,
        /// Underlying I/O error.
        #[source]
        source: std::io::Error,
    },
    /// The remote returned no usable hint for which file extension to
    /// use — neither the URL path nor the `Content-Type` header named a
    /// recognized type.
    #[error("unknown content-type for {url}; cannot pick extension")]
    UnknownContentType {
        /// The URL whose type couldn't be inferred.
        url: String,
    },
}

/// Download `url` once to `<cache_root>/assets/<sha256(url)>.<ext>` and
/// return the resulting path. Idempotent — if a same-hash file exists,
/// no network call is made.
///
/// Bridges HTTPS-only vendor outputs (e.g. ADALIGN product crops) to
/// callers that need a local file path (e.g. Veo `image=` input).
///
/// Extension picking:
/// 1. The URL's existing last-segment extension when it's a recognized
///    image / audio / video type.
/// 2. The response's `Content-Type` header mapped to a canonical
///    extension.
/// 3. Otherwise [`CacheError::UnknownContentType`].
///
/// Concurrent callers with the same URL race-safely: each writes to a
/// unique `<sha>.<ext>.tmp.<pid>.<nanos>` file then atomically renames
/// onto the final path. POSIX `rename` is atomic; the last writer wins
/// and earlier writers' temp files are removed if they're still present.
pub fn cache_url(url: &str, cache_root: &Path) -> Result<PathBuf, CacheError> {
    let hash = sha256_hex(url.as_bytes());
    let assets_dir = cache_root.join("assets");

    if let Some(existing) = find_existing_asset(&assets_dir, &hash) {
        return Ok(existing);
    }

    std::fs::create_dir_all(&assets_dir).map_err(|e| CacheError::Io {
        path: assets_dir.display().to_string(),
        source: e,
    })?;

    let url_ext = url_extension_hint(url);
    let resp = ureq::get(url).call().map_err(|e| CacheError::Http {
        url: url.to_string(),
        source: Box::new(e),
    })?;
    let content_type = resp.header("Content-Type").map(|s| s.to_string());
    let ext = match url_ext {
        Some(e) => e,
        None => content_type
            .as_deref()
            .and_then(content_type_to_ext)
            .ok_or_else(|| CacheError::UnknownContentType { url: url.to_string() })?,
    };

    let final_path = assets_dir.join(format!("{hash}.{ext}"));

    if final_path.exists() {
        return Ok(final_path);
    }

    let tmp_path = assets_dir.join(format!(
        "{hash}.{ext}.tmp.{}.{}",
        std::process::id(),
        unique_suffix()
    ));

    let mut reader = resp.into_reader();
    let mut buf = Vec::with_capacity(64 * 1024);
    reader.read_to_end(&mut buf).map_err(|e| CacheError::Http {
        url: url.to_string(),
        source: Box::new(e),
    })?;

    std::fs::write(&tmp_path, &buf).map_err(|e| CacheError::Io {
        path: tmp_path.display().to_string(),
        source: e,
    })?;

    if let Err(e) = std::fs::rename(&tmp_path, &final_path) {
        let _ = std::fs::remove_file(&tmp_path);
        if final_path.exists() {
            return Ok(final_path);
        }
        return Err(CacheError::Io {
            path: final_path.display().to_string(),
            source: e,
        });
    }

    Ok(final_path)
}

/// SHA-256 of `bytes` as lowercase hex.
fn sha256_hex(bytes: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(bytes);
    let digest = h.finalize();
    let mut out = String::with_capacity(64);
    for b in digest.iter() {
        out.push_str(&format!("{b:02x}"));
    }
    out
}

/// Scan the assets directory for an already-cached file matching the
/// hash, regardless of extension. Returns the first match.
fn find_existing_asset(assets_dir: &Path, hash: &str) -> Option<PathBuf> {
    let entries = std::fs::read_dir(assets_dir).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        let name = path.file_name()?.to_str()?;
        if let Some(stem) = name.split('.').next() {
            if stem == hash && !name.contains(".tmp.") {
                return Some(path);
            }
        }
    }
    None
}

/// Pull the trailing extension out of `url`'s path component when it's a
/// recognized media type. Strips query strings + fragments first.
fn url_extension_hint(url: &str) -> Option<&'static str> {
    let path = url
        .split('?')
        .next()
        .unwrap_or(url)
        .split('#')
        .next()
        .unwrap_or(url);
    let last_segment = path.rsplit('/').next()?;
    let dot = last_segment.rfind('.')?;
    let raw = &last_segment[dot + 1..];
    normalize_ext(raw)
}

/// Map common `Content-Type` strings to a canonical extension. Returns
/// `None` for types we don't have a use for in the asset pipeline.
fn content_type_to_ext(content_type: &str) -> Option<&'static str> {
    let main = content_type
        .split(';')
        .next()
        .unwrap_or(content_type)
        .trim()
        .to_ascii_lowercase();
    match main.as_str() {
        "image/png" => Some("png"),
        "image/jpeg" | "image/jpg" => Some("jpg"),
        "image/webp" => Some("webp"),
        "image/gif" => Some("gif"),
        "image/avif" => Some("avif"),
        "image/heic" | "image/heif" => Some("heic"),
        "audio/mpeg" | "audio/mp3" => Some("mp3"),
        "audio/wav" | "audio/x-wav" | "audio/wave" => Some("wav"),
        "audio/flac" => Some("flac"),
        "audio/ogg" | "audio/vorbis" => Some("ogg"),
        "audio/opus" => Some("opus"),
        "audio/aac" | "audio/mp4" => Some("aac"),
        "video/mp4" => Some("mp4"),
        "video/webm" => Some("webm"),
        "video/quicktime" => Some("mov"),
        _ => None,
    }
}

/// Canonicalize a raw extension string. Lowercases, and only accepts the
/// types we know about — keeps unknown query-string artifacts from
/// pretending to be extensions.
fn normalize_ext(raw: &str) -> Option<&'static str> {
    match raw.to_ascii_lowercase().as_str() {
        "png" => Some("png"),
        "jpg" | "jpeg" => Some("jpg"),
        "webp" => Some("webp"),
        "gif" => Some("gif"),
        "avif" => Some("avif"),
        "heic" | "heif" => Some("heic"),
        "mp3" => Some("mp3"),
        "wav" => Some("wav"),
        "flac" => Some("flac"),
        "ogg" => Some("ogg"),
        "opus" => Some("opus"),
        "aac" => Some("aac"),
        "m4a" => Some("m4a"),
        "mp4" => Some("mp4"),
        "webm" => Some("webm"),
        "mov" => Some("mov"),
        _ => None,
    }
}

/// Best-effort unique suffix for atomic-rename temp files. Combines the
/// monotonic nanosecond counter with the thread id so two threads in the
/// same process at the same nanosecond still get distinct paths.
fn unique_suffix() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos() as u64)
        .unwrap_or(0);
    let tid = format!("{:?}", std::thread::current().id());
    format!("{n}-{nanos}-{}", tid.replace(|c: char| !c.is_ascii_alphanumeric(), ""))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn hash_is_deterministic_and_provider_scoped() {
        let req = json!({"query": "ocean", "page": 1});
        let a = AssetCache::request_hash("pexels", "stock_search", &req).unwrap();
        let b = AssetCache::request_hash("pexels", "stock_search", &req).unwrap();
        let c = AssetCache::request_hash("pond5", "stock_search", &req).unwrap();
        assert_eq!(a, b, "same input must produce same hash");
        assert_ne!(a, c, "different providers must produce different hashes");
        assert_eq!(a.len(), 16, "hash must be 16 hex chars (xxhash64)");
    }

    #[test]
    fn store_then_hit_round_trips() {
        let tmp = std::env::temp_dir().join(format!(
            "wavelet-cache-test-{}",
            AssetCache::request_hash("seed", "seed", &"x").unwrap()
        ));
        let cache = AssetCache::new(&tmp);
        let manifest = Manifest {
            version: 1,
            provider: "pexels".into(),
            cluster: "stock_search".into(),
            request_hash: "abc123".into(),
            request: json!({"q": "test"}),
            response: json!({"items": []}),
            cost_estimate_usd: 0.0,
            asset_path: None,
            created_at: utc_now_iso8601(),
        };
        let written = cache.store(&manifest).unwrap();
        assert!(written.exists());
        let hit = cache.hit("pexels", "abc123").unwrap().unwrap();
        assert_eq!(hit.provider, "pexels");
        assert_eq!(hit.response, json!({"items": []}));
        // Cleanup.
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn miss_returns_none() {
        let tmp = std::env::temp_dir().join("wavelet-cache-miss-test");
        let cache = AssetCache::new(&tmp);
        let _ = std::fs::create_dir_all(&tmp);
        assert!(cache.hit("pexels", "nonexistent").unwrap().is_none());
    }

    #[test]
    fn utc_now_is_iso8601_shape() {
        let ts = utc_now_iso8601();
        // Format check: 2026-01-01T00:00:00Z
        assert_eq!(ts.len(), 20);
        assert_eq!(ts.chars().nth(4), Some('-'));
        assert_eq!(ts.chars().nth(10), Some('T'));
        assert_eq!(ts.chars().last(), Some('Z'));
    }

    #[test]
    fn url_ext_hint_strips_query_and_fragment() {
        assert_eq!(url_extension_hint("https://x.test/a/b/foo.png"), Some("png"));
        assert_eq!(
            url_extension_hint("https://x.test/a/foo.JPG?token=abc"),
            Some("jpg")
        );
        assert_eq!(
            url_extension_hint("https://x.test/a/foo.webp#frag"),
            Some("webp")
        );
        assert_eq!(url_extension_hint("https://x.test/no-extension"), None);
        assert_eq!(url_extension_hint("https://x.test/a.unknownext"), None);
    }

    #[test]
    fn content_type_maps_recognized_types() {
        assert_eq!(content_type_to_ext("image/png"), Some("png"));
        assert_eq!(
            content_type_to_ext("image/jpeg; charset=binary"),
            Some("jpg")
        );
        assert_eq!(content_type_to_ext("AUDIO/MPEG"), Some("mp3"));
        assert_eq!(content_type_to_ext("application/octet-stream"), None);
    }

    #[test]
    fn sha256_is_stable_lowercase_hex() {
        let a = sha256_hex(b"hello");
        let b = sha256_hex(b"hello");
        assert_eq!(a, b);
        assert_eq!(a.len(), 64);
        assert!(a.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
        assert_eq!(
            a,
            "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
        );
    }

    #[test]
    fn cache_url_idempotent_when_file_exists() {
        let tmp = tempfile::tempdir().unwrap();
        let url = "https://example.invalid/already-cached.png";
        let hash = sha256_hex(url.as_bytes());
        let assets = tmp.path().join("assets");
        std::fs::create_dir_all(&assets).unwrap();
        let pre_placed = assets.join(format!("{hash}.png"));
        std::fs::write(&pre_placed, b"\x89PNG\r\n\x1a\n").unwrap();

        let got = cache_url(url, tmp.path()).unwrap();
        assert_eq!(got, pre_placed);
        let bytes = std::fs::read(&got).unwrap();
        assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n", "cached file must not be re-fetched");
    }

    #[test]
    fn cache_url_rejects_unreachable_host() {
        let tmp = tempfile::tempdir().unwrap();
        // Port 1 is the TCPMUX reserved port; nothing listens by default,
        // so connect refuses fast. The URL has a `.png` so extension
        // resolution doesn't short-circuit before the network call.
        let url = "http://127.0.0.1:1/missing.png";
        let err = cache_url(url, tmp.path()).unwrap_err();
        match err {
            CacheError::Http { url: u, .. } => assert_eq!(u, url),
            other => panic!("expected Http, got {other:?}"),
        }
    }

    #[test]
    fn unique_suffix_is_unique_across_calls() {
        let a = unique_suffix();
        let b = unique_suffix();
        assert_ne!(a, b);
    }

    /// Integration test against a tiny public asset. Marked `#[ignore]`
    /// so CI without network access stays green; run locally with
    /// `cargo test -p wavelet --lib backends::cache::tests -- --ignored`.
    #[test]
    #[ignore]
    fn cache_url_downloads_and_skips_second_time() {
        let tmp = tempfile::tempdir().unwrap();
        // Tiny stable image asset on a permanent CDN.
        let url = "https://www.rust-lang.org/static/images/favicon-32x32.png";

        let first = cache_url(url, tmp.path()).unwrap();
        assert!(first.exists());
        let first_meta = std::fs::metadata(&first).unwrap();
        assert!(first_meta.len() > 0);
        let first_mtime = first_meta.modified().unwrap();

        // Sleep a touch so any re-fetch would visibly bump mtime.
        std::thread::sleep(std::time::Duration::from_millis(50));

        let second = cache_url(url, tmp.path()).unwrap();
        assert_eq!(first, second);
        let second_mtime = std::fs::metadata(&second).unwrap().modified().unwrap();
        assert_eq!(first_mtime, second_mtime, "second call must skip the fetch");
    }

    /// A URL whose path has no extension AND whose server returns a
    /// `Content-Type` we don't map (e.g. `text/plain`) must surface
    /// [`CacheError::UnknownContentType`]. Network-gated.
    #[test]
    #[ignore]
    fn cache_url_unknown_content_type_when_no_hint() {
        let tmp = tempfile::tempdir().unwrap();
        // GitHub raw serves this README as `text/plain` with no
        // extension in the URL — both extension-hint paths miss.
        let url =
            "https://raw.githubusercontent.com/octocat/Hello-World/master/README";
        let err = cache_url(url, tmp.path()).unwrap_err();
        match err {
            CacheError::UnknownContentType { url: u } => assert_eq!(u, url),
            other => panic!("expected UnknownContentType, got {other:?}"),
        }
    }

    /// Concurrent test against the same asset. Network-gated,
    /// `#[ignore]` by default.
    #[test]
    #[ignore]
    fn cache_url_concurrent_callers_converge_on_one_file() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().to_path_buf();
        let url = "https://www.rust-lang.org/static/images/favicon-32x32.png";

        let handles: Vec<_> = (0..4)
            .map(|_| {
                let r = root.clone();
                let u = url.to_string();
                std::thread::spawn(move || cache_url(&u, &r).unwrap())
            })
            .collect();
        let results: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();

        for r in &results {
            assert_eq!(*r, results[0]);
        }
        let assets = root.join("assets");
        let final_files: Vec<_> = std::fs::read_dir(&assets)
            .unwrap()
            .flatten()
            .filter(|e| {
                e.file_name()
                    .to_str()
                    .map(|n| !n.contains(".tmp."))
                    .unwrap_or(false)
            })
            .collect();
        assert_eq!(
            final_files.len(),
            1,
            "expected exactly one final asset file, found {}",
            final_files.len()
        );
    }

    #[test]
    fn known_unix_timestamps_decode_correctly() {
        // 1970-01-01T00:00:00Z
        assert_eq!(ts_to_ymdhms(0), (1970, 1, 1, 0, 0, 0));
        // 2000-01-01T00:00:00Z
        assert_eq!(ts_to_ymdhms(946_684_800), (2000, 1, 1, 0, 0, 0));
        // 2026-05-17T12:34:56Z — sanity check today's range.
        let (y, m, _, h, mi, s) = ts_to_ymdhms(1_779_510_896);
        assert_eq!((y, m), (2026, 5));
        assert!((0..24).contains(&h));
        assert!((0..60).contains(&mi));
        assert!((0..60).contains(&s));
    }
}
