//! `brand.*` — wrap the local `adalign` CLI as agent tools.
//!
//! Five verbs: `brand.brief` (cross-channel: identity + ads + social),
//! `brand.fetch` (identity only — faster), `brand.catalog` (product
//! crawl, capped at 20 URLs), `brand.product` (single SKU + image URLs
//! for hero-shot conditioning), `brand.ads` (Meta + Google ad creatives
//! for tonal grounding). Shells out to `adalign` and parses its
//! `--json` stdout. Read-only — none of these touch the local SQLite.

use std::process::Command;

use serde_json::{json, Value};

use super::{arg_str as s, Tool, ToolRegistry, ToolResult};

const ADALIGN_FALLBACK: &str = "/Users/shinyobjectz/.local/bin/adalign";

pub fn register(r: &mut ToolRegistry) {
    r.register(BrandBrief);
    r.register(BrandFetch);
    r.register(BrandCatalog);
    r.register(BrandProduct);
    r.register(BrandAds);
}

fn run_adalign(name: &str, args: &[&str]) -> ToolResult {
    match run_adalign_raw(name, args) {
        Ok(v) => ToolResult::local_ok(name, v),
        Err(r) => r,
    }
}

/// Same shell-out + auth-envelope detection as `run_adalign`, but
/// returns the parsed JSON so callers can post-process (e.g. filter a
/// product list) before wrapping it in a `ToolResult`. The `Err` arm
/// carries an already-formed `ToolResult::local_err`.
fn run_adalign_raw(name: &str, args: &[&str]) -> Result<Value, ToolResult> {
    let try_invoke = |bin: &str| Command::new(bin).args(args).output();

    let out = match try_invoke("adalign") {
        Ok(o) => o,
        Err(_) => match try_invoke(ADALIGN_FALLBACK) {
            Ok(o) => o,
            Err(e) => return Err(ToolResult::local_err(
                name,
                format!("adalign CLI not found on PATH or at {ADALIGN_FALLBACK}: {e}"),
            )),
        },
    };

    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();

    if !out.status.success() {
        let detail = if !stderr.trim().is_empty() {
            stderr.chars().take(800).collect::<String>()
        } else if !stdout.trim().is_empty() {
            stdout.chars().take(800).collect::<String>()
        } else {
            format!("exit={}", out.status.code().unwrap_or(-1))
        };
        return Err(ToolResult::local_err(name, detail));
    }

    match serde_json::from_str::<Value>(stdout.trim()) {
        Ok(v) => {
            if let Some(true) = v.get("error").and_then(|e| e.as_bool()) {
                let msg = v.get("message").and_then(|m| m.as_str()).unwrap_or("adalign error");
                return Err(ToolResult::local_err(name, msg.to_string()));
            }
            Ok(v)
        }
        Err(e) => Err(ToolResult::local_err(
            name,
            format!("could not parse adalign JSON: {e}; stdout: {}", stdout.chars().take(400).collect::<String>()),
        )),
    }
}

pub struct BrandBrief;
impl Tool for BrandBrief {
    fn name(&self) -> &str { "brand.brief" }
    fn description(&self) -> &str {
        "Fetch a one-shot cross-channel brand brief — identity (logos, palette, descriptors, \
         fonts), social profiles (IG/TT/YT/X/FB), Meta ads, Google ads — for a domain. Use this \
         FIRST when authoring a commercial brief so the creative is grounded in real brand data. \
         Returns normalised JSON. Read-only — no local DB writes. Cost: free (worker-hosted)."
    }
    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "required": ["domain"],
            "properties": {
                "domain": { "type": "string", "description": "Brand domain, e.g. \"patagonia.com\" or \"airbnb.com\"." }
            }
        })
    }
    fn dispatch(&self, args: &Value) -> ToolResult {
        let domain = match s(args, "domain") {
            Some(v) => v,
            None => return ToolResult::local_err(self.name(), "missing `domain`"),
        };
        run_adalign(self.name(), &["brief", &domain, "--json"])
    }
}

pub struct BrandFetch;
impl Tool for BrandFetch {
    fn name(&self) -> &str { "brand.fetch" }
    fn description(&self) -> &str {
        "Lightweight brand identity only — logos, palette, descriptors, fonts. Use when you only \
         need visual identity, not ads or social profiles. Faster than `brand.brief`."
    }
    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "required": ["domain"],
            "properties": {
                "domain": { "type": "string", "description": "Brand domain, e.g. \"patagonia.com\"." }
            }
        })
    }
    fn dispatch(&self, args: &Value) -> ToolResult {
        let domain = match s(args, "domain") {
            Some(v) => v,
            None => return ToolResult::local_err(self.name(), "missing `domain`"),
        };
        run_adalign(self.name(), &["brand", "fetch", &domain, "--json"])
    }
}

pub struct BrandCatalog;
impl Tool for BrandCatalog {
    fn name(&self) -> &str { "brand.catalog" }
    fn description(&self) -> &str {
        "Crawl up to 20 products from the brand's e-commerce site. Returns products with image \
         URLs you can pass to `wavelet.image.scene_still --refs` for reference-conditioned generation."
    }
    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "required": ["domain"],
            "properties": {
                "domain": { "type": "string", "description": "Brand domain, e.g. \"patagonia.com\"." }
            }
        })
    }
    fn dispatch(&self, args: &Value) -> ToolResult {
        let domain = match s(args, "domain") {
            Some(v) => v,
            None => return ToolResult::local_err(self.name(), "missing `domain`"),
        };
        run_adalign(self.name(), &["catalog", "crawl", &domain, "--json", "--max-urls", "20"])
    }
}

pub struct BrandProduct;
impl Tool for BrandProduct {
    fn name(&self) -> &str { "brand.product" }
    fn description(&self) -> &str {
        "Fetch a single product from the brand's catalog matching `query`, with all its image \
         URLs — feed these to `wavelet.shot.img2vid` (Veo 3.1 first-frame conditioning) or \
         `wavelet.image.scene_still --refs` for hero-shot grounding. Crawls up to 20 products and \
         picks the first whose name/title matches the query substring. Returns \
         `{ product: { name, url, image_urls, description?, price?, ... }, raw }`."
    }
    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "required": ["domain", "query"],
            "properties": {
                "domain": { "type": "string", "description": "Brand domain, e.g. \"patagonia.com\"." },
                "query": { "type": "string", "description": "Product to find, e.g. \"down sweater\". Substring-matched against product name/title (case-insensitive)." }
            }
        })
    }
    fn dispatch(&self, args: &Value) -> ToolResult {
        let domain = match s(args, "domain") {
            Some(v) => v,
            None => return ToolResult::local_err(self.name(), "missing `domain`"),
        };
        let query = match s(args, "query") {
            Some(v) => v,
            None => return ToolResult::local_err(self.name(), "missing `query`"),
        };
        // `adalign catalog crawl` has no --query flag, so we crawl 20 and
        // filter client-side. Verified against `adalign catalog crawl --help`
        // on 2026-05-20.
        let raw = match run_adalign_raw(
            self.name(),
            &["catalog", "crawl", &domain, "--json", "--max-urls", "20"],
        ) {
            Ok(v) => v,
            Err(r) => return r,
        };
        let needle = query.to_lowercase();
        let products = locate_product_array(&raw);
        let hit = products.iter().find(|p| product_matches(p, &needle));
        match hit {
            Some(p) => {
                let images = collect_image_urls(p);
                let mut shaped = p.clone();
                if let Value::Object(map) = &mut shaped {
                    map.insert("image_urls".into(), Value::Array(
                        images.into_iter().map(Value::String).collect(),
                    ));
                }
                ToolResult::local_ok(self.name(), json!({ "product": shaped, "raw": raw }))
            }
            None => ToolResult::local_err(
                self.name(),
                format!(
                    "no product matched `{query}` in the first 20 crawled products from {domain}"
                ),
            ),
        }
    }
}

pub struct BrandAds;
impl Tool for BrandAds {
    fn name(&self) -> &str { "brand.ads" }
    fn description(&self) -> &str {
        "Discover Meta + Google ad creatives the brand currently runs — palette, copy voice, \
         pacing reference. Use after `brand.brief` to mirror the brand's actual ad register. \
         Returns `{ ads: [...] }` from `adalign ads search`."
    }
    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "required": ["domain"],
            "properties": {
                "domain": { "type": "string", "description": "Brand domain — used as the search query." },
                "limit": { "type": "integer", "description": "Max ads to return.", "default": 5 },
                "source": {
                    "type": "string",
                    "enum": ["meta", "google", "all"],
                    "description": "Which ad library to query.",
                    "default": "all"
                }
            }
        })
    }
    fn dispatch(&self, args: &Value) -> ToolResult {
        let domain = match s(args, "domain") {
            Some(v) => v,
            None => return ToolResult::local_err(self.name(), "missing `domain`"),
        };
        let limit = args.get("limit").and_then(|v| v.as_i64()).unwrap_or(5).max(1).to_string();
        let source = s(args, "source").unwrap_or_else(|| "all".to_string());
        let raw = match run_adalign_raw(
            self.name(),
            &["ads", "search", &domain, "--source", &source, "--limit", &limit, "--json"],
        ) {
            Ok(v) => v,
            Err(r) => return r,
        };
        let ads = match &raw {
            Value::Array(_) => raw.clone(),
            Value::Object(map) => map
                .get("ads")
                .or_else(|| map.get("results"))
                .or_else(|| map.get("data"))
                .cloned()
                .unwrap_or(raw.clone()),
            _ => raw.clone(),
        };
        ToolResult::local_ok(self.name(), json!({ "ads": ads }))
    }
}

/// Find the array of products in adalign's `catalog crawl --json` output.
/// The shape varies — sometimes top-level array, sometimes nested under
/// `products` / `items` / `data`. Returns an empty Vec if nothing
/// resembles a product list.
fn locate_product_array(v: &Value) -> Vec<Value> {
    if let Value::Array(arr) = v {
        return arr.clone();
    }
    if let Value::Object(map) = v {
        for key in ["products", "items", "data", "results", "catalog"] {
            if let Some(Value::Array(arr)) = map.get(key) {
                return arr.clone();
            }
        }
    }
    Vec::new()
}

fn product_matches(p: &Value, needle_lower: &str) -> bool {
    for key in ["name", "title", "product_name", "handle"] {
        if let Some(s) = p.get(key).and_then(|v| v.as_str()) {
            if s.to_lowercase().contains(needle_lower) {
                return true;
            }
        }
    }
    false
}

fn collect_image_urls(p: &Value) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut push = |s: &str| {
        let s = s.to_string();
        if !out.contains(&s) {
            out.push(s);
        }
    };
    for key in ["image_urls", "images", "product_images"] {
        if let Some(Value::Array(arr)) = p.get(key) {
            for item in arr {
                if let Some(s) = item.as_str() {
                    push(s);
                } else if let Some(s) = item.get("url").and_then(|v| v.as_str()) {
                    push(s);
                } else if let Some(s) = item.get("src").and_then(|v| v.as_str()) {
                    push(s);
                }
            }
        }
    }
    for key in ["image", "image_url", "thumbnail"] {
        if let Some(s) = p.get(key).and_then(|v| v.as_str()) {
            push(s);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::sync::Mutex;

    // PATH is process-global — serialize tests that mutate it, otherwise
    // parallel cargo-test threads see each other's mock shadows.
    static PATH_LOCK: Mutex<()> = Mutex::new(());

    /// Write a mock `adalign` shell script that emits `stdout_body` and
    /// returns `exit_code`. Returns the tempdir so the caller can prepend
    /// it to `PATH` for the duration of the test.
    fn mock_adalign(stdout_body: &str, exit_code: i32) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("adalign");
        // Single-quote the heredoc so the body isn't shell-expanded.
        let script = format!(
            "#!/bin/sh\ncat <<'__ADALIGN_EOF__'\n{stdout_body}\n__ADALIGN_EOF__\nexit {exit_code}\n"
        );
        fs::write(&path, script).unwrap();
        let mut perms = fs::metadata(&path).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&path, perms).unwrap();
        dir
    }

    fn with_mock_path<F: FnOnce()>(dir: &tempfile::TempDir, body: F) {
        let _guard = PATH_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let prev = std::env::var("PATH").unwrap_or_default();
        let new = format!("{}:{}", dir.path().display(), prev);
        std::env::set_var("PATH", &new);
        body();
        std::env::set_var("PATH", prev);
    }

    #[test]
    fn brand_product_picks_matching_sku_and_lifts_images() {
        let body = r#"{
          "products": [
            { "name": "Better Sweater", "url": "https://patagonia.com/p/bs", "images": ["https://cdn/x1.jpg"] },
            { "name": "Down Sweater Hoody", "url": "https://patagonia.com/p/dsh",
              "images": [{"url":"https://cdn/d1.jpg"}, "https://cdn/d2.jpg"],
              "price": "$329" }
          ]
        }"#;
        let dir = mock_adalign(body, 0);
        with_mock_path(&dir, || {
            let t = BrandProduct;
            let r = t.dispatch(&json!({ "domain": "patagonia.com", "query": "down sweater" }));
            assert!(r.ok, "summary: {}", r.summary);
            let p = &r.response["product"];
            assert_eq!(p["name"], "Down Sweater Hoody");
            let imgs = p["image_urls"].as_array().expect("image_urls");
            assert_eq!(imgs.len(), 2);
            assert_eq!(imgs[0], "https://cdn/d1.jpg");
            assert_eq!(imgs[1], "https://cdn/d2.jpg");
        });
    }

    #[test]
    fn brand_product_no_match_returns_local_err() {
        let body = r#"{ "products": [ { "name": "Better Sweater" } ] }"#;
        let dir = mock_adalign(body, 0);
        with_mock_path(&dir, || {
            let r = BrandProduct.dispatch(&json!({ "domain": "x.com", "query": "zzz nothing" }));
            assert!(!r.ok);
            assert!(r.summary.contains("no product matched"), "got: {}", r.summary);
        });
    }

    #[test]
    fn brand_ads_wraps_array_under_ads_key() {
        let body = r#"[ { "id": "1", "headline": "hi" }, { "id": "2", "headline": "ho" } ]"#;
        let dir = mock_adalign(body, 0);
        with_mock_path(&dir, || {
            let r = BrandAds.dispatch(&json!({ "domain": "patagonia.com", "limit": 2 }));
            assert!(r.ok, "summary: {}", r.summary);
            let ads = r.response["ads"].as_array().expect("ads array");
            assert_eq!(ads.len(), 2);
            assert_eq!(ads[0]["id"], "1");
        });
    }

    #[test]
    fn brand_ads_auth_envelope_routes_to_local_err() {
        let body = r#"{ "error": true, "message": "not signed in: run `adalign login`" }"#;
        let dir = mock_adalign(body, 0);
        with_mock_path(&dir, || {
            let r = BrandAds.dispatch(&json!({ "domain": "x.com" }));
            assert!(!r.ok);
            assert!(r.summary.contains("not signed in"), "got: {}", r.summary);
        });
    }

    #[test]
    fn brand_product_auth_envelope_routes_to_local_err() {
        let body = r#"{ "error": true, "message": "ADALIGN_API_KEY missing" }"#;
        let dir = mock_adalign(body, 0);
        with_mock_path(&dir, || {
            let r = BrandProduct.dispatch(&json!({ "domain": "x.com", "query": "any" }));
            assert!(!r.ok);
            assert!(r.summary.contains("ADALIGN_API_KEY"), "got: {}", r.summary);
        });
    }
}
