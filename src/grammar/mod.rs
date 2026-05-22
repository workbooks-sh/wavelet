//! Filmmaking grammar — continuity check + transition classifier.
//!
//! Phase 7 of the screenplay-to-MP4 epic (wb-iv3c → wb-z5ys). Implements
//! the rules from the PRD §5:
//!
//! - **180° rule**: every shot in a scene with an action line stays on
//!   one side of the line (or `Center` for establishing). When the
//!   camera flips sides between shots, it must do so via a deliberate
//!   transition (smash cut, jump cut, whip pan).
//! - **Motion continuity**: adjacent shots' motion vectors should
//!   continue (within ±30°) unless the transition is jarring.
//! - **Shot-type rhythm**: avoid runs of identical shot types; alternate
//!   wide/medium/close for visual variety.
//! - **Transition classification**: read screenplay transitions and a
//!   velocity profile, propose richer specs (fill in durations, whip-pan
//!   directions, J/L lead/trail).

use serde::{Deserialize, Serialize};

pub mod continuity;
pub mod transitions;

pub use continuity::{check_continuity, ContinuityReport, CutFinding, CutSeverity};
pub use transitions::{classify_transitions, ClassifiedTransition, TransitionClassification};

/// Default angular tolerance (degrees) for the motion-continuity check.
pub const MOTION_CONTINUITY_TOLERANCE_DEG: f32 = 30.0;

/// Default max run length for identical shot types before the rhythm
/// gate complains.
pub const SHOT_TYPE_RUN_LIMIT: usize = 2;

/// Severity shared by grammar findings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GrammarSeverity {
    /// Hard rule violation — the cut breaks accepted grammar.
    Error,
    /// Soft hint — likely undesirable but situationally valid.
    Warning,
    /// Informational note — kept in the report so the agent can reason
    /// about every cut.
    Info,
}
