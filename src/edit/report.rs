//! JSON report shape emitted alongside the shipped MP4.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use super::plan::Plan;
use super::review::Verdict;

/// One attempt of the plan→execute→review loop.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AttemptOutcome {
    /// 1-indexed attempt number.
    pub n: u32,
    /// Plan the planner emitted for this attempt.
    pub plan: Plan,
    /// Reviewer's verdict. `None` when the executor failed before a
    /// reviewable artifact was produced.
    pub review: Option<Verdict>,
    /// Path to the rendered MP4 (the executor's output). `None` when
    /// execution failed.
    pub output_path: Option<PathBuf>,
    /// USD cost the planner estimated for this attempt.
    pub cost_estimate_usd: f32,
    /// Error message if this attempt failed, otherwise `None`.
    pub error: Option<String>,
}

/// End-to-end edit result. Serialized to the report path.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EditResult {
    /// The input path the user provided.
    pub input: PathBuf,
    /// The natural-language intent.
    pub intent: String,
    /// Path to the shipped MP4 (whichever attempt won).
    pub shipped: Option<PathBuf>,
    /// Score of the shipped attempt.
    pub shipped_score: Option<f32>,
    /// 1-indexed attempt number that produced the shipped output.
    pub shipped_attempt: Option<u32>,
    /// Every attempt the loop made.
    pub attempts: Vec<AttemptOutcome>,
    /// Sum of planner cost estimates across attempts.
    pub total_cost_usd: f32,
    /// Wall-clock duration of the whole loop.
    pub total_wall_ms: u128,
    /// Optional operator-facing note explaining why the loop
    /// terminated short of `pass`.
    pub note: Option<String>,
}
