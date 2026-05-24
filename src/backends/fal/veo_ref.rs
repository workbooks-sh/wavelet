//! Fal-hosted Veo 3.1 reference-to-video adapter — `MultiRefVideoGen` cluster.
//!
//! Fal exposes `fal-ai/veo3.1/reference-to-video` (Standard) and
//! `fal-ai/veo3.1/fast/reference-to-video` (Fast) which condition video
//! generation on 1–3 reference images. The model maintains subject /
//! identity anchoring across the generated clip — the advertised
//! use-case is character-consistent UGC: same face, same wardrobe,
//! same hands across multiple cuts.
//!
//! NOTE on the fast path: it's `veo3.1/fast/...` (slash) not
//! `veo3.1-fast/...` (dash). Earlier versions of this adapter shipped
//! the wrong path and every Fast call 404'd — see eval 010 trace from
//! 2026-05-23 for the regression that prompted the fix.
//!
//! ## Wire protocol
//!
//! ```text
//! POST https://queue.fal.run/fal-ai/veo3.1/reference-to-video
//! Authorization: Key <FAL_KEY>
//! {
//!   "prompt": "…",
//!   "image_urls": ["https://…/ref1.png", "https://…/ref2.png"],
//!   "aspect_ratio": "9:16",
//!   "duration": "8s",
//!   "resolution": "720p",
//!   "generate_audio": true
//! }
//!
//! → same queue/poll/response_url shape as fal-ai/veo3.
//! ```
//!
//! Local reference files are uploaded to fal-storage via
//! `FalClient::upload_bytes` before the generation call. Remote HTTPS
//! URLs are forwarded directly.
//!
//! ## Pricing (Fal, probed 2026-05-23 — VERIFY before raising budget gates)
//!
//! - Standard (`fal-ai/veo3.1/reference-to-video`): ~$0.50/s
//! - Fast (`fal-ai/veo3.1/fast/reference-to-video`): ~$0.25/s
//!
//! Verify at <https://fal.ai/models/fal-ai/veo3.1/reference-to-video>
//! and <https://fal.ai/pricing> before deploying — preview pricing may
//! change as Veo 3.1 exits preview.
//!
//! ## Accepted durations
//!
//! Quantized identically to Veo 3: `"4s"` or `"8s"`. Values ≤ 5 s map
//! to `"4s"`; values > 5 s map to `"8s"`.
//!
//! ## Reference image count
//!
//! Google's Gemini docs cap Veo 3.1 ref-to-video at **3** reference
//! images ("Provide up to three reference images to guide your
//! generated video's content."
//! <https://ai.google.dev/gemini-api/docs/video>). Fal's hosted endpoint
//! inherits the same limit. The adapter validates this constraint at
//! submission time, not at struct construction — so multi-shot
//! pipelines can accumulate refs before calling generate.

use std::path::Path;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::backends::cache::{utc_now_iso8601, AssetCache, Manifest};
use crate::backends::fal::FalClient;
use crate::backends::http_client::FAL_KEY_ENV;
use crate::backends::video::{MultiRefVideoGenBackend, MultiRefVideoRequest, VideoResult, CLUSTER_MULTI_REF_VIDEO};
use crate::backends::{
    check_budget, mode_label, BackendCallOutcome, BackendError, CostEstimate, RunMode,
};

use super::veo::{POLL_INTERVAL_SECS, POLL_TIMEOUT_SECS, QUEUE_BASE};

/// Maximum reference images the endpoint accepts.
///
/// Google's Gemini docs cap this at 3
/// (<https://ai.google.dev/gemini-api/docs/video>). Fal's hosted
/// endpoint inherits the same limit.
pub const MAX_REFERENCES: usize = 3;

/// Fal model path for Veo 3.1 reference-to-video Standard tier.
pub const MODEL_PATH_STANDARD: &str = "fal-ai/veo3.1/reference-to-video";

/// Fal model path for Veo 3.1 reference-to-video Fast tier.
///
/// Path is `veo3.1/fast/reference-to-video` (slash, not dash) — see
/// <https://fal.ai/models/fal-ai/veo3.1/fast/reference-to-video>. The
/// dash form `veo3.1-fast/reference-to-video` 404s.
pub const MODEL_PATH_FAST: &str = "fal-ai/veo3.1/fast/reference-to-video";

/// Per-second price for Standard tier in USD.
/// Matches Fal's Veo 3.1 ref-to-video preview pricing (~$0.50/s).
/// VERIFY against <https://fal.ai/pricing> before raising budget gates.
pub const PRICE_PER_SECOND_STANDARD_USD: f32 = 0.50;

/// Per-second price for Fast tier in USD (~$0.25/s).
/// VERIFY against <https://fal.ai/pricing> before raising budget gates.
pub const PRICE_PER_SECOND_FAST_USD: f32 = 0.25;

/// The two Fal-hosted Veo 3.1 reference-to-video model tiers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FalVeoRefModel {
    /// Full Veo 3.1 Standard — higher quality, ~$0.50/s.
    Standard,
    /// Veo 3.1 Fast — lower latency, ~$0.25/s.
    Fast,
}

impl FalVeoRefModel {
    /// Fal queue path for the model (appended to `QUEUE_BASE`).
    pub fn path(self) -> &'static str {
        match self {
            FalVeoRefModel::Standard => MODEL_PATH_STANDARD,
            FalVeoRefModel::Fast => MODEL_PATH_FAST,
        }
    }

    /// Per-second cost in USD.
    pub fn price_per_second(self) -> f32 {
        match self {
            FalVeoRefModel::Standard => PRICE_PER_SECOND_STANDARD_USD,
            FalVeoRefModel::Fast => PRICE_PER_SECOND_FAST_USD,
        }
    }

    /// Provider identifier used in cache keys and manifests.
    pub fn provider(self) -> &'static str {
        match self {
            FalVeoRefModel::Standard => "fal-veo3-ref-standard",
            FalVeoRefModel::Fast => "fal-veo3-ref-fast",
        }
    }

    /// Parse from a backend-name string.
    pub fn parse(s: &str) -> Result<Self, String> {
        match s {
            "fal-veo3-ref"
            | "fal-veo3.1-ref"
            | "fal-veo3-ref-standard"
            | "fal-veo3.1-ref-standard"
            | MODEL_PATH_STANDARD => Ok(FalVeoRefModel::Standard),
            "fal-veo3-ref-fast"
            | "fal-veo3.1-ref-fast"
            | MODEL_PATH_FAST => Ok(FalVeoRefModel::Fast),
            other => Err(format!(
                "unknown Fal Veo ref-to-video model '{other}'. want one of: \
                 fal-veo3-ref|fal-veo3.1-ref|fal-veo3-ref-standard (Standard), \
                 fal-veo3-ref-fast|fal-veo3.1-ref-fast (Fast)"
            )),
        }
    }
}

/// Fal-hosted Veo 3.1 reference-to-video adapter.
///
/// Implements `MultiRefVideoGenBackend`. Reference images in
/// `MultiRefVideoRequest::reference_images` may be:
/// - Local filesystem paths (`/path/to/ref.png`) — uploaded to
///   fal-storage before submission.
/// - HTTPS URLs — forwarded directly to the model.
#[derive(Debug, Clone)]
pub struct FalVeoRefAdapter {
    client: FalClient,
    model: FalVeoRefModel,
    api_key: String,
}

impl FalVeoRefAdapter {
    /// Build from a pre-constructed client + model variant + API key.
    pub fn new(client: FalClient, model: FalVeoRefModel, api_key: impl Into<String>) -> Self {
        Self {
            client,
            model,
            api_key: api_key.into(),
        }
    }

    /// Build from `FAL_KEY` environment variable.
    pub fn from_env(
        model: FalVeoRefModel,
        cache_root: impl Into<std::path::PathBuf>,
    ) -> Result<Self, BackendError> {
        let key = std::env::var(FAL_KEY_ENV)
            .map_err(|_| BackendError::MissingCredential(FAL_KEY_ENV.into()))?;
        if key.trim().is_empty() {
            return Err(BackendError::MissingCredential(FAL_KEY_ENV.into()));
        }
        let client = FalClient::with_key(&key, cache_root);
        Ok(Self::new(client, model, key))
    }

    /// Resolve a reference image string to a public URL.
    ///
    /// - HTTPS URLs → returned as-is.
    /// - Local paths → uploaded to fal-storage, returns the CDN URL.
    fn resolve_reference_url(&self, reference: &str) -> Result<String, BackendError> {
        if reference.starts_with("https://") || reference.starts_with("http://") {
            return Ok(reference.to_string());
        }
        let path = Path::new(reference);
        let bytes = std::fs::read(path).map_err(|e| {
            BackendError::InvalidRequest(format!(
                "read reference image '{}': {e}",
                path.display()
            ))
        })?;
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("png")
            .to_lowercase();
        let content_type = match ext.as_str() {
            "jpg" | "jpeg" => "image/jpeg",
            "webp" => "image/webp",
            "gif" => "image/gif",
            _ => "image/png",
        };
        let file_name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("ref.png");
        self.client
            .upload_bytes(&bytes, content_type, file_name)
    }
}

impl MultiRefVideoGenBackend for FalVeoRefAdapter {
    fn name(&self) -> &'static str {
        self.model.provider()
    }

    fn estimate_cost(&self, request: &MultiRefVideoRequest) -> CostEstimate {
        let cost = request.duration_secs * self.model.price_per_second();
        CostEstimate {
            provider: self.model.provider().into(),
            cost_usd: cost,
            explanation: format!(
                "{duration:.1}s × ${rate:.2}/s = ${cost:.4} ({path})",
                duration = request.duration_secs,
                rate = self.model.price_per_second(),
                cost = cost,
                path = self.model.path(),
            ),
        }
    }

    fn generate(
        &self,
        request: &MultiRefVideoRequest,
        mode: RunMode,
    ) -> Result<BackendCallOutcome<VideoResult>, BackendError> {
        if request.prompt.trim().is_empty() {
            return Err(BackendError::InvalidRequest("prompt is empty".into()));
        }
        if request.reference_images.is_empty() {
            return Err(BackendError::InvalidRequest(
                "ref-to-video requires at least one --reference image".into(),
            ));
        }
        if request.reference_images.len() > MAX_REFERENCES {
            return Err(BackendError::InvalidRequest(format!(
                "ref-to-video accepts at most {MAX_REFERENCES} reference images (passed {})",
                request.reference_images.len()
            )));
        }

        let estimate = <Self as MultiRefVideoGenBackend>::estimate_cost(self, request);
        check_budget(&estimate, mode)?;

        let provider = self.model.provider();
        let request_hash = AssetCache::request_hash(provider, CLUSTER_MULTI_REF_VIDEO, request)?;
        let cache = self.client.cache();

        if let Some(manifest) = cache.hit(provider, &request_hash)? {
            let response: VideoResult = serde_json::from_value(manifest.response.clone())
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

        if !mode.is_live() {
            let response = VideoResult {
                provider: provider.into(),
                video_path: cache.asset_path(provider, &request_hash, "mp4"),
                video_bytes: 0,
                duration_secs: request.duration_secs,
                width: 0,
                height: 0,
                mime: "video/mp4".into(),
                prompt_sent: request.prompt.clone(),
                seed_used: request.seed,
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

        fal_ref_queue_call(
            &self.api_key,
            self,
            self.model,
            request,
            &estimate,
            cache,
            provider,
            &request_hash,
            mode,
        )
    }
}

/// Submit → poll → download via Fal's queue API for ref-to-video.
#[allow(clippy::too_many_arguments)]
fn fal_ref_queue_call(
    api_key: &str,
    adapter: &FalVeoRefAdapter,
    model: FalVeoRefModel,
    request: &MultiRefVideoRequest,
    estimate: &CostEstimate,
    cache: &AssetCache,
    provider: &'static str,
    request_hash: &str,
    mode: RunMode,
) -> Result<BackendCallOutcome<VideoResult>, BackendError> {
    // Resolve all reference images to public URLs (upload locals).
    let image_urls: Vec<String> = request
        .reference_images
        .iter()
        .map(|r| adapter.resolve_reference_url(r))
        .collect::<Result<Vec<_>, _>>()?;

    let body = FalVeoRefBody::from_request(request, image_urls);

    // --- 1. Submit ---
    let submit_url = format!("{}/{}", QUEUE_BASE, model.path());
    let submit_resp: FalQueueSubmitResponse = fal_post(api_key, &submit_url, &body)?;

    // --- 2. Poll status_url until COMPLETED ---
    let deadline = std::time::Instant::now() + Duration::from_secs(POLL_TIMEOUT_SECS);
    loop {
        let status: FalQueueStatus = fal_get(api_key, &submit_resp.status_url)?;
        if status.status == "COMPLETED" {
            break;
        }
        if std::time::Instant::now() >= deadline {
            return Err(BackendError::Decode(format!(
                "Fal Veo ref-to-video request {} did not complete within {}s (last status: {})",
                submit_resp.request_id, POLL_TIMEOUT_SECS, status.status,
            )));
        }
        std::thread::sleep(Duration::from_secs(POLL_INTERVAL_SECS));
    }

    // --- 3. Fetch result ---
    let result_resp: FalVeoResultResponse = fal_get(api_key, &submit_resp.response_url)?;
    let video_url = &result_resp.video.url;

    // --- 4. Download asset ---
    let bytes = fal_fetch_bytes(api_key, video_url)?;
    let video_path = cache.write_asset(provider, request_hash, "mp4", &bytes)?;
    let video_bytes = bytes.len() as u64;

    let duration_secs = fal_duration_f32(&body.duration);
    let result = VideoResult {
        provider: provider.into(),
        video_path: video_path.clone(),
        video_bytes,
        duration_secs,
        width: 0,
        height: 0,
        mime: result_resp.video.content_type.clone(),
        prompt_sent: body.prompt.clone(),
        seed_used: request.seed,
    };

    let manifest = Manifest {
        version: 1,
        provider: provider.into(),
        cluster: CLUSTER_MULTI_REF_VIDEO.into(),
        request_hash: request_hash.into(),
        request: serde_json::to_value(request)
            .map_err(|e| BackendError::Cache(format!("serialize request for cache: {e}")))?,
        response: serde_json::to_value(&result)
            .map_err(|e| BackendError::Cache(format!("serialize response for cache: {e}")))?,
        cost_estimate_usd: estimate.cost_usd,
        asset_path: Some(video_path.display().to_string()),
        created_at: utc_now_iso8601(),
    };
    cache.store(&manifest)?;

    Ok(BackendCallOutcome {
        response: result,
        provider: provider.into(),
        request_hash: request_hash.into(),
        cached: false,
        cost_estimate_usd: estimate.cost_usd,
        mode: mode_label(mode),
    })
}

/// Coerce duration to `"4s"` or `"8s"` — the two values Fal's Veo 3.1
/// ref-to-video endpoint accepts.
fn fal_duration_str(secs: f32) -> &'static str {
    if secs <= 5.0 {
        "4s"
    } else {
        "8s"
    }
}

/// Parse `"4s"` / `"8s"` back to `f32`.
fn fal_duration_f32(s: &str) -> f32 {
    match s {
        "8s" => 8.0,
        _ => 4.0,
    }
}

// ---------------------------------------------------------------------------
// HTTP helpers (Fal queue protocol — mirrors fal/veo.rs helpers but
// scoped to this module to avoid a cross-module pub(crate) surface)
// ---------------------------------------------------------------------------

/// POST JSON to a Fal queue URL with `Authorization: Key <api_key>`.
fn fal_post<B, R>(api_key: &str, url: &str, body: &B) -> Result<R, BackendError>
where
    B: Serialize,
    R: for<'de> Deserialize<'de>,
{
    let json_body = serde_json::to_string(body)
        .map_err(|e| BackendError::InvalidRequest(format!("serialize body: {e}")))?;
    let resp = ureq::post(url)
        .set("Authorization", &format!("Key {api_key}"))
        .set("Accept", "application/json")
        .set("Content-Type", "application/json")
        .send_string(&json_body);
    decode_response(resp)
}

/// GET a Fal queue URL with `Authorization: Key <api_key>`.
fn fal_get<R>(api_key: &str, url: &str) -> Result<R, BackendError>
where
    R: for<'de> Deserialize<'de>,
{
    let resp = ureq::get(url)
        .set("Authorization", &format!("Key {api_key}"))
        .set("Accept", "application/json")
        .call();
    decode_response(resp)
}

/// Download bytes from a CDN URL.
fn fal_fetch_bytes(api_key: &str, url: &str) -> Result<Vec<u8>, BackendError> {
    use std::io::Read;
    let resp = ureq::get(url)
        .set("Authorization", &format!("Key {api_key}"))
        .call()
        .map_err(|e| BackendError::Transport(format!("fetch video asset: {e}")))?;
    let mut buf = Vec::with_capacity(1024 * 1024);
    resp.into_reader()
        .read_to_end(&mut buf)
        .map_err(|e| BackendError::Transport(format!("read video asset: {e}")))?;
    if buf.is_empty() {
        return Err(BackendError::Decode("empty video asset response".into()));
    }
    Ok(buf)
}

fn decode_response<R>(resp: Result<ureq::Response, ureq::Error>) -> Result<R, BackendError>
where
    R: for<'de> Deserialize<'de>,
{
    match resp {
        Ok(r) => {
            let raw = r
                .into_string()
                .map_err(|e| BackendError::Transport(e.to_string()))?;
            serde_json::from_str(&raw)
                .map_err(|e| BackendError::Decode(format!("fal response: {e} — body: {raw}")))
        }
        Err(ureq::Error::Status(status, r)) => {
            let body = r.into_string().unwrap_or_default();
            Err(BackendError::HttpStatus { status, body })
        }
        Err(e) => Err(BackendError::Transport(e.to_string())),
    }
}

// ---------------------------------------------------------------------------
// Wire types
// ---------------------------------------------------------------------------

/// Request body for `POST queue.fal.run/fal-ai/veo3.1/reference-to-video`.
#[derive(Debug, Serialize)]
struct FalVeoRefBody {
    prompt: String,
    /// 1–4 public HTTPS URLs for reference images.
    image_urls: Vec<String>,
    /// `"4s"` or `"8s"`.
    duration: String,
    /// `"16:9"`, `"9:16"`, `"1:1"`, etc.
    aspect_ratio: String,
    /// `"720p"` (only resolution currently advertised).
    resolution: &'static str,
    /// Include ambient audio in the generated clip.
    generate_audio: bool,
}

impl FalVeoRefBody {
    fn from_request(req: &MultiRefVideoRequest, image_urls: Vec<String>) -> Self {
        FalVeoRefBody {
            prompt: req.prompt.clone(),
            image_urls,
            duration: fal_duration_str(req.duration_secs).to_string(),
            aspect_ratio: req.aspect_ratio.clone(),
            resolution: "720p",
            generate_audio: true,
        }
    }
}

/// Response to the initial queue submit POST.
#[derive(Debug, Deserialize)]
struct FalQueueSubmitResponse {
    request_id: String,
    response_url: String,
    status_url: String,
}

/// Payload returned by polling `status_url`.
#[derive(Debug, Deserialize)]
struct FalQueueStatus {
    status: String,
}

/// Final result payload from `response_url`.
#[derive(Debug, Deserialize)]
struct FalVeoResultResponse {
    video: FalVeoVideoFile,
}

/// Video file descriptor inside the final result.
#[derive(Debug, Deserialize)]
struct FalVeoVideoFile {
    url: String,
    content_type: String,
    #[serde(default)]
    #[allow(dead_code)]
    file_size: u64,
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backends::cache::AssetCache;

    fn fresh_cache() -> std::path::PathBuf {
        let tmp = std::env::temp_dir().join(format!(
            "wavelet-fal-veo-ref-{}",
            AssetCache::request_hash("seed", "seed", &"x").unwrap()
        ));
        let _ = std::fs::remove_dir_all(&tmp);
        tmp
    }

    fn stub_adapter(model: FalVeoRefModel) -> FalVeoRefAdapter {
        FalVeoRefAdapter::new(
            FalClient::with_key("test-key", fresh_cache()),
            model,
            "test-key",
        )
    }

    fn stub_request_with_refs(refs: Vec<&str>) -> MultiRefVideoRequest {
        MultiRefVideoRequest::new(
            "a woman demonstrates the product",
            refs.into_iter().map(str::to_string).collect(),
        )
    }

    // --- Model parse ---

    #[test]
    fn model_parse_standard_aliases() {
        assert_eq!(
            FalVeoRefModel::parse("fal-veo3-ref").unwrap(),
            FalVeoRefModel::Standard
        );
        assert_eq!(
            FalVeoRefModel::parse("fal-veo3.1-ref").unwrap(),
            FalVeoRefModel::Standard
        );
        assert_eq!(
            FalVeoRefModel::parse("fal-veo3-ref-standard").unwrap(),
            FalVeoRefModel::Standard
        );
        assert_eq!(
            FalVeoRefModel::parse(MODEL_PATH_STANDARD).unwrap(),
            FalVeoRefModel::Standard
        );
    }

    #[test]
    fn model_parse_fast_aliases() {
        assert_eq!(
            FalVeoRefModel::parse("fal-veo3-ref-fast").unwrap(),
            FalVeoRefModel::Fast
        );
        assert_eq!(
            FalVeoRefModel::parse("fal-veo3.1-ref-fast").unwrap(),
            FalVeoRefModel::Fast
        );
        assert_eq!(
            FalVeoRefModel::parse(MODEL_PATH_FAST).unwrap(),
            FalVeoRefModel::Fast
        );
    }

    #[test]
    fn model_parse_unknown_errors() {
        assert!(FalVeoRefModel::parse("nope").is_err());
        assert!(FalVeoRefModel::parse("fal-veo3").is_err());
    }

    // --- Cost estimates ---

    #[test]
    fn cost_estimate_standard_4s() {
        let adapter = stub_adapter(FalVeoRefModel::Standard);
        let req = stub_request_with_refs(vec!["https://x/ref.png"]);
        let mut req = req;
        req.duration_secs = 4.0;
        let est = <FalVeoRefAdapter as MultiRefVideoGenBackend>::estimate_cost(&adapter, &req);
        assert!((est.cost_usd - 4.0 * PRICE_PER_SECOND_STANDARD_USD).abs() < 1e-4);
    }

    #[test]
    fn cost_estimate_fast_4s() {
        let adapter = stub_adapter(FalVeoRefModel::Fast);
        let req = stub_request_with_refs(vec!["https://x/ref.png"]);
        let mut req = req;
        req.duration_secs = 4.0;
        let est = <FalVeoRefAdapter as MultiRefVideoGenBackend>::estimate_cost(&adapter, &req);
        assert!((est.cost_usd - 4.0 * PRICE_PER_SECOND_FAST_USD).abs() < 1e-4);
    }

    #[test]
    fn standard_costs_more_than_fast() {
        let req = {
            let mut r = stub_request_with_refs(vec!["https://x/ref.png"]);
            r.duration_secs = 4.0;
            r
        };
        let std_cost = <FalVeoRefAdapter as MultiRefVideoGenBackend>::estimate_cost(
            &stub_adapter(FalVeoRefModel::Standard),
            &req,
        )
        .cost_usd;
        let fast_cost = <FalVeoRefAdapter as MultiRefVideoGenBackend>::estimate_cost(
            &stub_adapter(FalVeoRefModel::Fast),
            &req,
        )
        .cost_usd;
        assert!(std_cost > fast_cost);
    }

    // --- Validation ---

    #[test]
    fn empty_prompt_rejected() {
        let adapter = stub_adapter(FalVeoRefModel::Standard);
        let req = MultiRefVideoRequest::new(
            "  ",
            vec!["https://x/ref.png".into()],
        );
        let err =
            <FalVeoRefAdapter as MultiRefVideoGenBackend>::generate(&adapter, &req, RunMode::DryRun)
                .unwrap_err();
        assert!(matches!(err, BackendError::InvalidRequest(_)));
    }

    #[test]
    fn missing_references_rejected() {
        let adapter = stub_adapter(FalVeoRefModel::Standard);
        let req = MultiRefVideoRequest::new("a product demo", vec![]);
        let err =
            <FalVeoRefAdapter as MultiRefVideoGenBackend>::generate(&adapter, &req, RunMode::DryRun)
                .unwrap_err();
        match &err {
            BackendError::InvalidRequest(msg) => assert!(msg.contains("--reference")),
            other => panic!("expected InvalidRequest, got {other:?}"),
        }
    }

    #[test]
    fn too_many_references_rejected() {
        let adapter = stub_adapter(FalVeoRefModel::Standard);
        let refs = vec![
            "https://x/r1.png".into(),
            "https://x/r2.png".into(),
            "https://x/r3.png".into(),
            "https://x/r4.png".into(),
        ];
        let req = MultiRefVideoRequest::new("a product demo", refs);
        let err =
            <FalVeoRefAdapter as MultiRefVideoGenBackend>::generate(&adapter, &req, RunMode::DryRun)
                .unwrap_err();
        match &err {
            BackendError::InvalidRequest(msg) => {
                assert!(
                    msg.contains("at most 3 reference images"),
                    "expected new at-most-3 message, got: {msg}"
                );
                assert!(msg.contains("passed 4"), "expected count in message, got: {msg}");
            }
            other => panic!("expected InvalidRequest, got {other:?}"),
        }
    }

    #[test]
    fn exactly_three_references_accepted() {
        let adapter = stub_adapter(FalVeoRefModel::Standard);
        let refs = vec![
            "https://x/r1.png".into(),
            "https://x/r2.png".into(),
            "https://x/r3.png".into(),
        ];
        let req = MultiRefVideoRequest::new("a product demo", refs);
        // Should accept — DryRun returns a placeholder outcome instead of erroring.
        let outcome =
            <FalVeoRefAdapter as MultiRefVideoGenBackend>::generate(&adapter, &req, RunMode::DryRun)
                .expect("3 refs is the documented maximum, should be accepted");
        assert!(!outcome.cached);
    }

    #[test]
    fn dry_run_returns_placeholder_without_calling() {
        let adapter = stub_adapter(FalVeoRefModel::Standard);
        let req = {
            let mut r = stub_request_with_refs(vec!["https://x/ref.png"]);
            r.duration_secs = 4.0;
            r
        };
        let outcome =
            <FalVeoRefAdapter as MultiRefVideoGenBackend>::generate(&adapter, &req, RunMode::DryRun)
                .unwrap();
        assert_eq!(outcome.provider, "fal-veo3-ref-standard");
        assert!(!outcome.cached);
        assert!(outcome.cost_estimate_usd > 0.0);
        assert_eq!(outcome.mode, "dry-run");
    }

    #[test]
    fn over_budget_rejected_before_network() {
        let adapter = stub_adapter(FalVeoRefModel::Standard);
        let req = {
            let mut r = stub_request_with_refs(vec!["https://x/ref.png"]);
            r.duration_secs = 8.0;
            r
        };
        // 8s × $0.50 = $4.00 > $1.00 budget.
        let err = <FalVeoRefAdapter as MultiRefVideoGenBackend>::generate(
            &adapter,
            &req,
            RunMode::Live { max_cost_usd: 1.0 },
        )
        .unwrap_err();
        assert!(matches!(err, BackendError::OverBudget { .. }));
    }

    #[test]
    fn from_env_errors_when_key_unset() {
        unsafe { std::env::remove_var(FAL_KEY_ENV) };
        let tmp = std::env::temp_dir().join("wavelet-fal-veo-ref-no-env");
        let err = FalVeoRefAdapter::from_env(FalVeoRefModel::Standard, tmp).unwrap_err();
        assert!(matches!(err, BackendError::MissingCredential(_)));
    }

    // --- Wire body ---

    #[test]
    fn body_serializes_image_urls_array() {
        let req = {
            let mut r = MultiRefVideoRequest::new(
                "a product demo",
                vec![
                    "https://x/ref1.png".into(),
                    "https://x/ref2.png".into(),
                ],
            );
            r.duration_secs = 8.0;
            r.aspect_ratio = "9:16".into();
            r
        };
        let image_urls = req.reference_images.clone();
        let body = FalVeoRefBody::from_request(&req, image_urls);
        let v = serde_json::to_value(&body).unwrap();
        assert_eq!(v["prompt"], "a product demo");
        assert_eq!(v["duration"], "8s");
        assert_eq!(v["aspect_ratio"], "9:16");
        assert_eq!(v["resolution"], "720p");
        assert_eq!(v["generate_audio"], true);
        let urls = v["image_urls"].as_array().unwrap();
        assert_eq!(urls.len(), 2);
        assert_eq!(urls[0], "https://x/ref1.png");
        assert_eq!(urls[1], "https://x/ref2.png");
    }

    #[test]
    fn fast_model_path_uses_slash_not_dash() {
        // Regression: eval 010 (2026-05-23) shipped with the wrong path
        // `veo3.1-fast/reference-to-video` (dash), which 404'd. The real
        // Fal path is `veo3.1/fast/reference-to-video` (slash). Pin this
        // so the next typo gets caught here, not in a paid Fal call.
        // See <https://fal.ai/models/fal-ai/veo3.1/fast/reference-to-video>.
        assert_eq!(MODEL_PATH_FAST, "fal-ai/veo3.1/fast/reference-to-video");
        assert_eq!(FalVeoRefModel::Fast.path(), "fal-ai/veo3.1/fast/reference-to-video");
        assert!(
            !MODEL_PATH_FAST.contains("veo3.1-fast"),
            "fast path must use slash, not dash"
        );
    }

    #[test]
    fn wire_body_pins_full_fal_schema() {
        // Wire-schema regression. If this drifts from Fal's documented
        // schema at <https://fal.ai/models/fal-ai/veo3.1/reference-to-video/api>
        // the next live call 4xx's and a human has to read the eval
        // trace to find out why. Pin the exact JSON shape here instead.
        //
        // Documented fields (probed 2026-05-23):
        //   prompt           — required, string
        //   image_urls       — required, list<string> (≤3)
        //   aspect_ratio     — optional enum, "16:9" or "9:16"
        //   duration         — optional string, "4s" / "8s" (we quantize)
        //   resolution       — optional enum, "720p" | "1080p" | "4k"
        //   generate_audio   — optional bool, default true
        // We intentionally omit auto_fix and safety_tolerance.
        let req = {
            let mut r = MultiRefVideoRequest::new(
                "a woman demonstrates the product",
                vec!["https://x/ref1.png".into()],
            );
            r.duration_secs = 8.0;
            r.aspect_ratio = "9:16".into();
            r
        };
        let body = FalVeoRefBody::from_request(&req, req.reference_images.clone());
        let v = serde_json::to_value(&body).unwrap();
        let obj = v.as_object().expect("body is a JSON object");
        let mut keys: Vec<&str> = obj.keys().map(String::as_str).collect();
        keys.sort();
        assert_eq!(
            keys,
            vec![
                "aspect_ratio",
                "duration",
                "generate_audio",
                "image_urls",
                "prompt",
                "resolution",
            ],
            "wire body keys drifted from Fal's documented schema"
        );
    }

    #[test]
    fn body_serializes_single_reference() {
        let req = MultiRefVideoRequest::new(
            "close-up of hands",
            vec!["https://x/hands.png".into()],
        );
        let body = FalVeoRefBody::from_request(&req, req.reference_images.clone());
        let v = serde_json::to_value(&body).unwrap();
        assert_eq!(v["image_urls"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn duration_coercion() {
        assert_eq!(fal_duration_str(4.0), "4s");
        assert_eq!(fal_duration_str(5.0), "4s");
        assert_eq!(fal_duration_str(5.1), "8s");
        assert_eq!(fal_duration_str(8.0), "8s");
    }

    #[test]
    fn status_response_decodes() {
        let json = r#"{"status":"COMPLETED"}"#;
        let s: FalQueueStatus = serde_json::from_str(json).unwrap();
        assert_eq!(s.status, "COMPLETED");
    }

    #[test]
    fn submit_response_decodes() {
        let json = r#"{
            "status": "IN_QUEUE",
            "request_id": "019e55e3-ref",
            "response_url": "https://queue.fal.run/fal-ai/veo3.1/reference-to-video/requests/019e55e3-ref",
            "status_url": "https://queue.fal.run/fal-ai/veo3.1/reference-to-video/requests/019e55e3-ref/status",
            "cancel_url": "https://queue.fal.run/fal-ai/veo3.1/reference-to-video/requests/019e55e3-ref/cancel"
        }"#;
        let r: FalQueueSubmitResponse = serde_json::from_str(json).unwrap();
        assert_eq!(r.request_id, "019e55e3-ref");
        assert!(r.status_url.ends_with("/status"));
    }

    #[test]
    fn result_response_decodes() {
        let json = r#"{
            "video": {
                "url": "https://v3b.fal.media/files/abc/out.mp4",
                "content_type": "video/mp4",
                "file_name": "out.mp4",
                "file_size": 1234567
            }
        }"#;
        let r: FalVeoResultResponse = serde_json::from_str(json).unwrap();
        assert_eq!(r.video.content_type, "video/mp4");
        assert!(r.video.url.ends_with(".mp4"));
    }

    // --- Integration test (live API — gated on env var) ---

    #[test]
    #[ignore = "set WAVELET_LIVE_API_TESTS=1 to run; makes real Fal API calls and incurs cost"]
    fn integration_single_reference_generates_clip() {
        if std::env::var("WAVELET_LIVE_API_TESTS").as_deref() != Ok("1") {
            return;
        }
        let cache = std::env::temp_dir().join("wavelet-fal-veo-ref-live");
        let adapter =
            FalVeoRefAdapter::from_env(FalVeoRefModel::Standard, &cache).expect("FAL_KEY must be set");
        let req = MultiRefVideoRequest::new(
            "a woman smiles and looks at the camera, creator-to-camera UGC style",
            vec!["https://storage.googleapis.com/falserverless/gallery/dog.webp".into()],
        );
        let outcome = <FalVeoRefAdapter as MultiRefVideoGenBackend>::generate(
            &adapter,
            &req,
            RunMode::Live { max_cost_usd: 5.0 },
        )
        .expect("generation should succeed");
        assert!(!outcome.cached);
        assert!(outcome.response.video_bytes > 0);
        assert!(outcome.response.video_path.exists());
    }
}
