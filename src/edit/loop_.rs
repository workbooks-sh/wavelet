//! Bounded plan → execute → review loop.
//!
//! Stays trait-shaped: each stage is a closure so tests can stub
//! planner / executor / reviewer without hitting the network.

use std::path::Path;
use std::time::Instant;

use super::execute::ExecOutput;
use super::intent::EditRequest;
use super::plan::Plan;
use super::report::{AttemptOutcome, EditResult};
use super::review::Verdict;
use super::EditError;

/// Stage callbacks. Production builds wire these to the real Gemini
/// + execute paths; tests provide deterministic stubs.
pub struct LoopHooks<'a> {
    /// Build a plan for the given (request, prior_critique).
    pub plan: Box<dyn FnMut(&EditRequest, Option<&str>) -> Result<Plan, EditError> + 'a>,
    /// Execute a plan and return the rendered MP4 path.
    pub execute: Box<dyn FnMut(&EditRequest, &Plan, &Path, u32) -> Result<ExecOutput, EditError> + 'a>,
    /// Review the produced MP4 against the original intent.
    pub review: Box<dyn FnMut(&EditRequest, &Plan, &Path) -> Result<Verdict, EditError> + 'a>,
}

/// Run the loop end-to-end.
pub fn run_loop(req: &EditRequest, mut hooks: LoopHooks<'_>) -> EditResult {
    let started = Instant::now();
    let mut attempts: Vec<AttemptOutcome> = Vec::new();
    let mut total_cost: f32 = 0.0;
    let mut prior_critique: Option<String> = None;
    let mut shipped: Option<(u32, std::path::PathBuf, f32)> = None;
    let mut note: Option<String> = None;

    for n in 1..=req.cfg.max_attempts {
        // Plan
        let plan_result = (hooks.plan)(req, prior_critique.as_deref());
        let plan = match plan_result {
            Ok(p) => p,
            Err(e) => {
                attempts.push(AttemptOutcome {
                    n,
                    plan: empty_plan_for_error(),
                    review: None,
                    output_path: None,
                    cost_estimate_usd: 0.0,
                    error: Some(format!("plan: {e}")),
                });
                continue;
            }
        };

        // Budget check before executing.
        if total_cost + plan.estimated_cost_usd > req.cfg.max_cost_usd {
            attempts.push(AttemptOutcome {
                n,
                plan,
                review: None,
                output_path: None,
                cost_estimate_usd: 0.0,
                error: Some(format!(
                    "skipped: estimated ${:.2} + spent ${:.2} > max ${:.2}",
                    attempts
                        .last()
                        .map(|a| a.cost_estimate_usd)
                        .unwrap_or(0.0),
                    total_cost,
                    req.cfg.max_cost_usd
                )),
            });
            note = Some(format!(
                "exhausted budget after {n} attempts (max ${:.2})",
                req.cfg.max_cost_usd
            ));
            break;
        }

        // Dry-run short-circuit: emit plan and exit.
        if req.cfg.dry_run {
            attempts.push(AttemptOutcome {
                n,
                plan,
                review: None,
                output_path: None,
                cost_estimate_usd: 0.0,
                error: Some("dry-run".into()),
            });
            note = Some("dry-run — plan only".into());
            break;
        }

        let cost = plan.estimated_cost_usd;
        let attempt_mp4 = req
            .cfg
            .out_path
            .with_extension(format!("attempt-{n}.mp4"));

        // Execute
        let exec_result = (hooks.execute)(req, &plan, &attempt_mp4, n);
        let exec = match exec_result {
            Ok(o) => o,
            Err(e) => {
                attempts.push(AttemptOutcome {
                    n,
                    plan,
                    review: None,
                    output_path: None,
                    cost_estimate_usd: cost,
                    error: Some(format!("execute: {e}")),
                });
                total_cost += cost;
                prior_critique = Some(format!("execution failed: {e}"));
                continue;
            }
        };

        // Review
        let verdict_result = (hooks.review)(req, &plan, &exec.output_path);
        let verdict = match verdict_result {
            Ok(v) => v,
            Err(e) => {
                attempts.push(AttemptOutcome {
                    n,
                    plan,
                    review: None,
                    output_path: Some(exec.output_path.clone()),
                    cost_estimate_usd: cost,
                    error: Some(format!("review: {e}")),
                });
                total_cost += cost;
                continue;
            }
        };

        total_cost += cost;
        let ships = verdict.ships_at(req.cfg.pass_threshold);
        let score = verdict.score_or_zero();
        attempts.push(AttemptOutcome {
            n,
            plan,
            review: Some(verdict.clone()),
            output_path: Some(exec.output_path.clone()),
            cost_estimate_usd: cost,
            error: None,
        });

        if ships {
            shipped = Some((n, exec.output_path, score));
            break;
        }

        // Feed reviewer feedback into the next plan.
        prior_critique = Some(build_critique(&verdict));
    }

    // If no attempt shipped, fall back to best-of-N by score.
    if shipped.is_none() && !attempts.is_empty() {
        let best = attempts
            .iter()
            .filter(|a| a.output_path.is_some() && a.review.is_some())
            .max_by(|a, b| {
                a.review
                    .as_ref()
                    .map(|r| r.score_or_zero())
                    .unwrap_or(0.0)
                    .partial_cmp(
                        &b.review.as_ref().map(|r| r.score_or_zero()).unwrap_or(0.0),
                    )
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
        if let Some(b) = best {
            let score = b.review.as_ref().map(|r| r.score_or_zero()).unwrap_or(0.0);
            shipped = Some((b.n, b.output_path.clone().unwrap(), score));
            if note.is_none() {
                note = Some(format!(
                    "exhausted attempts after {} tries; shipped best-of-N (score = {:.2})",
                    attempts.len(),
                    score
                ));
            }
        }
    }

    // Promote attempt-N.mp4 → final out_path.
    let mut shipped_path = None;
    let mut shipped_score = None;
    let mut shipped_attempt = None;
    if let Some((n, p, s)) = shipped {
        if let Err(e) = std::fs::copy(&p, &req.cfg.out_path) {
            note = Some(format!(
                "{}; copy to {} failed: {e}",
                note.unwrap_or_default(),
                req.cfg.out_path.display()
            ));
        } else {
            shipped_path = Some(req.cfg.out_path.clone());
            shipped_score = Some(s);
            shipped_attempt = Some(n);
        }
    }

    EditResult {
        input: req.input.clone(),
        intent: req.intent.clone(),
        shipped: shipped_path,
        shipped_score,
        shipped_attempt,
        attempts,
        total_cost_usd: total_cost,
        total_wall_ms: started.elapsed().as_millis(),
        note,
    }
}

fn build_critique(v: &Verdict) -> String {
    let mut s = String::new();
    if !v.reasoning.is_empty() {
        s.push_str(&format!("reasoning: {}\n", v.reasoning));
    }
    if !v.competing_view.is_empty() {
        s.push_str(&format!("competing_view: {}\n", v.competing_view));
    }
    if !v.bias_audit.is_empty() {
        s.push_str(&format!("bias_audit: {}\n", v.bias_audit));
    }
    s
}

fn empty_plan_for_error() -> Plan {
    Plan {
        intent_summary: String::new(),
        approach: crate::edit::plan::Approach::CssOnly,
        estimated_cost_usd: 0.0,
        estimated_seconds: 0,
        steps: Vec::new(),
        reasoning: "(no plan — planner errored)".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::edit::intent::{EditConfig, EditRequest, InputKind};
    use crate::edit::plan::{Approach, Plan};
    use std::cell::RefCell;
    use std::path::PathBuf;

    fn req(tmp: &std::path::Path) -> EditRequest {
        EditRequest {
            input: tmp.join("input.html"),
            kind: InputKind::SceneHtml,
            intent: "make it dusk".into(),
            cfg: EditConfig {
                max_attempts: 3,
                max_cost_usd: 1.00,
                pass_threshold: 0.7,
                planner_model: "stub-planner".into(),
                reviewer_model: "stub-reviewer".into(),
                out_path: tmp.join("out.mp4"),
                report_path: tmp.join("report.json"),
                dry_run: false,
            },
        }
    }

    fn stub_plan() -> Plan {
        Plan {
            intent_summary: "stubbed".into(),
            approach: Approach::CssOnly,
            estimated_cost_usd: 0.01,
            estimated_seconds: 5,
            steps: vec![],
            reasoning: "stub".into(),
        }
    }

    #[test]
    fn ships_on_first_pass() {
        let tmp = tempfile::tempdir().unwrap();
        let r = req(tmp.path());
        let exec_calls = RefCell::new(0u32);
        let hooks = LoopHooks {
            plan: Box::new(|_, _| Ok(stub_plan())),
            execute: Box::new(|_req, _plan, out, _n| {
                *exec_calls.borrow_mut() += 1;
                std::fs::write(out, b"fake mp4").unwrap();
                Ok(ExecOutput {
                    output_path: out.to_path_buf(),
                    plan_summary: String::new(),
                })
            }),
            review: Box::new(|_, _, _| {
                Ok(Verdict {
                    pass: Some(true),
                    score: Some(0.9),
                    reasoning: "great".into(),
                    competing_view: String::new(),
                    bias_audit: String::new(),
                })
            }),
        };
        let result = run_loop(&r, hooks);
        assert_eq!(result.attempts.len(), 1);
        assert_eq!(result.shipped_attempt, Some(1));
        assert_eq!(*exec_calls.borrow(), 1);
        assert!(result.shipped.is_some());
        assert!(result.note.is_none());
    }

    #[test]
    fn best_of_n_ships_when_no_pass() {
        let tmp = tempfile::tempdir().unwrap();
        let r = req(tmp.path());
        let scores: RefCell<Vec<f32>> = RefCell::new(vec![0.3, 0.6, 0.5]);
        let hooks = LoopHooks {
            plan: Box::new(|_, _| Ok(stub_plan())),
            execute: Box::new(|_req, _plan, out, _n| {
                std::fs::write(out, b"fake mp4").unwrap();
                Ok(ExecOutput {
                    output_path: out.to_path_buf(),
                    plan_summary: String::new(),
                })
            }),
            review: Box::new(|_, _, _| {
                let s = scores.borrow_mut().remove(0);
                Ok(Verdict {
                    pass: Some(false),
                    score: Some(s),
                    reasoning: "below threshold".into(),
                    competing_view: String::new(),
                    bias_audit: String::new(),
                })
            }),
        };
        let result = run_loop(&r, hooks);
        assert_eq!(result.attempts.len(), 3);
        assert_eq!(result.shipped_attempt, Some(2)); // 0.6 wins
        assert!(result.shipped_score.unwrap() > 0.55);
        assert!(result.note.as_ref().unwrap().contains("best-of-N"));
    }

    #[test]
    fn budget_exhaustion_stops_loop() {
        let tmp = tempfile::tempdir().unwrap();
        let mut r = req(tmp.path());
        r.cfg.max_cost_usd = 0.005; // less than the per-plan estimate
        let hooks = LoopHooks {
            plan: Box::new(|_, _| Ok(stub_plan())), // costs 0.01 each
            execute: Box::new(|_, _, _, _| {
                panic!("should not execute when budget is exhausted")
            }),
            review: Box::new(|_, _, _| panic!("should not review")),
        };
        let result = run_loop(&r, hooks);
        assert_eq!(result.attempts.len(), 1);
        assert!(result.note.as_ref().unwrap().contains("exhausted budget"));
        assert!(result.shipped.is_none());
    }

    #[test]
    fn dry_run_emits_plan_only() {
        let tmp = tempfile::tempdir().unwrap();
        let mut r = req(tmp.path());
        r.cfg.dry_run = true;
        let exec_calls = RefCell::new(0u32);
        let hooks = LoopHooks {
            plan: Box::new(|_, _| Ok(stub_plan())),
            execute: Box::new(|_, _, _, _| {
                *exec_calls.borrow_mut() += 1;
                Ok(ExecOutput {
                    output_path: PathBuf::new(),
                    plan_summary: String::new(),
                })
            }),
            review: Box::new(|_, _, _| panic!("review not expected on dry-run")),
        };
        let result = run_loop(&r, hooks);
        assert_eq!(*exec_calls.borrow(), 0);
        assert_eq!(result.attempts.len(), 1);
        assert!(result.note.as_ref().unwrap().contains("dry-run"));
    }
}
