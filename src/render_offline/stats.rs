//! `render_offline::stats` — extracted from godfile split.

#![allow(missing_docs)]

use std::path::PathBuf;
use crate::video::VideoError;
use super::types::DEFAULT_FRAME_BUDGET_SECS;

/// Summary of a completed render, for diagnostics.
#[derive(Debug, Clone, Default)]
pub struct RenderStats {
    /// Total frames pushed to the encoder.
    pub video_frames: u64,
    /// Total audio samples (per channel) rendered.
    pub audio_samples_per_channel: u64,
    /// Wall-clock duration of the render, in milliseconds.
    pub elapsed_ms: u128,
    /// Output MP4 file size in bytes.
    pub mp4_bytes: u64,
    /// Output WAV file size in bytes (0 if no audio cues).
    pub wav_bytes: u64,
}

/// Errors that can happen during offline render.
#[derive(Debug, thiserror::Error)]
pub enum RenderOfflineError {
    /// File I/O failure (scene HTML missing, output path unwritable).
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    /// Video encode error (rsmpeg/libav).
    #[error("video: {0}")]
    Video(#[from] VideoError),
    /// Audio decode / mix error (symphonia/rubato).
    #[error("audio: {0}")]
    Audio(#[from] crate::audio::AudioError),
    /// Composition references a frame past its declared duration.
    #[error("scene '{0}' extends past composition duration ({1} > {2} frames)")]
    SceneOverflow(String, u32, u32),
    /// One or more referenced assets (inline <video src>, <audio src>,
    /// scene.video_bg, or comp.audio_cues paths) don't exist on disk.
    /// Caught by the pre-flight pass before the encoder opens — agents
    /// get a structured failure instead of a 9-minute hang.
    #[error("missing assets referenced by composition: {0:?}")]
    MissingAssets(Vec<PathBuf>),
    /// No frame was pushed to the encoder within the per-frame budget.
    /// Indicates render is hung — pathological CSS, decode deadlock,
    /// or Blitz/Stylo/Vello bug. Lets the caller (agent or harness)
    /// fail-fast and try a different approach instead of waiting
    /// for the eval-level timeout.
    #[error("frame budget exceeded: frame {frame_index} took longer than {budget_secs}s (last successful frame: {last_frame_index})")]
    FrameBudgetExceeded {
        /// Frame index the budget was exceeded on (zero-based).
        frame_index: u32,
        /// The configured budget in seconds.
        budget_secs: u64,
        /// Index of the last successfully pushed frame, or `-1` if no
        /// frame had completed before the budget tripped.
        last_frame_index: i64,
    },
}

/// Options that tune [`render_composition`] behavior without changing
/// the composition itself. Kept as a struct so we can grow it without
/// churning every call site.
#[derive(Debug, Clone)]
pub struct RenderOptions {
    /// Maximum wall-clock seconds allowed for a single frame. See
    /// [`DEFAULT_FRAME_BUDGET_SECS`] for the default.
    pub frame_budget_secs: u64,
    /// Mux the rendered audio buffer into the MP4 as an AAC stream.
    /// Default ON: a `<audio>` reference in the HTML produces a single
    /// MP4 with both video and audio. Setting this to false leaves the
    /// MP4 video-only (the sidecar WAV is still written) — useful when
    /// the caller wants to mux audio manually downstream.
    pub mux_audio: bool,
}

impl Default for RenderOptions {
    fn default() -> Self {
        Self {
            frame_budget_secs: DEFAULT_FRAME_BUDGET_SECS,
            mux_audio: true,
        }
    }
}
