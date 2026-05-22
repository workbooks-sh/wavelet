//! Google Veo 3.x adapter — `Txt2VidGen` + `Img2VidGen` clusters.
//!
//! **Native synced audio is on by default.** Probed live 2026-05-19:
//! Veo returns h264 + aac MP4s every time, no parameter required.
//! Audio is automatically synthesized from the prompt (dialogue,
//! ambient, foley). The `generateAudio` request parameter is rejected
//! by every model; if you want video-only output, strip the audio
//! stream post-fetch (the adapter doesn't do that — the bundled
//! audio is part of the gen contract).
//!
//! Wire format (probed live 2026-05-19):
//!
//! ```text
//! POST https://generativelanguage.googleapis.com/v1beta/models/<model>:predictLongRunning?key=…
//! { "instances":[{"prompt":"…"}], "parameters":{"aspectRatio":"16:9","durationSeconds":8} }
//! → { "name":"models/<model>/operations/<id>" }
//!
//! GET https://generativelanguage.googleapis.com/v1beta/<operation>?key=…
//! → { "name":"…","done":true,
//!     "response":{"generateVideoResponse":{"generatedSamples":[{"video":{"uri":"…/files/<id>:download?alt=media"}}]}}}
//! ```
//!
//! The asset URI is a Google AI Studio file download — same auth key
//! works as a query param.
//!
//! Image-to-video uses the same endpoint with an extra
//! `instances[0].image` field carrying a base64-encoded reference
//! frame. Veo 3.1 supports up to 3 reference frames (per Google docs).

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::backends::cache::{utc_now_iso8601, AssetCache, Manifest};
use crate::backends::video::{
    Img2VidGenBackend, Img2VidRequest, Txt2VidGenBackend, Txt2VidRequest, VideoResult,
    CLUSTER_IMG2VID, CLUSTER_TXT2VID,
};
use crate::backends::{
    check_budget, mode_label, BackendCallOutcome, BackendError, CostEstimate, RunMode,
};

use super::client::{GoogleAiClient, IsOperationDone};

/// Standard tier (full Veo 3.1).
pub const MODEL_VEO_3_1_STANDARD: &str = "veo-3.1-generate-preview";

/// Fast tier — same quality ceiling, faster wall-time.
pub const MODEL_VEO_3_1_FAST: &str = "veo-3.1-fast-generate-preview";

/// Lite tier — cheapest, used for draft passes.
pub const MODEL_VEO_3_1_LITE: &str = "veo-3.1-lite-generate-preview";

/// Per-second price for standard tier. Conservative — Google's listed
/// preview price is ~$0.50/s with audio, $0.30/s without; we charge
/// the with-audio rate to keep budget gates honest.
pub const PRICE_PER_SECOND_STANDARD_USD: f32 = 0.50;

/// Per-second price for fast tier (~half standard).
pub const PRICE_PER_SECOND_FAST_USD: f32 = 0.25;

/// Per-second price for lite tier (draft tier — cheap).
pub const PRICE_PER_SECOND_LITE_USD: f32 = 0.10;

/// One of the three Veo 3.1 model variants. Drives both routing + cost.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VeoModel {
    /// Full Veo 3.1.
    Standard,
    /// Fast tier.
    Fast,
    /// Lite tier (draft).
    Lite,
}

impl VeoModel {
    /// Model name used in the URL.
    pub fn name(self) -> &'static str {
        match self {
            VeoModel::Standard => MODEL_VEO_3_1_STANDARD,
            VeoModel::Fast => MODEL_VEO_3_1_FAST,
            VeoModel::Lite => MODEL_VEO_3_1_LITE,
        }
    }

    /// Per-second cost.
    pub fn price_per_second(self) -> f32 {
        match self {
            VeoModel::Standard => PRICE_PER_SECOND_STANDARD_USD,
            VeoModel::Fast => PRICE_PER_SECOND_FAST_USD,
            VeoModel::Lite => PRICE_PER_SECOND_LITE_USD,
        }
    }

    /// Provider identifier — used in cache keys + manifests.
    pub fn provider(self) -> &'static str {
        match self {
            VeoModel::Standard => "google-veo-3.1",
            VeoModel::Fast => "google-veo-3.1-fast",
            VeoModel::Lite => "google-veo-3.1-lite",
        }
    }

    /// Parse from a string. Accepts the model id directly or the short
    /// alias.
    pub fn parse(s: &str) -> Result<Self, String> {
        match s {
            "veo" | "veo-3.1" | "veo3.1" | "standard"
            | "google-veo" | "google-veo-3.1"
            | MODEL_VEO_3_1_STANDARD => Ok(VeoModel::Standard),
            "veo-fast" | "veo-3.1-fast" | "fast"
            | "google-veo-fast" | "google-veo-3.1-fast"
            | MODEL_VEO_3_1_FAST => Ok(VeoModel::Fast),
            "veo-lite" | "veo-3.1-lite" | "lite"
            | "google-veo-lite" | "google-veo-3.1-lite"
            | MODEL_VEO_3_1_LITE => Ok(VeoModel::Lite),
            other => Err(format!(
                "unknown Veo model '{other}'. want one of: \
                 veo|veo-3.1|google-veo-3.1, veo-fast|veo-3.1-fast|google-veo-3.1-fast, \
                 veo-lite|veo-3.1-lite|google-veo-3.1-lite"
            )),
        }
    }
}

/// Google Veo adapter.
#[derive(Debug, Clone)]
pub struct GoogleVeoAdapter {
    client: GoogleAiClient,
    model: VeoModel,
}

impl GoogleVeoAdapter {
    /// Build from a pre-constructed client + a chosen model variant.
    pub fn new(client: GoogleAiClient, model: VeoModel) -> Self {
        Self { client, model }
    }
}

impl Txt2VidGenBackend for GoogleVeoAdapter {
    fn name(&self) -> &'static str {
        self.model.provider()
    }

    fn estimate_cost(&self, request: &Txt2VidRequest) -> CostEstimate {
        let cost = request.duration_secs * self.model.price_per_second();
        CostEstimate {
            provider: self.model.provider().into(),
            cost_usd: cost,
            explanation: format!(
                "{duration:.1}s × ${rate:.2}/s = ${cost:.4} ({model})",
                duration = request.duration_secs,
                rate = self.model.price_per_second(),
                cost = cost,
                model = self.model.name(),
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

        let body = VeoBody::for_txt2vid(request);
        live_call(&self.client, self.model, &body, request, &estimate, cache, provider, CLUSTER_TXT2VID, &request_hash, mode)
    }
}

impl Img2VidGenBackend for GoogleVeoAdapter {
    fn name(&self) -> &'static str {
        self.model.provider()
    }

    fn estimate_cost(&self, request: &Img2VidRequest) -> CostEstimate {
        let cost = request.duration_secs * self.model.price_per_second();
        CostEstimate {
            provider: self.model.provider().into(),
            cost_usd: cost,
            explanation: format!(
                "{duration:.1}s × ${rate:.2}/s = ${cost:.4} ({model})",
                duration = request.duration_secs,
                rate = self.model.price_per_second(),
                cost = cost,
                model = self.model.name(),
            ),
        }
    }

    fn generate(
        &self,
        request: &Img2VidRequest,
        mode: RunMode,
    ) -> Result<BackendCallOutcome<VideoResult>, BackendError> {
        if request.prompt.trim().is_empty() {
            return Err(BackendError::InvalidRequest("prompt is empty".into()));
        }
        if request.image.trim().is_empty() {
            return Err(BackendError::InvalidRequest("image is empty".into()));
        }
        let estimate = <Self as Img2VidGenBackend>::estimate_cost(self, request);
        check_budget(&estimate, mode)?;

        let provider = self.model.provider();
        let request_hash = AssetCache::request_hash(provider, CLUSTER_IMG2VID, request)?;
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

        let body = VeoBody::for_img2vid(request)?;
        live_call(&self.client, self.model, &body, request, &estimate, cache, provider, CLUSTER_IMG2VID, &request_hash, mode)
    }
}

#[allow(clippy::too_many_arguments)]
fn live_call<Req: serde::Serialize>(
    client: &GoogleAiClient,
    model: VeoModel,
    body: &VeoBody,
    request: &Req,
    estimate: &CostEstimate,
    cache: &AssetCache,
    provider: &'static str,
    cluster: &'static str,
    request_hash: &str,
    mode: RunMode,
) -> Result<BackendCallOutcome<VideoResult>, BackendError> {
    let started: VeoLongRunningStart =
        client.post_sync(model.name(), "predictLongRunning", body)?;
    let op: VeoOperation = client.poll_until_done(&started.name)?;
    if let Some(err) = op.error {
        return Err(BackendError::Transport(format!(
            "Veo op {} failed (code {}): {}",
            started.name, err.code, err.message
        )));
    }
    let video_uri = op
        .response
        .as_ref()
        .and_then(|r| r.generate_video_response.as_ref())
        .and_then(|g| g.generated_samples.first())
        .map(|s| s.video.uri.as_str())
        .ok_or_else(|| {
            BackendError::Decode(
                "Veo op missing response.generateVideoResponse.generatedSamples[0].video.uri".into(),
            )
        })?;

    let bytes = client.fetch_asset(video_uri)?;
    let video_path = cache.write_asset(provider, request_hash, "mp4", &bytes)?;
    let video_bytes = bytes.len() as u64;

    let result = VideoResult {
        provider: provider.into(),
        video_path: video_path.clone(),
        video_bytes,
        duration_secs: body.parameters.duration_seconds.unwrap_or(0.0),
        width: 0,
        height: 0,
        mime: "video/mp4".into(),
        prompt_sent: body.instances[0].prompt.clone(),
        seed_used: body.parameters.seed,
    };

    let manifest = Manifest {
        version: 1,
        provider: provider.into(),
        cluster: cluster.into(),
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

#[derive(Debug, Serialize)]
struct VeoBody {
    instances: Vec<VeoInstance>,
    parameters: VeoParameters,
}

impl VeoBody {
    fn for_txt2vid(request: &Txt2VidRequest) -> Self {
        VeoBody {
            instances: vec![VeoInstance {
                prompt: request.prompt.clone(),
                image: None,
            }],
            parameters: VeoParameters {
                aspect_ratio: Some(request.aspect_ratio.clone()),
                duration_seconds: Some(request.duration_secs),
                negative_prompt: request.negative_prompt.clone(),
                seed: request.seed,
            },
        }
    }

    fn for_img2vid(request: &Img2VidRequest) -> Result<Self, BackendError> {
        let image = encode_image(&request.image)?;
        Ok(VeoBody {
            instances: vec![VeoInstance {
                prompt: request.prompt.clone(),
                image: Some(image),
            }],
            parameters: VeoParameters {
                aspect_ratio: Some(request.aspect_ratio.clone()),
                duration_seconds: Some(request.duration_secs),
                negative_prompt: request.negative_prompt.clone(),
                seed: request.seed,
            },
        })
    }
}

#[derive(Debug, Serialize)]
struct VeoInstance {
    prompt: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    image: Option<VeoImage>,
}

#[derive(Debug, Serialize)]
struct VeoImage {
    #[serde(rename = "bytesBase64Encoded")]
    bytes_base64: String,
    #[serde(rename = "mimeType")]
    mime_type: String,
}

#[derive(Debug, Serialize)]
struct VeoParameters {
    #[serde(rename = "aspectRatio", skip_serializing_if = "Option::is_none")]
    aspect_ratio: Option<String>,
    #[serde(rename = "durationSeconds", skip_serializing_if = "Option::is_none")]
    duration_seconds: Option<f32>,
    #[serde(rename = "negativePrompt", skip_serializing_if = "Option::is_none")]
    negative_prompt: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    seed: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct VeoLongRunningStart {
    name: String,
}

#[derive(Debug, Deserialize)]
struct VeoOperation {
    #[serde(default)]
    done: bool,
    #[serde(default)]
    response: Option<VeoResponseEnvelope>,
    #[serde(default)]
    error: Option<VeoError>,
}

impl IsOperationDone for VeoOperation {
    fn is_done(&self) -> bool {
        self.done
    }
}

#[derive(Debug, Deserialize)]
struct VeoResponseEnvelope {
    #[serde(default, rename = "generateVideoResponse")]
    generate_video_response: Option<GenerateVideoResponse>,
}

#[derive(Debug, Deserialize)]
struct GenerateVideoResponse {
    #[serde(default, rename = "generatedSamples")]
    generated_samples: Vec<GeneratedSample>,
}

#[derive(Debug, Deserialize)]
struct GeneratedSample {
    video: GeneratedVideo,
}

#[derive(Debug, Deserialize)]
struct GeneratedVideo {
    uri: String,
}

#[derive(Debug, Deserialize)]
struct VeoError {
    code: i32,
    message: String,
}

fn encode_image(source: &str) -> Result<VeoImage, BackendError> {
    use crate::backends::util::{base64_encode, ext_to_mime, sniff_image_ext};
    if source.starts_with("http://") || source.starts_with("https://") {
        return Err(BackendError::InvalidRequest(
            "Veo image input must be a local path or data: URL; got a remote URL. \
             Download it locally first (wavelet handles this for fal scene-still via path passthrough)."
                .into(),
        ));
    }
    if let Some(rest) = source.strip_prefix("data:") {
        let (header, b64) = rest.split_once(',').ok_or_else(|| {
            BackendError::InvalidRequest("malformed data: URL (no comma)".into())
        })?;
        let mime = header
            .split(';')
            .next()
            .unwrap_or("image/png")
            .to_string();
        return Ok(VeoImage {
            bytes_base64: b64.to_string(),
            mime_type: mime,
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
    let mime = ext_to_mime(ext);
    let bytes_base64 = base64_encode(&bytes);
    Ok(VeoImage {
        bytes_base64,
        mime_type: mime.into(),
    })
}

// Silence "unused field" — kept for forward compatibility with the
// VideoResult schema fields the cache decoder fills in.
#[allow(dead_code)]
fn _path_placeholder() -> PathBuf {
    PathBuf::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fresh_cache() -> std::path::PathBuf {
        let tmp = std::env::temp_dir().join(format!(
            "wavelet-google-veo-{}",
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
        assert_eq!(VeoModel::parse("veo").unwrap(), VeoModel::Standard);
        assert_eq!(VeoModel::parse("veo-3.1").unwrap(), VeoModel::Standard);
        assert_eq!(VeoModel::parse("fast").unwrap(), VeoModel::Fast);
        assert_eq!(VeoModel::parse("veo-3.1-fast").unwrap(), VeoModel::Fast);
        assert_eq!(VeoModel::parse("lite").unwrap(), VeoModel::Lite);
        assert!(VeoModel::parse("nope").is_err());
    }

    #[test]
    fn cost_estimate_scales_with_duration() {
        let adapter = GoogleVeoAdapter::new(stub_client(), VeoModel::Fast);
        let req = Txt2VidRequest {
            prompt: "x".into(),
            negative_prompt: None,
            apply_default_negatives: true,
            duration_secs: 8.0,
            aspect_ratio: "16:9".into(),
            seed: None,
        };
        let est = <GoogleVeoAdapter as Txt2VidGenBackend>::estimate_cost(&adapter, &req);
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
        let standard = <GoogleVeoAdapter as Txt2VidGenBackend>::estimate_cost(
            &GoogleVeoAdapter::new(stub_client(), VeoModel::Standard),
            &req,
        );
        let fast = <GoogleVeoAdapter as Txt2VidGenBackend>::estimate_cost(
            &GoogleVeoAdapter::new(stub_client(), VeoModel::Fast),
            &req,
        );
        let lite = <GoogleVeoAdapter as Txt2VidGenBackend>::estimate_cost(
            &GoogleVeoAdapter::new(stub_client(), VeoModel::Lite),
            &req,
        );
        assert!(standard.cost_usd > fast.cost_usd);
        assert!(fast.cost_usd > lite.cost_usd);
    }

    #[test]
    fn dry_run_returns_request_shape_without_calling() {
        let adapter = GoogleVeoAdapter::new(stub_client(), VeoModel::Fast);
        let req = Txt2VidRequest {
            prompt: "a cat blinks".into(),
            negative_prompt: None,
            apply_default_negatives: true,
            duration_secs: 4.0,
            aspect_ratio: "16:9".into(),
            seed: Some(42),
        };
        let outcome = <GoogleVeoAdapter as Txt2VidGenBackend>::generate(&adapter, &req, RunMode::DryRun).unwrap();
        assert_eq!(outcome.provider, "google-veo-3.1-fast");
        assert!(!outcome.cached);
        assert!(outcome.cost_estimate_usd > 0.0);
    }

    #[test]
    fn empty_prompt_rejected() {
        let adapter = GoogleVeoAdapter::new(stub_client(), VeoModel::Fast);
        let req = Txt2VidRequest {
            prompt: "  ".into(),
            negative_prompt: None,
            apply_default_negatives: true,
            duration_secs: 4.0,
            aspect_ratio: "16:9".into(),
            seed: None,
        };
        let err =
            <GoogleVeoAdapter as Txt2VidGenBackend>::generate(&adapter, &req, RunMode::DryRun)
                .unwrap_err();
        assert!(matches!(err, BackendError::InvalidRequest(_)));
    }

    #[test]
    fn remote_url_in_image_rejected_with_clear_message() {
        let adapter = GoogleVeoAdapter::new(stub_client(), VeoModel::Fast);
        let req = Img2VidRequest {
            prompt: "push in".into(),
            image: "https://example.com/x.png".into(),
            last_frame_url: None,
            negative_prompt: None,
            apply_default_negatives: true,
            duration_secs: 4.0,
            aspect_ratio: "16:9".into(),
            seed: None,
        };
        let err = <GoogleVeoAdapter as Img2VidGenBackend>::generate(
            &adapter,
            &req,
            RunMode::Live { max_cost_usd: 10.0 },
        )
        .unwrap_err();
        match err {
            BackendError::InvalidRequest(msg) => assert!(msg.contains("local path")),
            other => panic!("expected InvalidRequest, got {other:?}"),
        }
    }

    #[test]
    fn body_shape_matches_google_wire() {
        let req = Txt2VidRequest {
            prompt: "a cat".into(),
            negative_prompt: Some("blurry".into()),
            apply_default_negatives: true,
            duration_secs: 8.0,
            aspect_ratio: "16:9".into(),
            seed: Some(7),
        };
        let body = VeoBody::for_txt2vid(&req);
        let v = serde_json::to_value(&body).unwrap();
        assert_eq!(v["instances"][0]["prompt"], "a cat");
        assert_eq!(v["parameters"]["aspectRatio"], "16:9");
        assert_eq!(v["parameters"]["durationSeconds"], 8.0);
        assert_eq!(v["parameters"]["negativePrompt"], "blurry");
        assert_eq!(v["parameters"]["seed"], 7);
    }
}
