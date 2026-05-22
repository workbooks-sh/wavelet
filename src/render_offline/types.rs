//! `render_offline::types` — extracted from godfile split.

#![allow(missing_docs)]

use std::path::{Path, PathBuf};
use crate::audio::AudioCue;
use crate::video::VideoError;
use super::stats::RenderOfflineError;

/// Default per-frame wall-clock budget. If a single frame takes longer
/// than this, the render aborts with [`RenderOfflineError::FrameBudgetExceeded`].
///
/// 30 seconds is roomy: a 1080p frame in CPU-Vello takes ~200ms-2s on
/// pathological CSS; anything past 30s is decode/style/paint hang.
pub const DEFAULT_FRAME_BUDGET_SECS: u64 = 30;

/// One scene placed on the timeline. Resolves relative `html_path` against the
/// composition root directory passed to [`render_composition`].
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct SceneSpec {
    /// Path to the scene's HTML file, relative to the composition root.
    pub html_path: PathBuf,
    /// First frame the scene is visible on (inclusive).
    pub start_frame: u32,
    /// Number of frames the scene is visible.
    pub duration_frames: u32,
    /// Optional shader transition into this scene. When present, the first
    /// `transition_in.duration_secs` of the scene's timeline are rendered
    /// as a wavelet_fx-driven crossfade/wipe/etc. from the previous scene's last
    /// settled frame to this scene's content.
    #[serde(default)]
    pub transition_in: Option<TransitionSpec>,

    /// Optional MP4 path used as the visible background under the HTML
    /// overlay. When set, the HTML is rendered with a transparent
    /// background and alpha-composited over the video clip at the scene's
    /// local time. The clip loops if the scene is longer than the clip.
    #[serde(default)]
    pub video_bg: Option<PathBuf>,
}

/// Declares a shader transition into a scene. Currently only inline wavelet_fx
/// source is supported; file-source (`src: "transitions/wipe.wavelet_fx"`) is a
/// small follow-on once we hit a use case for reusable transitions.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TransitionSpec {
    /// Inline wavelet_fx source expression. Must produce a 2-input transition
    /// — `src(0)` = previous scene, `src(1)` = this scene. The `progress`
    /// CSS prop (0..1) is bound by the orchestrator.
    pub wavelet_fx: String,
    /// Transition window length in seconds.
    pub duration_secs: f32,
}

/// One audio cue scheduled against the composition timeline.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AudioCueSpec {
    /// Path to an audio asset (any symphonia-supported container).
    pub asset_path: PathBuf,
    /// Unique cue id — used by ducking targets to reference this cue.
    pub id: String,
    /// First frame this cue starts producing audio.
    pub start_frame: u32,
    /// Cue duration in frames.
    pub duration_frames: u32,
    /// Linear gain multiplier (1.0 = unity).
    pub volume: f32,
    /// Stereo pan (-1.0 = full left, 0 = center, +1.0 = full right).
    pub pan: f32,
    /// Fade-in length in frames (linear ramp 0 → volume).
    pub fade_in_frames: u32,
    /// Fade-out length in frames (linear ramp volume → 0).
    pub fade_out_frames: u32,
    /// Cue ids to duck while this cue is playing.
    pub duck_targets: Vec<String>,
    /// Ducking depth in decibels (positive = louder reduction).
    pub duck_db: f32,
    /// When true, the mixer snaps `start_frame` to the nearest music
    /// onset within ±0.3s. Default false.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub align_to_beat: bool,
}

/// Whole composition input.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Composition {
    /// Output frame width in pixels.
    pub width: u32,
    /// Output frame height in pixels.
    pub height: u32,
    /// Frame rate (frames per second).
    pub fps: u32,
    /// Total composition length in frames.
    pub duration_frames: u32,
    /// Aspect ratio for this composition. Informational at the moment —
    /// drives the safe-area math in [`crate::aspect::safe_areas`] and
    /// the future multi-aspect render (wb-lnhl). `width` / `height`
    /// remain the source of truth for the actual pixel dimensions.
    /// Absent in older `comp.json` files; defaults to `None` for
    /// backwards compatibility (callers should treat that as 16:9).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub aspect: Option<crate::aspect::AspectRatio>,
    /// Scenes laid out on the timeline.
    pub scenes: Vec<SceneSpec>,
    /// Audio cues laid out on the timeline.
    #[serde(default)]
    pub audio_cues: Vec<AudioCueSpec>,
}

impl Composition {
    /// Parse a composition from a JSON file. Relative paths inside the file
    /// are resolved against the file's parent directory; that directory is
    /// returned alongside the composition for the caller to pass to
    /// [`render_composition`].
    pub fn from_json_path(
        path: impl AsRef<Path>,
    ) -> Result<(Self, PathBuf), RenderOfflineError> {
        let p = path.as_ref();
        let json = std::fs::read_to_string(p)?;
        let comp: Composition = serde_json::from_str(&json).map_err(|e| {
            RenderOfflineError::Io(std::io::Error::new(std::io::ErrorKind::InvalidData, e))
        })?;
        let dir = p.parent().unwrap_or(Path::new(".")).to_path_buf();
        Ok((comp, dir))
    }
}
