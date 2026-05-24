//! Fal-hosted Veo 3 adapter — `Txt2VidGen` cluster.
//!
//! Fal added Veo 3 (Standard + Fast tiers) to its queue API on or
//! around 2026-05-23. This adapter routes through `queue.fal.run`
//! rather than Google AI Studio directly — useful when the Google
//! project quota is exhausted.
//!
//! ## Wire protocol
//!
//! ```text
//! POST https://queue.fal.run/fal-ai/veo3          (Standard)
//! POST https://queue.fal.run/fal-ai/veo3/fast     (Fast)
//! Authorization: Key <FAL_KEY>
//! { "prompt":"…", "duration":"4s", "aspect_ratio":"9:16", "resolution":"720p" }
//!
//! → { "status":"IN_QUEUE", "request_id":"019e55e3-…",
//!     "response_url":"https://queue.fal.run/fal-ai/veo3/requests/<id>",
//!     "status_url":"https://queue.fal.run/fal-ai/veo3/requests/<id>/status" }
//!
//! GET <status_url>   → { "status":"IN_QUEUE"|"IN_PROGRESS"|"COMPLETED" }
//! GET <response_url> → { "video":{ "url":"https://v3b.fal.media/…/*.mp4",
//!                                  "content_type":"video/mp4",
//!                                  "file_name":"…", "file_size":569708 } }
//! ```
//!
//! ## Pricing (Fal, probed 2026-05-23)
//!
//! Fal mirrors Google's published preview pricing for Veo 3:
//!
//! - Standard (`fal-ai/veo3`): ~$0.50/s (audio included)
//! - Fast (`fal-ai/veo3/fast`): ~$0.25/s
//!
//! Verify at <https://fal.ai/models/fal-ai/veo3> before raising budget
//! gates — these may change as Veo exits preview.
//!
//! ## Accepted durations
//!
//! Fal's Veo 3 accepts `"4s"` or `"8s"` (string form, not integer).
//! Values outside those are clamped. 4 s is used for 4–5 s requests;
//! 8 s for anything over 5 s.

use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::backends::cache::{utc_now_iso8601, AssetCache, Manifest};
use crate::backends::fal::FalClient;
use crate::backends::http_client::FAL_KEY_ENV;
use crate::backends::video::{Txt2VidGenBackend, Txt2VidRequest, VideoResult, CLUSTER_TXT2VID};
use crate::backends::{
    check_budget, mode_label, BackendCallOutcome, BackendError, CostEstimate, RunMode,
};

/// Queue base URL — the sync endpoint (`fal.run`) does not support Veo
/// because generations take 30–120 s and would time out. The queue
/// endpoint accepts the job, returns a `request_id`, and lets us poll.
pub const QUEUE_BASE: &str = "https://queue.fal.run";

/// Fal model path for the Standard tier.
pub const MODEL_PATH_STANDARD: &str = "fal-ai/veo3";

/// Fal model path for the Fast tier.
pub const MODEL_PATH_FAST: &str = "fal-ai/veo3/fast";

/// Per-second price for Standard tier (Fal mirrors Google's preview
/// pricing of ~$0.50/s with audio). Check fal.ai/models/fal-ai/veo3.
pub const PRICE_PER_SECOND_STANDARD_USD: f32 = 0.50;

/// Per-second price for Fast tier (~$0.25/s). Check fal.ai/models/fal-ai/veo3/fast.
pub const PRICE_PER_SECOND_FAST_USD: f32 = 0.25;

/// Maximum wall-time for a single generation poll loop (5 minutes).
/// Veo 3 Standard typically completes in 30–120 s; Fast in ~30 s.
pub const POLL_TIMEOUT_SECS: u64 = 300;

/// Interval between status polls (5 seconds).
pub const POLL_INTERVAL_SECS: u64 = 5;

/// The two Fal-hosted Veo 3 model tiers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FalVeoModel {
    /// Full Veo 3 Standard — higher quality, ~$0.50/s.
    Standard,
    /// Veo 3 Fast — lower latency, ~$0.25/s.
    Fast,
}

impl FalVeoModel {
    /// Fal queue path for the model (appended to `QUEUE_BASE`).
    pub fn path(self) -> &'static str {
        match self {
            FalVeoModel::Standard => MODEL_PATH_STANDARD,
            FalVeoModel::Fast => MODEL_PATH_FAST,
        }
    }

    /// Per-second cost in USD.
    pub fn price_per_second(self) -> f32 {
        match self {
            FalVeoModel::Standard => PRICE_PER_SECOND_STANDARD_USD,
            FalVeoModel::Fast => PRICE_PER_SECOND_FAST_USD,
        }
    }

    /// Provider identifier used in cache keys and manifests.
    pub fn provider(self) -> &'static str {
        match self {
            FalVeoModel::Standard => "fal-veo3-standard",
            FalVeoModel::Fast => "fal-veo3-fast",
        }
    }

    /// Parse from a backend-name string. Accepts the aliases registered
    /// in `shot_txt2vid` dispatch.
    pub fn parse(s: &str) -> Result<Self, String> {
        match s {
            "fal-veo3" | "fal-veo-3" | "fal-veo3-standard" | MODEL_PATH_STANDARD => {
                Ok(FalVeoModel::Standard)
            }
            "fal-veo3-fast" | "fal-veo-fast-fal" | MODEL_PATH_FAST => Ok(FalVeoModel::Fast),
            other => Err(format!(
                "unknown Fal Veo model '{other}'. want one of: \
                 fal-veo3|fal-veo-3|fal-veo3-standard, fal-veo3-fast|fal-veo-fast-fal"
            )),
        }
    }
}

/// Fal-hosted Veo 3 adapter — implements `Txt2VidGenBackend` by routing
/// through Fal's queue API.
#[derive(Debug, Clone)]
pub struct FalVeoAdapter {
    client: FalClient,
    model: FalVeoModel,
    /// Fal API key, held separately so we can inject it into queue HTTP
    /// calls (which use `QUEUE_BASE`, not the sync `fal.run` base that
    /// `FalClient::post_sync` targets).
    api_key: String,
}

impl FalVeoAdapter {
    /// Build from a pre-constructed client + model variant + API key.
    ///
    /// Most callers should use [`FalVeoAdapter::from_env`] instead.
    pub fn new(client: FalClient, model: FalVeoModel, api_key: impl Into<String>) -> Self {
        Self {
            client,
            model,
            api_key: api_key.into(),
        }
    }

    /// Build from `FAL_KEY` environment variable.
    pub fn from_env(
        model: FalVeoModel,
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
}

impl Txt2VidGenBackend for FalVeoAdapter {
    fn name(&self) -> &'static str {
        self.model.provider()
    }

    fn estimate_cost(&self, request: &Txt2VidRequest) -> CostEstimate {
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
        request: &Txt2VidRequest,
        mode: RunMode,
    ) -> Result<BackendCallOutcome<VideoResult>, BackendError> {
        if request.prompt.trim().is_empty() {
            return Err(BackendError::InvalidRequest("prompt is empty".into()));
        }
        let estimate = <Self as Txt2VidGenBackend>::estimate_cost(self, request);
        check_budget(&estimate, mode)?;

        let provider = self.model.provider();
        let request_hash = AssetCache::request_hash(provider, CLUSTER_TXT2VID, request)?;
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

        fal_queue_call(
            &self.api_key,
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

/// Submit → poll → download via Fal's queue API.
#[allow(clippy::too_many_arguments)]
fn fal_queue_call(
    api_key: &str,
    model: FalVeoModel,
    request: &Txt2VidRequest,
    estimate: &CostEstimate,
    cache: &AssetCache,
    provider: &'static str,
    request_hash: &str,
    mode: RunMode,
) -> Result<BackendCallOutcome<VideoResult>, BackendError> {
    let body = FalVeoBody::from_request(request);

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
                "Fal Veo request {} did not complete within {}s (last status: {})",
                submit_resp.request_id, POLL_TIMEOUT_SECS, status.status,
            )));
        }
        std::thread::sleep(Duration::from_secs(POLL_INTERVAL_SECS));
    }

    // --- 3. Fetch result ---
    let result_resp: FalVeoResultResponse = fal_get(api_key, &submit_resp.response_url)?;
    let video_url = &result_resp.video.url;

    // --- 4. Download asset ---
    // The v3b.fal.media CDN URL is publicly accessible without auth,
    // but we include the auth header anyway in case Fal changes this.
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
        cluster: CLUSTER_TXT2VID.into(),
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
    decode_fal_response(resp)
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
    decode_fal_response(resp)
}

/// Download bytes from `url`, trying the auth header first (Fal CDN
/// does not require it today, but this is forward-safe).
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

fn decode_fal_response<R>(
    resp: Result<ureq::Response, ureq::Error>,
) -> Result<R, BackendError>
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

/// Coerce duration to one of `"4s"` or `"8s"` — the two values Fal's
/// Veo 3 endpoint accepts. Anything ≤ 5 s maps to 4 s; anything > 5 s
/// maps to 8 s.
fn fal_duration_str(secs: f32) -> &'static str {
    if secs <= 5.0 {
        "4s"
    } else {
        "8s"
    }
}

/// Parse the `"4s"` / `"8s"` strings back to `f32` for `VideoResult`.
fn fal_duration_f32(s: &str) -> f32 {
    match s {
        "8s" => 8.0,
        _ => 4.0,
    }
}

// ---------------------------------------------------------------------------
// Wire types
// ---------------------------------------------------------------------------

/// Request body for `POST queue.fal.run/fal-ai/veo3[/fast]`.
#[derive(Debug, Serialize)]
struct FalVeoBody {
    prompt: String,
    /// `"4s"` or `"8s"`.
    duration: String,
    /// `"16:9"`, `"9:16"`, `"1:1"`, etc.
    aspect_ratio: String,
    /// `"720p"` (only resolution the endpoint currently advertises).
    resolution: &'static str,
}

impl FalVeoBody {
    fn from_request(req: &Txt2VidRequest) -> Self {
        FalVeoBody {
            prompt: req.prompt.clone(),
            duration: fal_duration_str(req.duration_secs).to_string(),
            aspect_ratio: req.aspect_ratio.clone(),
            resolution: "720p",
        }
    }
}

/// Response to the initial queue submit POST.
#[derive(Debug, Deserialize)]
struct FalQueueSubmitResponse {
    /// Unique job identifier.
    request_id: String,
    /// URL to GET for the final payload once `COMPLETED`.
    response_url: String,
    /// URL to GET for `{ "status": "IN_QUEUE"|"IN_PROGRESS"|"COMPLETED" }`.
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
    /// CDN URL for the rendered MP4.
    url: String,
    /// MIME type (`"video/mp4"`).
    content_type: String,
    /// Reported file size in bytes (informational only).
    #[serde(default)]
    #[allow(dead_code)]
    file_size: u64,
}


#[cfg(test)]
mod tests {
    use super::*;

    fn fresh_cache() -> std::path::PathBuf {
        let tmp = std::env::temp_dir().join(format!(
            "wavelet-fal-veo-{}",
            AssetCache::request_hash("seed", "seed", &"x").unwrap()
        ));
        let _ = std::fs::remove_dir_all(&tmp);
        tmp
    }

    fn stub_adapter(model: FalVeoModel) -> FalVeoAdapter {
        FalVeoAdapter::new(FalClient::with_key("test-key", fresh_cache()), model, "test-key")
    }

    #[test]
    fn model_parse_aliases() {
        assert_eq!(
            FalVeoModel::parse("fal-veo3").unwrap(),
            FalVeoModel::Standard
        );
        assert_eq!(
            FalVeoModel::parse("fal-veo-3").unwrap(),
            FalVeoModel::Standard
        );
        assert_eq!(
            FalVeoModel::parse("fal-veo3-standard").unwrap(),
            FalVeoModel::Standard
        );
        assert_eq!(
            FalVeoModel::parse("fal-veo3-fast").unwrap(),
            FalVeoModel::Fast
        );
        assert_eq!(
            FalVeoModel::parse("fal-veo-fast-fal").unwrap(),
            FalVeoModel::Fast
        );
        assert!(FalVeoModel::parse("nope").is_err());
    }

    #[test]
    fn cost_estimate_scales_with_duration() {
        let adapter = stub_adapter(FalVeoModel::Fast);
        let req = Txt2VidRequest {
            prompt: "x".into(),
            negative_prompt: None,
            apply_default_negatives: true,
            duration_secs: 8.0,
            aspect_ratio: "16:9".into(),
            seed: None,
        };
        let est = <FalVeoAdapter as Txt2VidGenBackend>::estimate_cost(&adapter, &req);
        assert!((est.cost_usd - 8.0 * PRICE_PER_SECOND_FAST_USD).abs() < 1e-4);
    }

    #[test]
    fn cost_scales_by_tier() {
        let req = Txt2VidRequest {
            prompt: "x".into(),
            negative_prompt: None,
            apply_default_negatives: true,
            duration_secs: 4.0,
            aspect_ratio: "16:9".into(),
            seed: None,
        };
        let standard_cost = <FalVeoAdapter as Txt2VidGenBackend>::estimate_cost(
            &stub_adapter(FalVeoModel::Standard),
            &req,
        )
        .cost_usd;
        let fast_cost = <FalVeoAdapter as Txt2VidGenBackend>::estimate_cost(
            &stub_adapter(FalVeoModel::Fast),
            &req,
        )
        .cost_usd;
        assert!(standard_cost > fast_cost);
    }

    #[test]
    fn dry_run_returns_request_shape_without_calling() {
        let adapter = stub_adapter(FalVeoModel::Standard);
        let req = Txt2VidRequest {
            prompt: "a cat blinks".into(),
            negative_prompt: None,
            apply_default_negatives: true,
            duration_secs: 4.0,
            aspect_ratio: "16:9".into(),
            seed: Some(42),
        };
        let outcome =
            <FalVeoAdapter as Txt2VidGenBackend>::generate(&adapter, &req, RunMode::DryRun)
                .unwrap();
        assert_eq!(outcome.provider, "fal-veo3-standard");
        assert!(!outcome.cached);
        assert!(outcome.cost_estimate_usd > 0.0);
        assert_eq!(outcome.mode, "dry-run");
    }

    #[test]
    fn empty_prompt_rejected() {
        let adapter = stub_adapter(FalVeoModel::Fast);
        let req = Txt2VidRequest {
            prompt: "  ".into(),
            negative_prompt: None,
            apply_default_negatives: true,
            duration_secs: 4.0,
            aspect_ratio: "16:9".into(),
            seed: None,
        };
        let err =
            <FalVeoAdapter as Txt2VidGenBackend>::generate(&adapter, &req, RunMode::DryRun)
                .unwrap_err();
        assert!(matches!(err, BackendError::InvalidRequest(_)));
    }

    #[test]
    fn over_budget_rejected_before_network() {
        let adapter = stub_adapter(FalVeoModel::Standard);
        let req = Txt2VidRequest {
            prompt: "expensive scene".into(),
            negative_prompt: None,
            apply_default_negatives: true,
            duration_secs: 8.0,
            aspect_ratio: "16:9".into(),
            seed: None,
        };
        // 8s × $0.50 = $4.00 — exceeds the $1.00 budget.
        let err = <FalVeoAdapter as Txt2VidGenBackend>::generate(
            &adapter,
            &req,
            RunMode::Live { max_cost_usd: 1.0 },
        )
        .unwrap_err();
        assert!(matches!(err, BackendError::OverBudget { .. }));
    }

    #[test]
    fn duration_coercion() {
        assert_eq!(fal_duration_str(4.0), "4s");
        assert_eq!(fal_duration_str(5.0), "4s");
        assert_eq!(fal_duration_str(5.1), "8s");
        assert_eq!(fal_duration_str(8.0), "8s");
    }

    #[test]
    fn body_shape_matches_fal_wire() {
        let req = Txt2VidRequest {
            prompt: "a cat".into(),
            negative_prompt: None,
            apply_default_negatives: true,
            duration_secs: 8.0,
            aspect_ratio: "9:16".into(),
            seed: None,
        };
        let body = FalVeoBody::from_request(&req);
        let v = serde_json::to_value(&body).unwrap();
        assert_eq!(v["prompt"], "a cat");
        assert_eq!(v["duration"], "8s");
        assert_eq!(v["aspect_ratio"], "9:16");
        assert_eq!(v["resolution"], "720p");
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
            "request_id": "019e55e3-abc",
            "response_url": "https://queue.fal.run/fal-ai/veo3/requests/019e55e3-abc",
            "status_url": "https://queue.fal.run/fal-ai/veo3/requests/019e55e3-abc/status",
            "cancel_url": "https://queue.fal.run/fal-ai/veo3/requests/019e55e3-abc/cancel"
        }"#;
        let r: FalQueueSubmitResponse = serde_json::from_str(json).unwrap();
        assert_eq!(r.request_id, "019e55e3-abc");
        assert!(r.status_url.ends_with("/status"));
    }

    #[test]
    fn result_response_decodes() {
        let json = r#"{
            "video": {
                "url": "https://v3b.fal.media/files/abc/out.mp4",
                "content_type": "video/mp4",
                "file_name": "out.mp4",
                "file_size": 569708
            }
        }"#;
        let r: FalVeoResultResponse = serde_json::from_str(json).unwrap();
        assert_eq!(r.video.content_type, "video/mp4");
        assert!(r.video.url.ends_with(".mp4"));
    }

    #[test]
    fn from_env_errors_when_key_unset() {
        unsafe { std::env::remove_var(FAL_KEY_ENV) };
        let tmp = std::env::temp_dir().join("wavelet-fal-veo-no-env");
        let err = FalVeoAdapter::from_env(FalVeoModel::Standard, tmp).unwrap_err();
        assert!(matches!(err, BackendError::MissingCredential(_)));
    }
}
