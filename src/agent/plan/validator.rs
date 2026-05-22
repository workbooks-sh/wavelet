//! Validator trait + dispatcher (wb-mqsb.2).
//!
//! Each `StageSuccessCriterion` on a `Task` names a `kind` and carries
//! `params`. A `Validator` impl knows how to grade one kind; the
//! `ValidatorRegistry` dispatches by kind. Failure detail is always
//! machine-readable JSON so the model can act on it.
//!
//! Heavier validators (`query.*`, `render.*`, `unit_test_passes`,
//! `rubric_passes`) land in wb-mqsb.3.

use std::collections::HashMap;
use std::path::Path;
use std::process::Command;
use std::time::Instant;

use serde_json::json;

use crate::agent::plan::schema::Task;
use crate::pipelines::schema::StageSuccessCriterion;

/// Grades a single criterion. Implementations are stateless and `Send +
/// Sync` so the registry can be shared across threads.
pub trait Validator: Send + Sync {
    /// The `kind` string this validator handles. Must be unique within a
    /// registry.
    fn kind(&self) -> &'static str;

    /// Run the check. `params` is the raw YAML value from the task front
    /// matter; `ctx` carries shared run state (workdir, wavelet binary,
    /// running cost).
    fn check(&self, params: &serde_yaml::Value, ctx: &ValidatorCtx) -> ValidatorOutcome;
}

/// Shared context for a validator pass. The caller wires in concrete
/// paths and the running cost meter; validators consume read-only.
pub struct ValidatorCtx<'a> {
    /// Directory the task is being executed inside. Relative paths in
    /// validator params resolve against this.
    pub workdir: &'a Path,
    /// Path to the `wavelet` binary, for validators that shell out to
    /// sub-commands (wb-mqsb.3+).
    pub gamut_bin: &'a Path,
    /// Cumulative USD spent in this agent session — fed to `cost_under`.
    pub session_cost_usd: f32,
}

/// One graded criterion. `detail` is JSON so failures can be re-fed to
/// the model without ad-hoc string parsing.
#[derive(Debug, Clone)]
pub struct ValidatorOutcome {
    /// Did the criterion pass.
    pub ok: bool,
    /// Structured detail. Required even on success — useful for logging.
    pub detail: serde_json::Value,
    /// Marginal USD this check spent (vision / rubric validators are
    /// non-zero; local checks are zero).
    pub cost_usd: f32,
    /// Wall time the check took, in milliseconds.
    pub wall_ms: u128,
}

/// Errors raised at registry-management time. Check-time errors surface
/// as `ValidatorOutcome { ok: false, .. }`, not `Result`.
#[derive(Debug, thiserror::Error)]
pub enum ValidatorRegistryError {
    /// Two validators tried to claim the same `kind`. Configuration bug.
    #[error("duplicate validator kind: {0}")]
    DuplicateKind(&'static str),
}

/// Kind → validator dispatcher. Build once at startup, share by ref.
///
/// `register` returns a `Result` (not panic) because plugin-style
/// registration may grow runtime-fed kinds — a duplicate is a
/// configuration error worth recovering from, not an abort.
#[derive(Default)]
pub struct ValidatorRegistry {
    by_kind: HashMap<&'static str, Box<dyn Validator>>,
}

impl ValidatorRegistry {
    /// Empty registry.
    pub fn new() -> Self {
        Self {
            by_kind: HashMap::new(),
        }
    }

    /// Registry pre-loaded with the four lightweight built-ins
    /// (`artifact_exists`, `cost_under`, `cmd_zero_exit`, `assertion_eq`)
    /// plus the nine heavy validators (`query_scene_graph`,
    /// `query_pixels`, `query_snapshot`, `query_beat`, `query_shader`,
    /// `comp_verify_passes`, `c2pa_verify_passes`, `unit_test_passes`,
    /// `rubric_passes`).
    pub fn with_builtins() -> Self {
        let mut r = Self::new();
        r.register(Box::new(ArtifactExists)).unwrap();
        r.register(Box::new(CostUnder)).unwrap();
        r.register(Box::new(CmdZeroExit)).unwrap();
        r.register(Box::new(AssertionEq)).unwrap();
        super::validators::register_all(&mut r).unwrap();
        r
    }

    /// Add a validator. Rejects duplicate `kind` registrations.
    pub fn register(
        &mut self,
        v: Box<dyn Validator>,
    ) -> Result<(), ValidatorRegistryError> {
        let kind = v.kind();
        if self.by_kind.contains_key(kind) {
            return Err(ValidatorRegistryError::DuplicateKind(kind));
        }
        self.by_kind.insert(kind, v);
        Ok(())
    }

    /// Look up a validator by kind.
    pub fn get(&self, kind: &str) -> Option<&dyn Validator> {
        self.by_kind.get(kind).map(|b| b.as_ref())
    }
}

/// Grade every criterion on the task, in declared order. Caller folds
/// the outcomes into pass/fail.
pub fn check_all(
    task: &Task,
    registry: &ValidatorRegistry,
    ctx: &ValidatorCtx,
) -> Vec<(StageSuccessCriterion, ValidatorOutcome)> {
    task.validators
        .iter()
        .map(|crit| {
            let start = Instant::now();
            let outcome = match registry.get(&crit.kind) {
                Some(v) => v.check(&crit.params, ctx),
                None => ValidatorOutcome {
                    ok: false,
                    detail: json!({
                        "error": "unknown_kind",
                        "kind": crit.kind,
                    }),
                    cost_usd: 0.0,
                    wall_ms: start.elapsed().as_millis(),
                },
            };
            (crit.clone(), outcome)
        })
        .collect()
}

// ─── built-in impls ────────────────────────────────────────────────

/// `artifact_exists` — `params.path` resolves under `ctx.workdir` and
/// has size > 0.
pub struct ArtifactExists;

impl Validator for ArtifactExists {
    fn kind(&self) -> &'static str {
        "artifact_exists"
    }

    fn check(&self, params: &serde_yaml::Value, ctx: &ValidatorCtx) -> ValidatorOutcome {
        let start = Instant::now();
        let path_str = params.get("path").and_then(|v| v.as_str());
        let Some(rel) = path_str else {
            return ValidatorOutcome {
                ok: false,
                detail: json!({
                    "error": "missing_param",
                    "param": "path",
                }),
                cost_usd: 0.0,
                wall_ms: start.elapsed().as_millis(),
            };
        };

        let full = ctx.workdir.join(rel);
        let meta = std::fs::metadata(&full);
        match meta {
            Ok(m) if m.is_file() && m.len() > 0 => ValidatorOutcome {
                ok: true,
                detail: json!({
                    "path": rel,
                    "exists": true,
                    "size": m.len(),
                }),
                cost_usd: 0.0,
                wall_ms: start.elapsed().as_millis(),
            },
            Ok(m) if m.is_file() => ValidatorOutcome {
                ok: false,
                detail: json!({
                    "path": rel,
                    "exists": true,
                    "size": 0,
                    "reason": "empty_file",
                    "checked_in": ctx.workdir.display().to_string(),
                }),
                cost_usd: 0.0,
                wall_ms: start.elapsed().as_millis(),
            },
            Ok(_) => ValidatorOutcome {
                ok: false,
                detail: json!({
                    "path": rel,
                    "exists": true,
                    "reason": "not_a_file",
                    "checked_in": ctx.workdir.display().to_string(),
                }),
                cost_usd: 0.0,
                wall_ms: start.elapsed().as_millis(),
            },
            Err(_) => ValidatorOutcome {
                ok: false,
                detail: json!({
                    "path": rel,
                    "exists": false,
                    "checked_in": ctx.workdir.display().to_string(),
                }),
                cost_usd: 0.0,
                wall_ms: start.elapsed().as_millis(),
            },
        }
    }
}

/// `cost_under` — `ctx.session_cost_usd < params.max`.
pub struct CostUnder;

impl Validator for CostUnder {
    fn kind(&self) -> &'static str {
        "cost_under"
    }

    fn check(&self, params: &serde_yaml::Value, ctx: &ValidatorCtx) -> ValidatorOutcome {
        let start = Instant::now();
        let max = params.get("max").and_then(|v| v.as_f64());
        let Some(max) = max else {
            return ValidatorOutcome {
                ok: false,
                detail: json!({
                    "error": "missing_param",
                    "param": "max",
                }),
                cost_usd: 0.0,
                wall_ms: start.elapsed().as_millis(),
            };
        };

        let spent = ctx.session_cost_usd as f64;
        let ok = spent < max;
        let detail = if ok {
            json!({
                "session_cost_usd": spent,
                "max": max,
                "remaining_usd": max - spent,
            })
        } else {
            json!({
                "session_cost_usd": spent,
                "max": max,
                "remaining_usd": max - spent,
                "reason": "budget_exceeded",
            })
        };

        ValidatorOutcome {
            ok,
            detail,
            cost_usd: 0.0,
            wall_ms: start.elapsed().as_millis(),
        }
    }
}

/// `cmd_zero_exit` — spawn `params.cmd` (string array) in `ctx.workdir`
/// and require exit code 0.
pub struct CmdZeroExit;

impl Validator for CmdZeroExit {
    fn kind(&self) -> &'static str {
        "cmd_zero_exit"
    }

    fn check(&self, params: &serde_yaml::Value, ctx: &ValidatorCtx) -> ValidatorOutcome {
        let start = Instant::now();
        let cmd = params.get("cmd").and_then(|v| v.as_sequence());
        let Some(cmd) = cmd else {
            return ValidatorOutcome {
                ok: false,
                detail: json!({
                    "error": "missing_param",
                    "param": "cmd",
                    "expected": "array of strings",
                }),
                cost_usd: 0.0,
                wall_ms: start.elapsed().as_millis(),
            };
        };

        let argv: Vec<String> = cmd
            .iter()
            .filter_map(|v| v.as_str().map(|s| s.to_string()))
            .collect();

        if argv.is_empty() {
            return ValidatorOutcome {
                ok: false,
                detail: json!({
                    "error": "empty_cmd",
                }),
                cost_usd: 0.0,
                wall_ms: start.elapsed().as_millis(),
            };
        }

        let mut command = Command::new(&argv[0]);
        command.args(&argv[1..]).current_dir(ctx.workdir);

        match command.output() {
            Ok(out) => {
                let code = out.status.code();
                let ok = out.status.success();
                ValidatorOutcome {
                    ok,
                    detail: json!({
                        "cmd": argv,
                        "exit_code": code,
                        "success": ok,
                        "stdout_len": out.stdout.len(),
                        "stderr_len": out.stderr.len(),
                        "stderr_tail": String::from_utf8_lossy(&out.stderr)
                            .chars()
                            .rev()
                            .take(512)
                            .collect::<String>()
                            .chars()
                            .rev()
                            .collect::<String>(),
                    }),
                    cost_usd: 0.0,
                    wall_ms: start.elapsed().as_millis(),
                }
            }
            Err(e) => ValidatorOutcome {
                ok: false,
                detail: json!({
                    "cmd": argv,
                    "error": "spawn_failed",
                    "reason": e.to_string(),
                }),
                cost_usd: 0.0,
                wall_ms: start.elapsed().as_millis(),
            },
        }
    }
}

/// `assertion_eq` — compare `params.actual` to `params.expected` as
/// JSON-equivalent YAML values. Mostly a trait-shape harness.
pub struct AssertionEq;

impl Validator for AssertionEq {
    fn kind(&self) -> &'static str {
        "assertion_eq"
    }

    fn check(&self, params: &serde_yaml::Value, _ctx: &ValidatorCtx) -> ValidatorOutcome {
        let start = Instant::now();
        let actual = params.get("actual");
        let expected = params.get("expected");
        match (actual, expected) {
            (Some(a), Some(e)) => {
                let ok = a == e;
                let a_json = serde_json::to_value(a).unwrap_or(serde_json::Value::Null);
                let e_json = serde_json::to_value(e).unwrap_or(serde_json::Value::Null);
                ValidatorOutcome {
                    ok,
                    detail: json!({
                        "actual": a_json,
                        "expected": e_json,
                        "equal": ok,
                    }),
                    cost_usd: 0.0,
                    wall_ms: start.elapsed().as_millis(),
                }
            }
            _ => ValidatorOutcome {
                ok: false,
                detail: json!({
                    "error": "missing_param",
                    "param": if actual.is_none() { "actual" } else { "expected" },
                }),
                cost_usd: 0.0,
                wall_ms: start.elapsed().as_millis(),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::plan::schema::{TaskId, TaskStatus};
    use chrono::{DateTime, Utc};
    use std::collections::BTreeMap;
    use std::fs;
    use std::path::PathBuf;

    fn ctx<'a>(workdir: &'a Path, gamut_bin: &'a Path, cost: f32) -> ValidatorCtx<'a> {
        ValidatorCtx {
            workdir,
            gamut_bin,
            session_cost_usd: cost,
        }
    }

    fn yaml(s: &str) -> serde_yaml::Value {
        serde_yaml::from_str(s).unwrap()
    }

    fn make_task(criteria: Vec<StageSuccessCriterion>) -> Task {
        let now: DateTime<Utc> = DateTime::parse_from_rfc3339("2026-05-20T14:30:00Z")
            .unwrap()
            .with_timezone(&Utc);
        Task {
            task: TaskId::new(),
            title: "synthetic".into(),
            status: TaskStatus::Todo,
            description: None,
            deps: vec![],
            parent: None,
            budget_usd: None,
            budget_wall_s: None,
            validators: criteria,
            created_at: now,
            updated_at: now,
            cost_usd: 0.0,
            attempts: 0,
            seed_from: None,
            extra: BTreeMap::new(),
        }
    }

    #[test]
    fn artifact_exists_ok_and_missing() {
        let dir = tempfile::tempdir().unwrap();
        let workdir = dir.path();
        fs::write(workdir.join("brief.md"), b"hello\n").unwrap();
        let bin = PathBuf::from("wavelet");
        let ctx = ctx(workdir, &bin, 0.0);

        let v = ArtifactExists;
        let ok = v.check(&yaml("{ path: brief.md }"), &ctx);
        assert!(ok.ok);
        assert_eq!(ok.detail["exists"], serde_json::Value::Bool(true));
        assert_eq!(ok.detail["path"], serde_json::Value::String("brief.md".into()));

        let miss = v.check(&yaml("{ path: nope.md }"), &ctx);
        assert!(!miss.ok);
        assert_eq!(miss.detail["exists"], serde_json::Value::Bool(false));
        assert_eq!(miss.detail["path"], serde_json::Value::String("nope.md".into()));
        assert!(miss.detail.get("checked_in").is_some());

        fs::write(workdir.join("empty.md"), b"").unwrap();
        let empty = v.check(&yaml("{ path: empty.md }"), &ctx);
        assert!(!empty.ok);
        assert_eq!(empty.detail["reason"], serde_json::Value::String("empty_file".into()));
    }

    #[test]
    fn cost_under_ok_and_exceeded() {
        let dir = tempfile::tempdir().unwrap();
        let bin = PathBuf::from("wavelet");

        let v = CostUnder;
        let under = v.check(&yaml("{ max: 5.0 }"), &ctx(dir.path(), &bin, 1.25));
        assert!(under.ok);
        assert_eq!(under.detail["max"], serde_json::Value::from(5.0));
        let remaining = under.detail["remaining_usd"].as_f64().unwrap();
        assert!((remaining - 3.75).abs() < 1e-6);

        let over = v.check(&yaml("{ max: 1.0 }"), &ctx(dir.path(), &bin, 2.5));
        assert!(!over.ok);
        assert_eq!(over.detail["reason"], serde_json::Value::String("budget_exceeded".into()));
        let rem = over.detail["remaining_usd"].as_f64().unwrap();
        assert!(rem < 0.0);
    }

    #[cfg(unix)]
    #[test]
    fn cmd_zero_exit_ok_nonzero_and_spawn_failure() {
        let dir = tempfile::tempdir().unwrap();
        let bin = PathBuf::from("wavelet");
        let ctx = ctx(dir.path(), &bin, 0.0);
        let v = CmdZeroExit;

        let ok = v.check(&yaml("{ cmd: [sh, -c, 'exit 0'] }"), &ctx);
        assert!(ok.ok, "expected zero exit to pass, detail={:?}", ok.detail);
        assert_eq!(ok.detail["exit_code"], serde_json::Value::from(0));

        let bad = v.check(&yaml("{ cmd: [sh, -c, 'exit 7'] }"), &ctx);
        assert!(!bad.ok);
        assert_eq!(bad.detail["exit_code"], serde_json::Value::from(7));

        let missing = v.check(
            &yaml("{ cmd: ['/definitely/not/a/binary/zzqq'] }"),
            &ctx,
        );
        assert!(!missing.ok);
        assert_eq!(
            missing.detail["error"],
            serde_json::Value::String("spawn_failed".into())
        );
        assert!(missing.detail.get("reason").is_some());
    }

    #[test]
    fn registry_rejects_duplicate_kind() {
        let mut r = ValidatorRegistry::new();
        r.register(Box::new(ArtifactExists)).unwrap();
        let err = r.register(Box::new(ArtifactExists)).unwrap_err();
        match err {
            ValidatorRegistryError::DuplicateKind(k) => assert_eq!(k, "artifact_exists"),
        }
    }

    #[test]
    fn check_all_returns_outcomes_in_declared_order() {
        let dir = tempfile::tempdir().unwrap();
        let workdir = dir.path();
        fs::write(workdir.join("brief.md"), b"hi\n").unwrap();
        let bin = PathBuf::from("wavelet");

        let registry = ValidatorRegistry::with_builtins();

        let task = make_task(vec![
            StageSuccessCriterion {
                kind: "artifact_exists".into(),
                params: yaml("{ path: brief.md }"),
            },
            StageSuccessCriterion {
                kind: "cost_under".into(),
                params: yaml("{ max: 10.0 }"),
            },
        ]);

        let outcomes = check_all(&task, &registry, &ctx(workdir, &bin, 1.0));
        assert_eq!(outcomes.len(), 2);
        assert_eq!(outcomes[0].0.kind, "artifact_exists");
        assert_eq!(outcomes[1].0.kind, "cost_under");
        assert!(outcomes.iter().all(|(_, o)| o.ok));
    }

    #[test]
    fn assertion_eq_matches_and_diffs() {
        let dir = tempfile::tempdir().unwrap();
        let bin = PathBuf::from("wavelet");
        let ctx = ctx(dir.path(), &bin, 0.0);
        let v = AssertionEq;

        let same = v.check(&yaml("{ actual: 42, expected: 42 }"), &ctx);
        assert!(same.ok);
        assert_eq!(same.detail["equal"], serde_json::Value::Bool(true));

        let diff = v.check(&yaml("{ actual: 42, expected: 7 }"), &ctx);
        assert!(!diff.ok);
        assert_eq!(diff.detail["equal"], serde_json::Value::Bool(false));
    }

    #[test]
    fn check_all_unknown_kind_surfaces_structured_error() {
        let dir = tempfile::tempdir().unwrap();
        let bin = PathBuf::from("wavelet");
        let registry = ValidatorRegistry::with_builtins();

        let task = make_task(vec![StageSuccessCriterion {
            kind: "totally_made_up".into(),
            params: serde_yaml::Value::Null,
        }]);

        let outcomes = check_all(&task, &registry, &ctx(dir.path(), &bin, 0.0));
        assert_eq!(outcomes.len(), 1);
        assert!(!outcomes[0].1.ok);
        assert_eq!(
            outcomes[0].1.detail["error"],
            serde_json::Value::String("unknown_kind".into())
        );
    }
}
