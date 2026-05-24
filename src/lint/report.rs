//! Shared lint-report types. Every rule emits `LintFinding`s; the
//! orchestrator collects them into one `LintReport`.

use crate::query::Rect;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Severity of a finding. Anything `Error` triggers a non-zero exit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    /// Blocking — the lint as a whole exits non-zero if any is present.
    Error,
    /// Worth flagging but not a hard failure.
    Warn,
    /// Informational only.
    Info,
}

impl Severity {
    /// Uppercase tag for text output.
    pub fn label(self) -> &'static str {
        match self {
            Severity::Error => "ERROR",
            Severity::Warn => "WARN ",
            Severity::Info => "INFO ",
        }
    }
}

/// One lint finding. Stable across rules so the report formatter can
/// emit a single uniform format.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LintFinding {
    /// Identifier for the rule that produced this finding. `safe-zone`,
    /// `glyph-clip`, etc.
    pub rule: String,
    /// Severity bucket.
    pub severity: Severity,
    /// Scene HTML file the finding came from.
    pub scene_path: PathBuf,
    /// Time within the scene the snapshot was taken at, in seconds.
    pub t_secs: f32,
    /// Best-effort selector — `#id` if present, `.class` otherwise,
    /// or `tag[n]` as a last resort.
    pub element_selector: String,
    /// The element's layout bbox (post-flow, pre-CSS-transform).
    pub element_bbox: Rect,
    /// One-line human-readable summary of the violation.
    pub message: String,
    /// Concrete remediation guidance the agent (or human) can act on.
    pub fix_hint: String,
    /// Optional discriminator within a rule — e.g. `cap-height` vs
    /// `contrast` for the `text-readability` rule. Lets the dedup logic
    /// in `handlers/lint.rs` keep two independent findings against the
    /// same element when they describe meaningfully different defects.
    /// Rules without sub-categories leave this unset.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subkind: Option<String>,
}

/// Top-level lint output.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LintReport {
    /// Number of scene files inspected.
    pub scenes_checked: usize,
    /// Rule identifiers that ran. Ordering matches CLI input.
    pub rules_run: Vec<String>,
    /// Platform target, if any (e.g. `tiktok`).
    pub platform: Option<String>,
    /// All findings, in scene-then-rule order.
    pub findings: Vec<LintFinding>,
}

impl LintReport {
    /// Count error-severity findings.
    pub fn error_count(&self) -> usize {
        self.findings
            .iter()
            .filter(|f| f.severity == Severity::Error)
            .count()
    }

    /// Count warning-severity findings.
    pub fn warn_count(&self) -> usize {
        self.findings
            .iter()
            .filter(|f| f.severity == Severity::Warn)
            .count()
    }

    /// Suggested exit code — 1 if any error finding, 0 otherwise.
    pub fn exit_code(&self) -> u8 {
        if self.error_count() > 0 {
            1
        } else {
            0
        }
    }
}
