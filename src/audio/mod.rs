//! Audio — multi-cue mixer aligned to the video timebase.
//!
//! Decodes via symphonia (pure-Rust — every codec MP3/AAC/FLAC/Opus/Vorbis/WAV),
//! resamples via rubato, sums to stereo f32 aligned to the output frame rate.
//! The mixer math (per-cue gain + pan + fade + ducking) is hand-rolled —
//! ~250 LOC of glue around two well-trodden crates.
//!
//! ## Typical use
//!
//! ```ignore
//! use gamut_engine::audio::{AudioCue, AudioMixer};
//!
//! let mut mixer = AudioMixer::new(48_000, 30);  // sample rate, fps
//! mixer.add_cue(AudioCue {
//!     asset_path: "vo.mp3".into(),
//!     start_frame: 30, duration_frames: 600,
//!     volume: 1.0, pan: 0.0,
//!     fade_in_frames: 9, fade_out_frames: 15,
//!     duck_targets: vec!["music".into()], duck_db: 12.0,
//!     id: "narration".into(),
//! })?;
//! let stereo: Vec<f32> = mixer.render(900)?;
//! ```

pub mod cue;
pub mod decoder;
pub mod errors;
pub mod mixer;
pub mod resample;

pub use cue::AudioCue;
pub use decoder::DecodedAudio;
pub use errors::AudioError;
pub use mixer::AudioMixer;
