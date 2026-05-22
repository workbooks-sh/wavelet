//! Video-generation clusters — two prompting shapes share the
//! "shot generation" category:
//!
//! - **`Txt2VidGen`** (this trait) — text prompt + duration + aspect.
//!   Members: Wan-T2V, CogVideoX, Hunyuan, LTX, Mochi.
//! - **`Img2VidGen`** (future trait) — still + motion_prompt.
//!   Members: Runway Gen-3, Kling, Pika, Luma.
//!
//! The two clusters share the same `VideoResult` shape so downstream
//! tools (compositor, renderer) consume them uniformly.

use crate::backends::{BackendCallOutcome, BackendError, CostEstimate, RunMode};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Cluster identifier for `Txt2VidGen` — used in cache keys.
pub const CLUSTER_TXT2VID: &str = "video_txt2vid";

/// Cluster identifier for `Img2VidGen` — used in cache keys.
pub const CLUSTER_IMG2VID: &str = "video_img2vid";

/// Cluster identifier for `LipSync` — used in cache keys.
pub const CLUSTER_LIPSYNC: &str = "video_lipsync";

/// Cluster identifier for `MultiRefVideoGen` — used in cache keys.
pub const CLUSTER_MULTI_REF_VIDEO: &str = "video_multi_ref";

/// One text-to-video request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Txt2VidRequest {
    /// Text prompt describing the shot.
    pub prompt: String,
    /// Optional negative prompt (things to avoid). Merged with the
    /// canonical default per wb-ynn0 unless `apply_default_negatives`
    /// is `false`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub negative_prompt: Option<String>,
    /// Append the canonical default negative prompt to every call
    /// (wb-ynn0). Default `true`; set false for the CLI's rare
    /// `--no-default-negatives` escape hatch.
    #[serde(default = "default_apply_negatives")]
    pub apply_default_negatives: bool,
    /// Desired duration in seconds. Each model has a cap; adapters
    /// clamp as needed.
    #[serde(default = "default_duration")]
    pub duration_secs: f32,
    /// Aspect ratio (`"16:9"`, `"9:16"`, `"1:1"`). Some models accept
    /// exact pixel dimensions instead.
    #[serde(default = "default_aspect")]
    pub aspect_ratio: String,
    /// Optional random seed for reproducibility.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seed: Option<u64>,
}

fn default_duration() -> f32 {
    5.0
}
fn default_aspect() -> String {
    "16:9".into()
}
fn default_apply_negatives() -> bool {
    true
}

impl Txt2VidRequest {
    /// Build a minimum-viable request.
    pub fn new(prompt: impl Into<String>) -> Self {
        Self {
            prompt: prompt.into(),
            negative_prompt: None,
            apply_default_negatives: default_apply_negatives(),
            duration_secs: default_duration(),
            aspect_ratio: default_aspect(),
            seed: None,
        }
    }
}

/// Result of a video-gen. Shared across `Txt2VidGen` and (future)
/// `Img2VidGen` clusters.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VideoResult {
    /// Provider identifier (`fal-wan-t2v`, `fal-cogvideox`, …).
    pub provider: String,
    /// Cached video path on disk.
    pub video_path: PathBuf,
    /// Video file size in bytes.
    pub video_bytes: u64,
    /// Mime type of the container.
    pub mime: String,
    /// Duration the backend produced, in seconds.
    pub duration_secs: f32,
    /// Width × height the backend produced.
    pub width: u32,
    /// Pixel height.
    pub height: u32,
    /// Prompt actually sent (after any adapter-side rewriting).
    pub prompt_sent: String,
    /// Random seed the backend reports using (when surfaced).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seed_used: Option<u64>,
}

/// One image-to-video request — animates a still via a text motion
/// prompt. Optional `last_frame_url` triggers dual-keyframe mode on
/// supporting backends (Kling O1, Veo 3.1) — used by the storyboard
/// frame-chaining path so the last frame of shot N becomes the first
/// frame of shot N+1.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Img2VidRequest {
    /// Motion prompt — describes what the still should *do* (push in,
    /// dolly left, particles fall, etc.).
    pub prompt: String,
    /// Path or URL to the source still. This is the **first** frame
    /// when dual-keyframe is in play. Adapters that need to upload will
    /// hash and forward the bytes; URL-accepting providers pass it
    /// through directly.
    pub image: String,
    /// Optional URL/data-URI of the **last** frame for backends that
    /// support dual-keyframe i2v (Kling O1 `end_image_url`,
    /// hypothetical Veo 3.1 `last_frame_url`). Backends that don't
    /// support it ignore the field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_frame_url: Option<String>,
    /// Optional negative prompt (motion to avoid). Merged with the
    /// canonical default per wb-ynn0 unless `apply_default_negatives`
    /// is `false`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub negative_prompt: Option<String>,
    /// Append the canonical default negative prompt to every call
    /// (wb-ynn0). Default `true`; set false for the CLI's rare
    /// `--no-default-negatives` escape hatch.
    #[serde(default = "default_apply_negatives")]
    pub apply_default_negatives: bool,
    /// Desired duration in seconds. Models clamp to their max.
    #[serde(default = "default_duration")]
    pub duration_secs: f32,
    /// Aspect ratio hint.
    #[serde(default = "default_aspect")]
    pub aspect_ratio: String,
    /// Optional random seed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seed: Option<u64>,
}

impl Img2VidRequest {
    /// Build a minimum-viable request from an image source + motion
    /// prompt.
    pub fn new(image: impl Into<String>, prompt: impl Into<String>) -> Self {
        Self {
            image: image.into(),
            prompt: prompt.into(),
            last_frame_url: None,
            negative_prompt: None,
            apply_default_negatives: default_apply_negatives(),
            duration_secs: default_duration(),
            aspect_ratio: default_aspect(),
            seed: None,
        }
    }

    /// Builder-style — set the chain-to last frame.
    pub fn with_last_frame(mut self, url: impl Into<String>) -> Self {
        self.last_frame_url = Some(url.into());
        self
    }
}

/// Cluster trait shared by every image-to-video adapter.
pub trait Img2VidGenBackend {
    /// Provider name (`"fal-wan-i2v"`, …).
    fn name(&self) -> &'static str;

    /// Estimate the cost.
    fn estimate_cost(&self, request: &Img2VidRequest) -> CostEstimate;

    /// Animate the still. Returns the cached video path.
    fn generate(
        &self,
        request: &Img2VidRequest,
        mode: RunMode,
    ) -> Result<BackendCallOutcome<VideoResult>, BackendError>;
}

/// Cluster trait shared by every text-to-video adapter.
pub trait Txt2VidGenBackend {
    /// Provider name (`"fal-wan-t2v"`, …).
    fn name(&self) -> &'static str;

    /// Estimate the cost. Most providers in this cluster bill per
    /// clip or per second.
    fn estimate_cost(&self, request: &Txt2VidRequest) -> CostEstimate;

    /// Generate the video. The cached path is in `VideoResult.video_path`.
    fn generate(
        &self,
        request: &Txt2VidRequest,
        mode: RunMode,
    ) -> Result<BackendCallOutcome<VideoResult>, BackendError>;
}

/// One lip-sync request — graft an audio track onto a driving video
/// so the speaker's mouth matches the new dialogue. Inputs may be
/// HTTPS URLs (preferred — the provider fetches directly) or local
/// paths (the adapter base64-encodes before upload).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LipSyncRequest {
    /// Driving video — the visual track. URL or local path.
    pub video: String,
    /// Replacement audio (dialogue + optional ambient mix). URL or
    /// local path.
    pub audio: String,
    /// Sync mode hint — providers vary in vocabulary (`"loop"`,
    /// `"bounce"`, `"cut_off"`, `"silence"` for sync.so). `None` lets
    /// the adapter pick its default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sync_mode: Option<String>,
    /// Optional generation temperature. Higher = more expressive but
    /// less faithful to the audio. Providers map to their own scale.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    /// When `true`, the adapter hints to the provider that only one
    /// face in the frame is speaking (better quality on multi-face
    /// shots). Defaults to provider's own default when `None`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_speaker: Option<bool>,
}

impl LipSyncRequest {
    /// Build a minimum-viable request.
    pub fn new(video: impl Into<String>, audio: impl Into<String>) -> Self {
        Self {
            video: video.into(),
            audio: audio.into(),
            sync_mode: None,
            temperature: None,
            active_speaker: None,
        }
    }
}

/// One multi-reference video gen request. Used for subject-locked /
/// multi-control video — the model conditions on every URL in
/// `reference_images` simultaneously. Depth maps + canny edges +
/// pose stacks live alongside subject refs in the same array; the
/// model decides how to weight them.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MultiRefVideoRequest {
    /// Text prompt describing the shot.
    pub prompt: String,
    /// 1–N reference images. Public HTTPS URLs preferred; some
    /// adapters accept local paths (base64-encoded inline).
    pub reference_images: Vec<String>,
    /// Reference videos (when applicable — Wan 2.7 R2V supports up
    /// to 2). Same URL conventions as `reference_images`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub reference_videos: Vec<String>,
    /// Optional negative prompt.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub negative_prompt: Option<String>,
    /// Duration in seconds. Models clamp to their max.
    #[serde(default = "default_duration")]
    pub duration_secs: f32,
    /// Aspect ratio (`"16:9"`, `"9:16"`, `"1:1"`).
    #[serde(default = "default_aspect")]
    pub aspect_ratio: String,
    /// Optional random seed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seed: Option<u64>,
}

impl MultiRefVideoRequest {
    /// Build a minimum-viable request.
    pub fn new(prompt: impl Into<String>, reference_images: Vec<String>) -> Self {
        Self {
            prompt: prompt.into(),
            reference_images,
            reference_videos: Vec::new(),
            negative_prompt: None,
            duration_secs: default_duration(),
            aspect_ratio: default_aspect(),
            seed: None,
        }
    }
}

/// Cluster trait shared by every multi-ref video adapter (Wan 2.7
/// R2V, hypothetical VACE adapters, …).
pub trait MultiRefVideoGenBackend {
    /// Provider name.
    fn name(&self) -> &'static str;
    /// Cost estimate.
    fn estimate_cost(&self, request: &MultiRefVideoRequest) -> CostEstimate;
    /// Generate the video.
    fn generate(
        &self,
        request: &MultiRefVideoRequest,
        mode: RunMode,
    ) -> Result<BackendCallOutcome<VideoResult>, BackendError>;
}

/// Cluster trait shared by every lip-sync adapter.
pub trait LipSyncBackend {
    /// Provider name.
    fn name(&self) -> &'static str;
    /// Estimate the cost (typically per-second of output).
    fn estimate_cost(&self, request: &LipSyncRequest) -> CostEstimate;
    /// Run the lip-sync. Returns the cached output video.
    fn sync(
        &self,
        request: &LipSyncRequest,
        mode: RunMode,
    ) -> Result<BackendCallOutcome<VideoResult>, BackendError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_round_trips() {
        let req = Txt2VidRequest {
            prompt: "a saguaro at dawn".into(),
            negative_prompt: Some("blurry, low quality".into()),
            apply_default_negatives: true,
            duration_secs: 5.0,
            aspect_ratio: "16:9".into(),
            seed: Some(42),
        };
        let json = serde_json::to_string(&req).unwrap();
        let back: Txt2VidRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(back.prompt, "a saguaro at dawn");
        assert_eq!(back.seed, Some(42));
    }

    #[test]
    fn defaults_are_useful() {
        let req = Txt2VidRequest::new("test");
        assert_eq!(req.duration_secs, 5.0);
        assert_eq!(req.aspect_ratio, "16:9");
        assert!(req.negative_prompt.is_none());
    }

    #[test]
    fn img2vid_last_frame_round_trips() {
        let req = Img2VidRequest::new("https://x/a.png", "push in")
            .with_last_frame("https://x/b.png");
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains("last_frame_url"));
        let back: Img2VidRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(back.last_frame_url.as_deref(), Some("https://x/b.png"));
    }

    #[test]
    fn img2vid_last_frame_omitted_when_none() {
        let req = Img2VidRequest::new("https://x/a.png", "push in");
        let json = serde_json::to_string(&req).unwrap();
        assert!(!json.contains("last_frame_url"));
    }

    #[test]
    fn img2vid_missing_last_frame_field_deserializes() {
        let body = r#"{"prompt":"p","image":"https://x/a.png"}"#;
        let req: Img2VidRequest = serde_json::from_str(body).unwrap();
        assert!(req.last_frame_url.is_none());
        assert_eq!(req.duration_secs, 5.0);
    }
}
