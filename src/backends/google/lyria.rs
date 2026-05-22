//! Google Lyria 3 adapter — `RefConditionedMusicGen` cluster, the
//! Google-direct default for tier=draft / tier=hero music in
//! `commercial.yaml`.
//!
//! ## Wire format (Google AI Studio, verified 2026-05-20)
//!
//! Both `lyria-3-pro-preview` and `lyria-3-clip-preview` run on the
//! generative-language API and only support `generateContent` — NOT
//! `predict` / `predictLongRunning`. They return audio synchronously
//! as inline data parts. The Vertex AI long-running shape this
//! backend originally targeted does not exist on AI Studio.
//!
//! ```text
//! POST https://generativelanguage.googleapis.com/v1beta/models/lyria-3-clip-preview:generateContent?key=…
//! {
//!   "contents":[{"parts":[{"text":"30 second gentle piano melody"}]}]
//! }
//! → {
//!     "candidates":[{ "content":{ "parts":[
//!         { "text": "<instrumental>" },
//!         { "inlineData": { "mimeType": "audio/mpeg", "data": "<base64-mp3>" } }
//!     ]}}]
//!   }
//! ```
//!
//! Audio is MP3 (`audio/mpeg`), already C2PA-signed by Google in the
//! stream. We do NOT transcode — if the user requested `track.wav`,
//! we write the MP3 bytes to that path; the `.wav` extension is
//! conventional and most decoders sniff the container.
//!
//! ## Cost
//!
//! Lyria 3 Pro preview is approximately $0.06/min generated, Lyria 3
//! Clip preview ~$0.024/min. We bill the per-second average
//! ($0.001/sec pro, $0.0004/sec clip) — keeps budget gates honest
//! without under-charging long arrangements.

use serde::{Deserialize, Serialize};

use crate::backends::cache::{utc_now_iso8601, AssetCache, Manifest};
use crate::backends::util::pick_audio_ext_from_mime;
use crate::backends::music::{
    MusicResult, RefConditionedMusicGenBackend, RefConditionedMusicRequest, CLUSTER_REF_COND,
};
use crate::backends::{
    check_budget, mode_label, BackendCallOutcome, BackendError, CostEstimate, RunMode,
};

use super::client::GoogleAiClient;

/// Pro-tier model name.
pub const MODEL_LYRIA_3_PRO: &str = "lyria-3-pro-preview";

/// Clip-tier model name.
pub const MODEL_LYRIA_3_CLIP: &str = "lyria-3-clip-preview";

/// Per-second cost estimate for Lyria 3 Pro (USD). ~$0.06/min.
pub const PRICE_PER_SEC_PRO_USD: f32 = 0.001;

/// Per-second cost estimate for Lyria 3 Clip (USD). ~$0.024/min.
pub const PRICE_PER_SEC_CLIP_USD: f32 = 0.0004;

/// Default pro-tier provider id.
pub const PROVIDER_PRO: &str = "google-lyria-3-pro";

/// Default clip-tier provider id.
pub const PROVIDER_CLIP: &str = "google-lyria-3-clip";

/// One of the two Lyria 3 model tiers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LyriaModel {
    /// Pro tier — `lyria-3-pro-preview`. Higher quality, slightly
    /// more expensive.
    Pro,
    /// Clip tier — `lyria-3-clip-preview`. Short, cheap.
    Clip,
}

impl LyriaModel {
    /// Model name used in the URL.
    pub fn name(self) -> &'static str {
        match self {
            LyriaModel::Pro => MODEL_LYRIA_3_PRO,
            LyriaModel::Clip => MODEL_LYRIA_3_CLIP,
        }
    }

    /// Per-second cost.
    pub fn price_per_sec(self) -> f32 {
        match self {
            LyriaModel::Pro => PRICE_PER_SEC_PRO_USD,
            LyriaModel::Clip => PRICE_PER_SEC_CLIP_USD,
        }
    }

    /// Provider id stored in manifests + cache keys.
    pub fn provider(self) -> &'static str {
        match self {
            LyriaModel::Pro => PROVIDER_PRO,
            LyriaModel::Clip => PROVIDER_CLIP,
        }
    }

    /// Parse from a backend-id alias.
    pub fn parse(s: &str) -> Result<Self, String> {
        match s {
            "lyria" | "lyria-pro" | "lyria-3-pro" | "pro" | MODEL_LYRIA_3_PRO | PROVIDER_PRO => {
                Ok(LyriaModel::Pro)
            }
            "lyria-clip" | "lyria-3-clip" | "clip" | MODEL_LYRIA_3_CLIP | PROVIDER_CLIP => {
                Ok(LyriaModel::Clip)
            }
            other => Err(format!(
                "unknown Lyria model '{other}'. want one of: \
                 lyria|lyria-pro|lyria-3-pro, lyria-clip|lyria-3-clip"
            )),
        }
    }
}

/// Google Lyria 3 adapter.
#[derive(Debug, Clone)]
pub struct GoogleLyriaAdapter {
    client: GoogleAiClient,
    model: LyriaModel,
}

impl GoogleLyriaAdapter {
    /// Build from a pre-constructed client + a chosen model tier.
    pub fn new(client: GoogleAiClient, model: LyriaModel) -> Self {
        Self { client, model }
    }
}

impl RefConditionedMusicGenBackend for GoogleLyriaAdapter {
    fn name(&self) -> &'static str {
        self.model.provider()
    }

    fn estimate_cost(&self, request: &RefConditionedMusicRequest) -> CostEstimate {
        let cost = request.duration_secs.max(0.0) * self.model.price_per_sec();
        CostEstimate {
            provider: self.model.provider().into(),
            cost_usd: cost,
            explanation: format!(
                "{duration:.1}s × ${rate:.4}/s = ${cost:.4} ({model})",
                duration = request.duration_secs,
                rate = self.model.price_per_sec(),
                cost = cost,
                model = self.model.name(),
            ),
        }
    }

    fn generate(
        &self,
        request: &RefConditionedMusicRequest,
        mode: RunMode,
    ) -> Result<BackendCallOutcome<MusicResult>, BackendError> {
        if request.prompt.trim().is_empty() {
            return Err(BackendError::InvalidRequest("prompt is empty".into()));
        }
        if request.duration_secs <= 0.0 {
            return Err(BackendError::InvalidRequest(
                "duration_secs must be > 0".into(),
            ));
        }

        let estimate = self.estimate_cost(request);
        check_budget(&estimate, mode)?;

        let provider = self.model.provider();
        let request_hash = AssetCache::request_hash(provider, CLUSTER_REF_COND, request)?;
        let cache = self.client.cache();

        if let Some(manifest) = cache.hit(provider, &request_hash)? {
            let response: MusicResult = serde_json::from_value(manifest.response.clone())
                .map_err(|e| BackendError::Cache(format!("decode cached response: {e}")))?;
            return Ok(BackendCallOutcome {
                response,
                provider: provider.into(),
                request_hash,
                cached: true,
                cost_estimate_usd: 0.0,
                mode: mode_label(mode),
            });
        }

        let prompt_sent = build_prompt(request);

        if !mode.is_live() {
            let response = MusicResult {
                provider: provider.into(),
                audio_path: cache.asset_path(provider, &request_hash, "mp3"),
                audio_bytes: 0,
                mime: "audio/mpeg".into(),
                model_variant: self.model.name().to_string(),
                prompt_sent,
                duration_secs: request.duration_secs,
            };
            return Ok(BackendCallOutcome {
                response,
                provider: provider.into(),
                request_hash,
                cached: false,
                cost_estimate_usd: estimate.cost_usd,
                mode: mode_label(mode),
            });
        }

        let body = build_body(&prompt_sent);
        let parsed: GenerateContentResponse =
            self.client.post_sync(self.model.name(), "generateContent", &body)?;
        let (bytes, mime) = extract_audio(&parsed)?;

        let ext = pick_audio_ext_from_mime(&mime);
        let audio_path = cache.write_asset(provider, &request_hash, ext, &bytes)?;
        let result = MusicResult {
            provider: provider.into(),
            audio_path: audio_path.clone(),
            audio_bytes: bytes.len() as u64,
            mime,
            model_variant: self.model.name().to_string(),
            prompt_sent,
            duration_secs: request.duration_secs,
        };

        let manifest = Manifest {
            version: 1,
            provider: provider.into(),
            cluster: CLUSTER_REF_COND.into(),
            request_hash: request_hash.clone(),
            request: serde_json::to_value(request)
                .map_err(|e| BackendError::Cache(format!("serialize request: {e}")))?,
            response: serde_json::to_value(&result)
                .map_err(|e| BackendError::Cache(format!("serialize response: {e}")))?,
            cost_estimate_usd: estimate.cost_usd,
            asset_path: Some(audio_path.display().to_string()),
            created_at: utc_now_iso8601(),
        };
        cache.store(&manifest)?;

        Ok(BackendCallOutcome {
            response: result,
            provider: provider.into(),
            request_hash,
            cached: false,
            cost_estimate_usd: estimate.cost_usd,
            mode: mode_label(mode),
        })
    }
}

fn build_prompt(request: &RefConditionedMusicRequest) -> String {
    let with_bpm = match request.bpm {
        Some(bpm) if !request.prompt.to_ascii_lowercase().contains("bpm") => {
            format!("{} ({:.0} bpm)", request.prompt, bpm)
        }
        _ => request.prompt.clone(),
    };
    // Lyria takes a plain text prompt — the duration hint is best
    // expressed inside the prompt itself because generateContent
    // doesn't have a structured `duration_seconds` field.
    let dur = request.duration_secs.max(1.0).round() as u32;
    if with_bpm.to_ascii_lowercase().contains("second") {
        with_bpm
    } else {
        format!("{dur} second {with_bpm}")
    }
}

fn build_body(prompt: &str) -> GenerateContentBody {
    GenerateContentBody {
        contents: vec![Content {
            parts: vec![Part {
                text: prompt.to_string(),
            }],
        }],
    }
}

fn extract_audio(resp: &GenerateContentResponse) -> Result<(Vec<u8>, String), BackendError> {
    use crate::backends::util::base64_decode;
    let candidate = resp
        .candidates
        .first()
        .ok_or_else(|| BackendError::Decode("lyria: empty candidates".into()))?;
    let mut audio_bytes: Vec<u8> = Vec::new();
    let mut mime: Option<String> = None;
    for part in &candidate.content.parts {
        if let Some(inline) = &part.inline_data {
            if inline.mime_type.starts_with("audio/") {
                let bytes = base64_decode(&inline.data).map_err(|e| {
                    BackendError::Decode(format!("lyria decode inline audio base64: {e}"))
                })?;
                audio_bytes.extend_from_slice(&bytes);
                if mime.is_none() {
                    mime = Some(inline.mime_type.clone());
                }
            }
        }
    }
    if audio_bytes.is_empty() {
        return Err(BackendError::Decode(
            "lyria: no audio inlineData parts in response".into(),
        ));
    }
    Ok((audio_bytes, mime.unwrap_or_else(|| "audio/mpeg".into())))
}

#[derive(Debug, Serialize)]
struct GenerateContentBody {
    contents: Vec<Content>,
}

#[derive(Debug, Serialize)]
struct Content {
    parts: Vec<Part>,
}

#[derive(Debug, Serialize)]
struct Part {
    text: String,
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
    parts: Vec<ResponsePart>,
}

#[derive(Debug, Deserialize)]
struct ResponsePart {
    #[serde(default, rename = "inlineData", alias = "inline_data")]
    inline_data: Option<InlineData>,
}

#[derive(Debug, Deserialize)]
struct InlineData {
    #[serde(rename = "mimeType", alias = "mime_type")]
    mime_type: String,
    data: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fresh_cache() -> std::path::PathBuf {
        let tmp = std::env::temp_dir().join(format!(
            "wavelet-google-lyria-{}",
            AssetCache::request_hash("seed", "seed", &"x").unwrap()
        ));
        let _ = std::fs::remove_dir_all(&tmp);
        tmp
    }

    fn stub_client() -> GoogleAiClient {
        GoogleAiClient::with_key("test-key", fresh_cache())
    }

    #[test]
    fn model_parse_aliases() {
        assert_eq!(LyriaModel::parse("lyria").unwrap(), LyriaModel::Pro);
        assert_eq!(LyriaModel::parse("lyria-pro").unwrap(), LyriaModel::Pro);
        assert_eq!(LyriaModel::parse("google-lyria-3-pro").unwrap(), LyriaModel::Pro);
        assert_eq!(LyriaModel::parse("clip").unwrap(), LyriaModel::Clip);
        assert_eq!(LyriaModel::parse("lyria-3-clip").unwrap(), LyriaModel::Clip);
        assert!(LyriaModel::parse("nope").is_err());
    }

    #[test]
    fn cost_scales_with_duration_and_tier() {
        let req = RefConditionedMusicRequest::new("ambient strings", 30.0);
        let pro = GoogleLyriaAdapter::new(stub_client(), LyriaModel::Pro).estimate_cost(&req);
        let clip = GoogleLyriaAdapter::new(stub_client(), LyriaModel::Clip).estimate_cost(&req);
        assert!(pro.cost_usd > clip.cost_usd);
        assert!((pro.cost_usd - 30.0 * PRICE_PER_SEC_PRO_USD).abs() < 1e-5);
        assert!((clip.cost_usd - 30.0 * PRICE_PER_SEC_CLIP_USD).abs() < 1e-5);
    }

    #[test]
    fn empty_prompt_rejected() {
        let adapter = GoogleLyriaAdapter::new(stub_client(), LyriaModel::Pro);
        let req = RefConditionedMusicRequest::new("   ", 5.0);
        assert!(matches!(
            adapter.generate(&req, RunMode::DryRun).unwrap_err(),
            BackendError::InvalidRequest(_)
        ));
    }

    #[test]
    fn zero_duration_rejected() {
        let adapter = GoogleLyriaAdapter::new(stub_client(), LyriaModel::Pro);
        let req = RefConditionedMusicRequest::new("ambient", 0.0);
        assert!(matches!(
            adapter.generate(&req, RunMode::DryRun).unwrap_err(),
            BackendError::InvalidRequest(_)
        ));
    }

    #[test]
    fn dry_run_emits_request_shape_no_network() {
        let adapter = GoogleLyriaAdapter::new(stub_client(), LyriaModel::Pro);
        let mut req = RefConditionedMusicRequest::new("cinematic strings", 8.0);
        req.bpm = Some(90.0);
        let out = adapter.generate(&req, RunMode::DryRun).unwrap();
        assert_eq!(out.mode, "dry-run");
        assert_eq!(out.response.provider, PROVIDER_PRO);
        assert_eq!(out.response.audio_bytes, 0);
        assert_eq!(out.response.model_variant, MODEL_LYRIA_3_PRO);
        assert!(out.response.prompt_sent.contains("90 bpm"));
    }

    #[test]
    fn build_prompt_adds_bpm_and_duration() {
        let mut req = RefConditionedMusicRequest::new("ambient strings", 5.0);
        req.bpm = Some(110.0);
        let p = build_prompt(&req);
        assert!(p.contains("110 bpm"));
        assert!(p.starts_with("5 second"));
    }

    #[test]
    fn build_prompt_preserves_existing_bpm_and_duration_word() {
        let mut req = RefConditionedMusicRequest::new("driving 130 BPM 12 second techno", 12.0);
        req.bpm = Some(120.0);
        let p = build_prompt(&req);
        assert!(!p.starts_with("12 second"), "should not double-prefix duration: {p}");
        assert!(p.contains("130 BPM"));
    }

    #[test]
    fn body_shape_matches_generate_content_wire() {
        let body = build_body("ambient strings");
        let v = serde_json::to_value(&body).unwrap();
        assert_eq!(v["contents"][0]["parts"][0]["text"], "ambient strings");
    }


    #[test]
    fn extract_audio_collects_audio_parts() {
        // Mirrors a real generateContent response — one text part, one audio part.
        let body = r#"{
            "candidates": [{
                "content": {
                    "parts": [
                        { "text": "<instrumental>" },
                        { "inlineData": { "mimeType": "audio/mpeg", "data": "aGVsbG8=" } }
                    ]
                }
            }]
        }"#;
        let parsed: GenerateContentResponse = serde_json::from_str(body).unwrap();
        let (bytes, mime) = extract_audio(&parsed).unwrap();
        assert_eq!(bytes, b"hello");
        assert_eq!(mime, "audio/mpeg");
    }

    #[test]
    fn extract_audio_concatenates_multiple_audio_parts() {
        let body = r#"{
            "candidates": [{
                "content": {
                    "parts": [
                        { "inlineData": { "mimeType": "audio/mpeg", "data": "aGVsbG8=" } },
                        { "inlineData": { "mimeType": "audio/mpeg", "data": "d29ybGQ=" } }
                    ]
                }
            }]
        }"#;
        let parsed: GenerateContentResponse = serde_json::from_str(body).unwrap();
        let (bytes, _) = extract_audio(&parsed).unwrap();
        assert_eq!(bytes, b"helloworld");
    }

    #[test]
    fn extract_audio_errors_when_no_audio_part() {
        let body = r#"{
            "candidates": [{
                "content": { "parts": [ { "text": "no audio here" } ] }
            }]
        }"#;
        let parsed: GenerateContentResponse = serde_json::from_str(body).unwrap();
        assert!(matches!(
            extract_audio(&parsed).unwrap_err(),
            BackendError::Decode(_)
        ));
    }

    #[test]
    fn over_budget_request_is_rejected() {
        let adapter = GoogleLyriaAdapter::new(stub_client(), LyriaModel::Pro);
        let req = RefConditionedMusicRequest::new("ambient", 60.0);
        let err = adapter
            .generate(&req, RunMode::Live { max_cost_usd: 0.001 })
            .unwrap_err();
        assert!(matches!(err, BackendError::OverBudget { .. }));
    }
}
