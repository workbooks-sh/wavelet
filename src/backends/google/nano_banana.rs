//! Google Nano Banana 3 (Gemini 3 Pro Image) adapter —
//! `RefConditionedImgGen` cluster, the **Google-direct** default for
//! tier=draft (still) and tier=hero (still) in `commercial.yaml`.
//!
//! ## Wire format (probed against Google AI Studio docs 2026-05-19)
//!
//! ```text
//! POST https://generativelanguage.googleapis.com/v1beta/models/gemini-3-pro-image-preview:generateContent?key=…
//! {
//!   "contents": [{
//!     "parts": [
//!       { "text": "<scene prompt>" },
//!       { "inline_data": { "mime_type": "image/jpeg", "data": "<base64>" } },
//!       …
//!     ]
//!   }],
//!   "generationConfig": { "responseModalities": ["IMAGE"] }
//! }
//! → { "candidates": [{ "content": { "parts": [
//!         { "inline_data": { "mime_type": "image/png", "data": "<base64 PNG>" } }
//!       ]}}]}
//! ```
//!
//! Reference photos go in as additional `inline_data` parts on the same
//! `contents[0].parts` list. The `responseModalities` field is required
//! to coerce image output (omitting it makes the model reply in text).
//!
//! ## Cost
//!
//! Google bills Nano Banana 3 by image tier; the published preview rate
//! is ~$0.04 per image at standard resolution. We charge the standard
//! rate flat for budget gating — the actual response carries no token
//! breakdown to bill per-megapixel.

use serde::{Deserialize, Serialize};
use crate::backends::util::pick_image_ext_from_mime;

use crate::backends::cache::{utc_now_iso8601, AssetCache, Manifest};
use crate::backends::image::{
    ImageResult, RefConditionedImgGenBackend, RefConditionedImgRequest, CLUSTER_REF_IMG_GEN,
};
use crate::backends::{
    check_budget, mode_label, BackendCallOutcome, BackendError, CostEstimate, RunMode,
};

use super::client::GoogleAiClient;

/// Provider id stored in manifests + cache keys.
pub const PROVIDER: &str = "google-nano-banana-3";

/// Model name on the generative-language API.
pub const MODEL: &str = "gemini-3-pro-image-preview";

/// Per-image cost estimate (USD). Google's preview rate for Nano Banana
/// 3 hovers at $0.04/image at standard resolution.
pub const PRICE_PER_CALL_USD: f32 = 0.04;

/// Server-enforced max reference images. Probed 2026-05-19: Gemini 3 Pro
/// Image accepts at least 8 inline image parts per request, refuses more.
pub const MAX_REF_IMAGES: usize = 8;

/// Google Nano Banana 3 image-gen adapter.
#[derive(Debug, Clone)]
pub struct GoogleNanoBanana3Adapter {
    client: GoogleAiClient,
}

impl GoogleNanoBanana3Adapter {
    /// Build from a pre-constructed client.
    pub fn new(client: GoogleAiClient) -> Self {
        Self { client }
    }
}

impl RefConditionedImgGenBackend for GoogleNanoBanana3Adapter {
    fn name(&self) -> &'static str {
        PROVIDER
    }

    fn estimate_cost(&self, _: &RefConditionedImgRequest) -> CostEstimate {
        CostEstimate {
            provider: PROVIDER.into(),
            cost_usd: PRICE_PER_CALL_USD,
            explanation: format!(
                "${PRICE_PER_CALL_USD:.4}/image ({MODEL}, Google-direct)"
            ),
        }
    }

    fn generate(
        &self,
        request: &RefConditionedImgRequest,
        mode: RunMode,
    ) -> Result<BackendCallOutcome<ImageResult>, BackendError> {
        if request.prompt.trim().len() < 3 {
            return Err(BackendError::InvalidRequest(
                "prompt must be at least 3 chars".into(),
            ));
        }
        if request.image_urls.len() > MAX_REF_IMAGES {
            return Err(BackendError::InvalidRequest(format!(
                "image_urls has {} entries, max {MAX_REF_IMAGES}",
                request.image_urls.len()
            )));
        }
        for url in &request.image_urls {
            if url.trim().is_empty() {
                return Err(BackendError::InvalidRequest(
                    "image_urls contains an empty entry".into(),
                ));
            }
        }

        let estimate = self.estimate_cost(request);
        check_budget(&estimate, mode)?;

        let request_hash = AssetCache::request_hash(PROVIDER, CLUSTER_REF_IMG_GEN, request)?;
        let cache = self.client.cache();

        if let Some(manifest) = cache.hit(PROVIDER, &request_hash)? {
            let response: ImageResult = serde_json::from_value(manifest.response.clone())
                .map_err(|e| BackendError::Cache(format!("decode cached response: {e}")))?;
            return Ok(BackendCallOutcome {
                response,
                provider: PROVIDER.into(),
                request_hash,
                cached: true,
                cost_estimate_usd: 0.0,
                mode: mode_label(mode),
            });
        }

        if !mode.is_live() {
            let response = ImageResult {
                provider: PROVIDER.into(),
                image_path: cache.asset_path(PROVIDER, &request_hash, "png"),
                image_bytes: 0,
                width: 0,
                height: 0,
                mime: "image/png".into(),
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

        let body = build_body(request)?;
        let parsed: GenerateContentResponse =
            self.client.post_sync(MODEL, "generateContent", &body)?;
        let (image_bytes_raw, mime) = extract_image(&parsed)?;
        let ext = pick_image_ext_from_mime(Some(&mime));
        let image_path = cache.write_asset(PROVIDER, &request_hash, ext, &image_bytes_raw)?;
        let image_bytes = image_bytes_raw.len() as u64;

        let result = ImageResult {
            provider: PROVIDER.into(),
            image_path: image_path.clone(),
            image_bytes,
            width: 0,
            height: 0,
            mime,
        };

        let manifest = Manifest {
            version: 1,
            provider: PROVIDER.into(),
            cluster: CLUSTER_REF_IMG_GEN.into(),
            request_hash: request_hash.clone(),
            request: serde_json::to_value(request)
                .map_err(|e| BackendError::Cache(format!("serialize request: {e}")))?,
            response: serde_json::to_value(&result)
                .map_err(|e| BackendError::Cache(format!("serialize response: {e}")))?,
            cost_estimate_usd: estimate.cost_usd,
            asset_path: Some(image_path.display().to_string()),
            created_at: utc_now_iso8601(),
        };
        cache.store(&manifest)?;

        Ok(BackendCallOutcome {
            response: result,
            provider: PROVIDER.into(),
            request_hash,
            cached: false,
            cost_estimate_usd: estimate.cost_usd,
            mode: mode_label(mode),
        })
    }
}

fn build_body(request: &RefConditionedImgRequest) -> Result<GenerateContentBody, BackendError> {
    let mut parts: Vec<Part> = Vec::with_capacity(1 + request.image_urls.len());
    let prompt = compose_prompt(request);
    parts.push(Part::Text { text: prompt });
    for url in &request.image_urls {
        let part = ref_to_inline_part(url)?;
        parts.push(part);
    }
    Ok(GenerateContentBody {
        contents: vec![Content { parts }],
        generation_config: GenerationConfig {
            response_modalities: vec!["IMAGE".into()],
        },
    })
}

/// Compose the textual prompt the model receives. Aspect-ratio hint is
/// folded into the text because the `generateContent` body has no
/// structured aspect field.
fn compose_prompt(request: &RefConditionedImgRequest) -> String {
    let aspect_hint = aspect_hint_for(&request.image_size);
    if aspect_hint.is_empty() {
        request.prompt.clone()
    } else {
        format!("{} (aspect ratio: {aspect_hint})", request.prompt)
    }
}

fn aspect_hint_for(size: &str) -> &'static str {
    match size {
        "landscape_16_9" => "16:9",
        "portrait_9_16" => "9:16",
        "landscape_4_3" => "4:3",
        "portrait_4_3" => "3:4",
        "square" | "square_hd" => "1:1",
        _ => "",
    }
}

/// Resolve a ref entry (local path, `data:` URL, or `https://…` URL) into
/// an `inline_data` part. Remote URLs are fetched and inlined since
/// Gemini's `generateContent` only accepts inline bytes for image parts
/// (the Files API is a separate path the agent uses for the rubric judge).
fn ref_to_inline_part(source: &str) -> Result<Part, BackendError> {
    use crate::backends::util::{base64_encode, ext_to_mime, sniff_image_ext};
    if source.starts_with("data:") {
        let rest = &source[5..];
        let (header, b64) = rest.split_once(',').ok_or_else(|| {
            BackendError::InvalidRequest("malformed data: URL (no comma)".into())
        })?;
        let mime = header
            .split(';')
            .next()
            .unwrap_or("image/png")
            .to_string();
        return Ok(Part::InlineData {
            inline_data: InlineData {
                mime_type: mime,
                data: b64.to_string(),
            },
        });
    }
    if source.starts_with("http://") || source.starts_with("https://") {
        let bytes = fetch_url_bytes(source)?;
        let ext = sniff_image_ext(&bytes);
        return Ok(Part::InlineData {
            inline_data: InlineData {
                mime_type: ext_to_mime(ext).into(),
                data: base64_encode(&bytes),
            },
        });
    }
    let path = std::path::Path::new(source);
    let bytes = std::fs::read(path)
        .map_err(|e| BackendError::InvalidRequest(format!("read image {}: {e}", path.display())))?;
    if bytes.is_empty() {
        return Err(BackendError::InvalidRequest(format!(
            "image file {} is empty",
            path.display()
        )));
    }
    let ext = sniff_image_ext(&bytes);
    Ok(Part::InlineData {
        inline_data: InlineData {
            mime_type: ext_to_mime(ext).into(),
            data: base64_encode(&bytes),
        },
    })
}

fn fetch_url_bytes(url: &str) -> Result<Vec<u8>, BackendError> {
    use std::io::Read;
    let resp = ureq::get(url)
        .call()
        .map_err(|e| BackendError::Transport(format!("fetch ref {url}: {e}")))?;
    let mut buf = Vec::new();
    resp.into_reader()
        .read_to_end(&mut buf)
        .map_err(|e| BackendError::Transport(format!("read ref body {url}: {e}")))?;
    if buf.is_empty() {
        return Err(BackendError::Transport(format!(
            "ref {url} returned 0 bytes"
        )));
    }
    Ok(buf)
}

fn extract_image(resp: &GenerateContentResponse) -> Result<(Vec<u8>, String), BackendError> {
    use crate::backends::util::base64_decode;
    let cand = resp.candidates.first().ok_or_else(|| {
        BackendError::Decode("nano-banana-3: no candidates in response".into())
    })?;
    for part in &cand.content.parts {
        if let CandidatePart::InlineData { inline_data } = part {
            let bytes = base64_decode(&inline_data.data).map_err(|e| {
                BackendError::Decode(format!("decode candidate base64: {e}"))
            })?;
            return Ok((bytes, inline_data.mime_type.clone()));
        }
    }
    Err(BackendError::Decode(
        "nano-banana-3: no inline_data image part in candidate".into(),
    ))
}

#[derive(Debug, Serialize)]
struct GenerateContentBody {
    contents: Vec<Content>,
    #[serde(rename = "generationConfig")]
    generation_config: GenerationConfig,
}

#[derive(Debug, Serialize)]
struct Content {
    parts: Vec<Part>,
}

#[derive(Debug, Serialize)]
#[serde(untagged)]
enum Part {
    Text {
        text: String,
    },
    InlineData {
        #[serde(rename = "inline_data")]
        inline_data: InlineData,
    },
}

#[derive(Debug, Serialize)]
struct InlineData {
    #[serde(rename = "mime_type")]
    mime_type: String,
    data: String,
}

#[derive(Debug, Serialize)]
struct GenerationConfig {
    #[serde(rename = "responseModalities")]
    response_modalities: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct GenerateContentResponse {
    #[serde(default)]
    candidates: Vec<Candidate>,
}

#[derive(Debug, Deserialize)]
struct Candidate {
    content: CandidateContent,
}

#[derive(Debug, Deserialize)]
struct CandidateContent {
    #[serde(default)]
    parts: Vec<CandidatePart>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum CandidatePart {
    InlineData {
        #[serde(rename = "inline_data", alias = "inlineData")]
        inline_data: CandidateInlineData,
    },
    /// Catch-all for `text` / `function_call` parts the model might emit
    /// alongside the image — discarded by `extract_image`. Kept so deserialize
    /// doesn't fail on extra parts.
    Other(#[allow(dead_code)] serde_json::Value),
}

#[derive(Debug, Deserialize)]
struct CandidateInlineData {
    #[serde(rename = "mime_type", alias = "mimeType")]
    mime_type: String,
    data: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fresh_cache() -> std::path::PathBuf {
        let tmp = std::env::temp_dir().join(format!(
            "wavelet-google-nb3-{}",
            AssetCache::request_hash("seed", "seed", &"x").unwrap()
        ));
        let _ = std::fs::remove_dir_all(&tmp);
        tmp
    }

    fn stub_client() -> GoogleAiClient {
        GoogleAiClient::with_key("test-key", fresh_cache())
    }

    fn sample_req() -> RefConditionedImgRequest {
        RefConditionedImgRequest::new(
            "a green Porsche 911 GT3 on a coastal cliff at golden hour",
            vec![],
        )
    }

    #[test]
    fn provider_id_is_namespaced() {
        assert_eq!(PROVIDER, "google-nano-banana-3");
    }

    #[test]
    fn cost_estimate_is_flat_per_image() {
        let adapter = GoogleNanoBanana3Adapter::new(stub_client());
        let est = adapter.estimate_cost(&sample_req());
        assert!((est.cost_usd - PRICE_PER_CALL_USD).abs() < f32::EPSILON);
        assert!(est.explanation.contains("Google-direct"));
    }

    #[test]
    fn empty_prompt_rejected() {
        let adapter = GoogleNanoBanana3Adapter::new(stub_client());
        let mut req = sample_req();
        req.prompt = "  ".into();
        assert!(matches!(
            adapter.generate(&req, RunMode::DryRun).unwrap_err(),
            BackendError::InvalidRequest(_)
        ));
    }

    #[test]
    fn too_many_refs_rejected() {
        let adapter = GoogleNanoBanana3Adapter::new(stub_client());
        let refs: Vec<String> = (0..(MAX_REF_IMAGES + 1))
            .map(|i| format!("https://example.com/{i}.png"))
            .collect();
        let req = RefConditionedImgRequest::new("a car at dusk on a coastal road", refs);
        assert!(matches!(
            adapter.generate(&req, RunMode::DryRun).unwrap_err(),
            BackendError::InvalidRequest(_)
        ));
    }

    #[test]
    fn dry_run_emits_request_shape_no_network() {
        let adapter = GoogleNanoBanana3Adapter::new(stub_client());
        let out = adapter.generate(&sample_req(), RunMode::DryRun).unwrap();
        assert_eq!(out.mode, "dry-run");
        assert_eq!(out.response.provider, PROVIDER);
        assert_eq!(out.response.image_bytes, 0);
        assert!(out.response.image_path.to_string_lossy().ends_with(".png"));
    }

    #[test]
    fn body_shape_matches_google_wire_text_only() {
        let req = RefConditionedImgRequest::new(
            "a saguaro at sunset, low angle wide shot",
            vec![],
        );
        let body = build_body(&req).unwrap();
        let v = serde_json::to_value(&body).unwrap();
        assert_eq!(v["contents"][0]["parts"][0]["text"], "a saguaro at sunset, low angle wide shot (aspect ratio: 16:9)");
        assert_eq!(v["generationConfig"]["responseModalities"][0], "IMAGE");
    }

    #[test]
    fn body_shape_includes_inline_data_parts_for_data_url_refs() {
        let png_b64 = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNkYAAAAAYAAjCB0C8AAAAASUVORK5CYII=";
        let data_url = format!("data:image/png;base64,{png_b64}");
        let req = RefConditionedImgRequest::new(
            "place the product into the scene with strong rim lighting",
            vec![data_url.clone()],
        );
        let body = build_body(&req).unwrap();
        let v = serde_json::to_value(&body).unwrap();
        let parts = v["contents"][0]["parts"].as_array().unwrap();
        assert_eq!(parts.len(), 2, "text + 1 ref");
        assert_eq!(parts[1]["inline_data"]["mime_type"], "image/png");
        assert_eq!(parts[1]["inline_data"]["data"], png_b64);
    }

    #[test]
    fn aspect_hint_maps_known_sizes() {
        assert_eq!(aspect_hint_for("landscape_16_9"), "16:9");
        assert_eq!(aspect_hint_for("portrait_9_16"), "9:16");
        assert_eq!(aspect_hint_for("square"), "1:1");
        assert_eq!(aspect_hint_for("square_hd"), "1:1");
        assert_eq!(aspect_hint_for("anything_else"), "");
    }

    #[test]
    fn over_budget_request_is_rejected() {
        let adapter = GoogleNanoBanana3Adapter::new(stub_client());
        let err = adapter
            .generate(&sample_req(), RunMode::Live { max_cost_usd: 0.001 })
            .unwrap_err();
        match err {
            BackendError::OverBudget { estimate, budget } => {
                assert!((estimate - PRICE_PER_CALL_USD).abs() < 1e-6);
                assert!((budget - 0.001).abs() < 1e-6);
            }
            other => panic!("expected OverBudget, got {other:?}"),
        }
    }

    #[test]
    fn extract_image_decodes_inline_data() {
        let body = r#"{
            "candidates": [{
                "content": {
                    "parts": [
                        { "inline_data": { "mime_type": "image/png", "data": "aGVsbG8=" } }
                    ]
                }
            }]
        }"#;
        let parsed: GenerateContentResponse = serde_json::from_str(body).unwrap();
        let (bytes, mime) = extract_image(&parsed).unwrap();
        assert_eq!(bytes, b"hello");
        assert_eq!(mime, "image/png");
    }

    #[test]
    fn extract_image_handles_camel_case_keys() {
        // Google sometimes returns camelCase. Both spellings must decode.
        let body = r#"{
            "candidates": [{
                "content": {
                    "parts": [
                        { "inlineData": { "mimeType": "image/jpeg", "data": "aGVsbG8=" } }
                    ]
                }
            }]
        }"#;
        let parsed: GenerateContentResponse = serde_json::from_str(body).unwrap();
        let (bytes, mime) = extract_image(&parsed).unwrap();
        assert_eq!(bytes, b"hello");
        assert_eq!(mime, "image/jpeg");
    }
}
