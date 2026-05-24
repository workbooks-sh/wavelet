//! Query rendered compositions for agent feedback.
//!
//! This module exposes pixel + scene-graph + spec queries against any
//! composition at any time *t*. The first surface (Phase 1, this module)
//! is scene-graph queries — questions that answer from the resolved Blitz
//! layout tree without touching rendered pixels. Most agent questions
//! can be answered cheaply this way: "where is `#headline` at 0.5s?",
//! "is `#stack` visible?", "did the chip's translate propagate to its
//! text children?".
//!
//! See epic wb-q4a6 and phase wb-k85o for the broader plan.

pub mod beat;
pub mod diff;
pub mod glyph_run;
pub mod ocr;
pub mod pixels;
pub mod repl;
pub mod scene_graph;
pub mod snapshot;

pub use beat::{check as on_beat, events_from_composition, CompositionEvent, OnBeatResult, ScoredEvent};
pub use diff::{diff_videos, DiffMetric, DiffOptions, DiffResult, FrameDiff};
pub use ocr::{text_visible, TextVisibleResult};
pub use repl::run as run_repl;
pub use pixels::{
    banding, color_at, color_in, contrast, BandingResult, ColorAtResult, ColorInResult,
    ContrastResult, FramePixels,
};
pub use scene_graph::{
    bbox_of, in_safe_area, no_overlap, transform_inherits, visibility_of, BboxResult,
    OverlapPair, OverlapResult, SafeAreaResult, TransformInheritsResult,
};
pub use glyph_run::{GlyphInk, GlyphRunData};
pub use snapshot::{FlexAxis, FrameSnapshot, NodeSnapshot, Rect, VisibilityVerdict};
