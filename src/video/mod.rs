//! Video encode — RGBA frames in, MP4 out.
//!
//! Thin glue around rsmpeg (system FFmpeg 8.x). Per the v3 plan: no custom
//! codec code, no container muxing math — libav handles everything. This
//! module is ~250 LOC of bookkeeping.
//!
//! ## Typical use
//!
//! ```ignore
//! use crate::video::{VideoEncoder, RgbaFrame, Codec};
//! let mut enc = VideoEncoder::open("/tmp/out.mp4", 1280, 720, 30, Codec::H264)?;
//! for frame_rgba in frames {
//!     enc.push_frame(&RgbaFrame::new(1280, 720, frame_rgba))?;
//! }
//! enc.finalize()?;
//! ```
//!
//! ## Why mp4-trailer matters
//!
//! `finalize()` writes the moov atom (the MP4 trailer). Without it the file
//! is unplayable — pause-mid-render and the artifact is corrupt. The `Drop`
//! impl is a best-effort safety net but callers should still call
//! `finalize()` explicitly.

pub mod codec;
pub mod encoder;
pub mod errors;
pub mod frame;

pub use codec::{hw_decoders, Codec};
pub use encoder::VideoEncoder;
pub use errors::VideoError;
pub use frame::RgbaFrame;
