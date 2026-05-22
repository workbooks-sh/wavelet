//! Typed representation of a `wavelet shot edit` request.
//!
//! The CLI lowers user flags into one of these structs; everything
//! downstream (planner, executor, reviewer, loop) takes typed input
//! only.

use std::path::PathBuf;

/// Whether the input is an already-rendered MP4 or a scene HTML that
/// can still be re-rendered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InputKind {
    /// Rendered MP4 — can only be processed by tools that accept
    /// pixels (Veo regen, OmniEdit, Composite). CSS-only edits are
    /// only available when a sibling scene HTML can be located.
    Mp4,
    /// Scene HTML — full CSS-only path is available.
    SceneHtml,
}

impl InputKind {
    /// Classify by extension.
    pub fn classify(path: &std::path::Path) -> Option<Self> {
        let ext = path.extension()?.to_str()?.to_ascii_lowercase();
        match ext.as_str() {
            "mp4" | "mov" | "webm" => Some(InputKind::Mp4),
            "html" | "htm" => Some(InputKind::SceneHtml),
            _ => None,
        }
    }
}

/// All knobs the loop needs to run an edit.
#[derive(Debug, Clone)]
pub struct EditConfig {
    /// Maximum number of plan→execute→review attempts. The loop ships
    /// the highest-scoring attempt if all of them fail the threshold.
    pub max_attempts: u32,
    /// Aggregate USD budget across all attempts. When the next
    /// attempt's estimated cost would push above this, the loop
    /// exits with `note: "exhausted budget"`.
    pub max_cost_usd: f32,
    /// Score (0..1) at which the reviewer's verdict is accepted as
    /// "ship it".
    pub pass_threshold: f32,
    /// Gemini model slug the planner runs on (default
    /// `gemini-3.1-pro-preview`).
    pub planner_model: String,
    /// Gemini model slug the reviewer runs on (default
    /// `gemini-3.5-flash`).
    pub reviewer_model: String,
    /// Where the shipped MP4 is written.
    pub out_path: PathBuf,
    /// Where the JSON report is written.
    pub report_path: PathBuf,
    /// Dry-run mode — plan only, do not execute or review.
    pub dry_run: bool,
}

/// Top-level request the loop runs.
#[derive(Debug, Clone)]
pub struct EditRequest {
    /// Path to the input. May be `.mp4` (`InputKind::Mp4`) or `.html`
    /// (`InputKind::SceneHtml`).
    pub input: PathBuf,
    /// Whether the input is rendered video or a scene file.
    pub kind: InputKind,
    /// Natural-language edit instruction provided by the user.
    pub intent: String,
    /// Loop configuration.
    pub cfg: EditConfig,
}
