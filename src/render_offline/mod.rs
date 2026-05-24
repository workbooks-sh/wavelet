//! Offline orchestrator — walk a composition spec, produce an MP4.
//!
//! Given a [`Composition`] describing a series of scenes (HTML files with
//! CSS animations) and audio cues, this module:
//!
//! 1. Opens a [`VideoEncoder`] at the composition's resolution + framerate.
//! 2. For each frame, identifies the active scene, advances Stylo's CSS
//!    animation clock via `BaseDocument::resolve(now)`, renders the document
//!    via Blitz/Vello-CPU, and pushes the RGBA buffer into the encoder.
//! 3. Builds an [`AudioMixer`] from the composition's cues, renders the full
//!    audio buffer.
//! 4. Writes a stereo PCM WAV alongside the MP4. (Container-side audio mux
//!    into the same MP4 is a follow-on — the rsmpeg encoder is video-only
//!    today.)
//!
//! Scope intentionally narrow: in-memory composition struct, no JSON IR
//! parsing here — that lives in the CLI (Phase 6). Per the v3 plan: clean
//! and DRY, single crate.
//!
//! ## Typical use
//!
//! ```ignore
//! use crate::render_offline::{render_composition, Composition, SceneSpec};
//! use std::path::PathBuf;
//!
//! let comp = Composition {
//!     width: 1280, height: 720, fps: 30, duration_frames: 60,
//!     scenes: vec![SceneSpec {
//!         html_path: PathBuf::from("scenes/title.html"),
//!         start_frame: 0, duration_frames: 60,
//!     }],
//!     audio_cues: vec![],
//! };
//! let stats = render_composition(&comp, &PathBuf::from("."), &PathBuf::from("out.mp4"))?;
//! ```


pub mod audio_mux;
pub mod types;
pub mod stats;
pub mod render;
pub mod scene;
pub mod utils;

pub use types::*;
pub use stats::*;
pub use render::*;
