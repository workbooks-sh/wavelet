//! `wavelet shot edit` — model-as-planner / agent-as-loop video edit verb.
//!
//! Architecture:
//! - **Plan** (`plan.rs`): Gemini decomposes the user's intent into a
//!   typed JSON plan with an approach + a list of typed steps.
//! - **Execute** (`execute.rs`): dispatch each step to the matching
//!   tool in `tools/` (CSS-only, Veo regen, Omni edit, Composite).
//! - **Review** (`review.rs`): a different Gemini model watches the
//!   output and grades against the original intent.
//! - **Decide** (`loop_.rs`): if score ≥ threshold, ship. Otherwise
//!   feed the reviewer's critique back into the next plan. Stop on
//!   max attempts or budget exhaustion; ship the best-of-N otherwise.

pub mod execute;
pub mod gemini;
pub mod intent;
#[path = "loop_.rs"]
pub mod loop_;
pub mod plan;
pub mod report;
pub mod review;
pub mod tools;

use std::path::PathBuf;

use thiserror::Error;

pub use intent::{EditConfig, EditRequest, InputKind};
pub use plan::{Approach, Plan, Step};
pub use report::{AttemptOutcome, EditResult};
pub use review::Verdict;

/// All errors the edit loop can surface.
#[derive(Debug, Error)]
pub enum EditError {
    /// `GOOGLE_API_KEY` is missing or blank.
    #[error("GOOGLE_API_KEY not set (or blank). Edit requires Gemini access.")]
    NoKey,
    /// Planner returned something we couldn't parse into a `Plan`.
    #[error("plan parse failure: {0}")]
    PlanParse(String),
    /// Reviewer returned something we couldn't parse into a `Verdict`.
    #[error("review parse failure: {0}")]
    ReviewParse(String),
    /// HTTP / filesystem / render failure.
    #[error("{0}")]
    Transport(String),
    /// Gemini Omni model isn't shipped — caller should pick a
    /// different approach.
    #[error("OmniEdit unavailable for input `{input}` with instruction `{instruction}`: {detail}")]
    OmniUnavailable {
        /// Input path the planner targeted.
        input: String,
        /// The instruction the planner wanted to route to Omni.
        instruction: String,
        /// Operator-facing detail (probed model slug, etc.).
        detail: String,
    },
}

/// High-level entry point — wire the production planner / executor /
/// reviewer together and run the loop.
pub fn run_edit(req: EditRequest) -> EditResult {
    let api_key = match gemini::api_key_from_env() {
        Ok(k) => k,
        Err(e) => {
            return EditResult {
                input: req.input.clone(),
                intent: req.intent.clone(),
                shipped: None,
                shipped_score: None,
                shipped_attempt: None,
                attempts: vec![],
                total_cost_usd: 0.0,
                total_wall_ms: 0,
                note: Some(format!("{e}")),
            };
        }
    };

    // Cache the reviewer's uploaded video URI per attempt so we don't
    // re-upload — but capture is per-call below; the closure owns its
    // state.
    let api_key_for_plan = api_key.clone();
    let planner_model = req.cfg.planner_model.clone();
    let api_key_for_review = api_key.clone();
    let reviewer_model = req.cfg.reviewer_model.clone();

    let hooks = loop_::LoopHooks {
        plan: Box::new(move |req, prior| {
            let prompt = plan::build_planner_prompt(req, prior);
            let raw = gemini::generate_text(&planner_model, &prompt, &api_key_for_plan)?;
            plan::parse_plan(&raw)
        }),
        execute: Box::new(move |req, plan, out, n| execute::execute_plan(req, plan, out, n)),
        review: Box::new(move |req, plan, mp4| {
            let prompt = review::build_review_prompt(
                &req.intent,
                &format!(
                    "intent_summary: {}\nreasoning: {}\nsteps:\n{}",
                    plan.intent_summary,
                    plan.reasoning,
                    plan.steps
                        .iter()
                        .map(|s| format!("- {s:?}"))
                        .collect::<Vec<_>>()
                        .join("\n"),
                ),
                req.cfg.pass_threshold,
            );
            let file_uri = gemini::upload_file(mp4, "video/mp4", &api_key_for_review)?;
            let raw = gemini::generate_with_video(
                &reviewer_model,
                &prompt,
                &file_uri,
                "video/mp4",
                &api_key_for_review,
            )?;
            review::parse_verdict(&raw)
        }),
    };

    loop_::run_loop(&req, hooks)
}

/// Convenience writer — serializes the result and writes it to the
/// caller's chosen report path.
pub fn write_report(result: &EditResult, path: &PathBuf) -> Result<(), EditError> {
    let json = serde_json::to_string_pretty(result)
        .map_err(|e| EditError::Transport(format!("serialize report: {e}")))?;
    std::fs::write(path, json)
        .map_err(|e| EditError::Transport(format!("write {}: {e}", path.display())))?;
    Ok(())
}

#[cfg(test)]
mod integration_tests {
    use super::*;
    use std::path::PathBuf;

    /// End-to-end smoke against the dry-run coffee fixture. Requires a
    /// real `GOOGLE_API_KEY` because both planner + reviewer hit
    /// Gemini. Gated behind `#[ignore]` so unit-test runs stay
    /// hermetic. Run manually with:
    ///
    /// ```text
    /// GOOGLE_API_KEY=... cargo test -p wavelet --lib edit::integration_tests -- --ignored --nocapture
    /// ```
    #[test]
    #[ignore]
    fn coffee_dusk_end_to_end() {
        let scene_html = PathBuf::from(
            "evals/runs/wavelet/001-mini-coffee-codex-dryrun-2026-05-19T23-15-25-292Z/workdir/scenes/coffee.html",
        );
        if !scene_html.exists() {
            eprintln!("smoke skip: fixture not present at {}", scene_html.display());
            return;
        }
        let tmp = tempfile::tempdir().unwrap();
        let req = EditRequest {
            input: scene_html.clone(),
            kind: InputKind::SceneHtml,
            intent: "crop to a tighter square frame and dim the highlights".into(),
            cfg: EditConfig {
                max_attempts: 2,
                max_cost_usd: 0.20,
                pass_threshold: 0.65,
                planner_model: "gemini-3.1-pro-preview".into(),
                reviewer_model: "gemini-3.5-flash".into(),
                out_path: tmp.path().join("coffee-edited.mp4"),
                report_path: tmp.path().join("coffee-edit-report.json"),
                dry_run: false,
            },
        };
        let result = run_edit(req);
        eprintln!(
            "smoke result: shipped={:?} attempts={} note={:?}",
            result.shipped,
            result.attempts.len(),
            result.note
        );
        assert!(!result.attempts.is_empty(), "no attempts ran");
    }
}
