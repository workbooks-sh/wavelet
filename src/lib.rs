//! # wavelet-engine — motion-graphics render engine (v3)
//!
//! Clean re-implementation against upstream Blitz + Animato. Replaces the
//! earlier `packages/wavelet-rust/` workspace which depended on RVST's
//! vendored fork of Blitz (the modifications in that fork caused
//! position-absolute + percentage-offset layout to silently fail).
//!
//! ## Stack
//!
//! - **HTML + CSS + layout**: upstream Blitz (`blitz-dom` + `blitz-html` +
//!   `blitz-paint`) from crates.io. Stylo for CSS, Taffy for layout, Parley
//!   for text shaping.
//! - **GPU render**: `anyrender_vello_cpu` on CPU (no GPU adapter needed —
//!   offline render is rate-limited by encode anyway).
//! - **Motion timeline**: upstream `animato` from crates.io. Tween, Timeline,
//!   stagger, seek_abs, 38+ easings.
//! - **Video encode**: rsmpeg + system FFmpeg (added in a follow-on phase).
//! - **Audio mix**: symphonia + rubato (added in a follow-on phase).
//! - **Compositor**: wgpu + WGSL shaders for CSS effects RVST/Blitz don't
//!   render natively (filter, blend-mode, mask, clip-path) — added in a
//!   follow-on phase.
//!
//! ## Dependency policy
//!
//! Path-deps on vendored crates are only allowed where upstream is not
//! yet released on crates.io (currently: `vendor/blitz-paint`, `vendor/stylo`).
//! Everything else uses crates.io version pins.

#![deny(missing_docs)]

pub mod agent;
pub mod aspect;
/// Depth estimation for background/foreground discrimination.
/// Gated behind the `depth` Cargo feature.
pub mod depth;
pub mod audio;
pub mod backends;
#[path = "c2pa/mod.rs"]
pub mod c2pa_credentials;
pub mod clip;
pub mod clipref;
pub mod compose;
pub mod config;
pub mod css_filter;
pub mod director;
pub mod edit;
pub mod grammar;
/// CLI argument types — clap enums + structs for the wavelet binary.
pub mod cli_args;
pub mod handlers;
pub mod image_analysis;
/// `wavelet lint` rules + report types.
pub mod lint;
/// ONNX-based OCR engine. Requires `--features ocr` to activate inference;
/// compiles to a no-op stub when the feature is absent.
pub mod ocr;
pub mod inline_video;
pub mod pipelines;
pub mod prompts;
pub mod query;
pub mod render;
pub mod render_offline;
pub mod screenplay;
pub mod shader;
pub mod storyboard;
pub mod variants;
pub mod velocity;
pub mod verify;
pub mod video;

/// Crate-internal test plumbing. Not exposed.
#[cfg(test)]
pub(crate) mod test_utils {
    use std::sync::{LazyLock, Mutex};

    /// Process-wide mutex serializing Blitz `HtmlDocument` construction
    /// during tests. Parley's font registration uses global state that
    /// races when many tests build documents in parallel; the mutex makes
    /// each test's `load_html_with_base` / `FrameSnapshot::at` block its
    /// peers. Cost is negligible (one cheap lock per test entry).
    pub static BLITZ_GUARD: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));
}
