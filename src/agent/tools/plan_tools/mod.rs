//! `plan.*` tools — agent-facing surface over the Plan (wb-mqsb.4).
//!
//! Each tool holds a `PlanCell` (cloned `Arc` of the session's plan
//! slot). When the inner `Option` is `None`, the session is in
//! plan-mode-off and all tools short-circuit with a structured error.
//!
//! `plan.complete` runs eager validation via the shared
//! `ValidatorRegistry`: every criterion is graded against the workdir
//! before the task flips to `Done`. Failures append a
//! `validator_failure` marker to the task body and leave the status
//! untouched so the model sees the structured detail and reacts.

use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::Arc;

use chrono::Utc;
use serde_json::{json, Value};

use super::{Tool, ToolRegistry, ToolResult};
use crate::agent::plan::schema::{Plan, Task, TaskId, TaskStatus};
use crate::agent::plan::seed::seed_from_pipeline;
use crate::agent::plan::validator::{check_all, ValidatorCtx, ValidatorRegistry};
use crate::agent::session::{empty_completion_flag, CompletionFlag, PlanCell};
use crate::pipelines::schema::StageSuccessCriterion;

/// Shared context every plan tool needs.
#[derive(Clone)]
pub(super) struct PlanToolCtx {
    pub(super) plan: PlanCell,
    validators: Arc<ValidatorRegistry>,
    gamut_bin: PathBuf,
    completion: CompletionFlag,
}

impl PlanToolCtx {
    fn with_plan<F, R>(&self, tool_name: &str, f: F) -> Result<R, ToolResult>
    where
        F: FnOnce(&mut Plan) -> Result<R, ToolResult>,
    {
        let outer = self.plan.lock().expect("plan cell poisoned");
        let inner_arc = match outer.as_ref() {
            Some(p) => p.clone(),
            None => {
                return Err(ToolResult::local_err(tool_name, "plan_mode_off"));
            }
        };
        drop(outer);
        let mut plan = inner_arc.lock().expect("inner plan poisoned");
        f(&mut plan)
    }
}

pub(super) fn parse_task_id(s: &str, tool_name: &str) -> Result<TaskId, ToolResult> {
    use std::str::FromStr;
    ulid::Ulid::from_str(s)
        .map(TaskId)
        .map_err(|e| ToolResult::local_err(tool_name, format!("bad task_id `{s}`: {e}")))
}

pub(super) fn task_to_json(task: &Task) -> Value {
    serde_json::to_value(task).unwrap_or(Value::Null)
}

/// Register every `plan.*` tool against a shared cell + validator
/// registry. The cell is also held by `Session::plan`.
pub fn register_with_plan(
    r: &mut ToolRegistry,
    plan: PlanCell,
    validators: Arc<ValidatorRegistry>,
) {
    register_with_plan_and_completion(r, plan, validators, empty_completion_flag());
}

/// Like `register_with_plan` but also shares a `CompletionFlag` with
/// `plan.done`. Use this when wiring up an `AgentLoop` so the
/// orchestrator can observe the flag.
pub fn register_with_plan_and_completion(
    r: &mut ToolRegistry,
    plan: PlanCell,
    validators: Arc<ValidatorRegistry>,
    completion: CompletionFlag,
) {
    let ctx = PlanToolCtx {
        plan,
        validators,
        gamut_bin: super::resolve_gamut_bin(),
        completion,
    };
    r.register(PlanAdd(ctx.clone()));
    r.register(PlanUpdate(ctx.clone()));
    r.register(PlanComplete(ctx.clone()));
    r.register(PlanReopen(ctx.clone()));
    r.register(PlanAbandon(ctx.clone()));
    r.register(PlanFork(ctx.clone()));
    r.register(PlanValidate(ctx.clone()));
    r.register(PlanShow(ctx.clone()));
    r.register(PlanSeed(ctx.clone()));
    r.register(PlanDone(ctx));
}

/// Resolve a known pipeline name to a `pipeline_defs/<name>.yaml` path.
/// Order: `$cwd/pipeline_defs/<name>.yaml` → `$WAVELET_PIPELINE_DEFS/<name>.yaml`
/// → `$CARGO_MANIFEST_DIR/pipeline_defs/<name>.yaml` (baked at build).
/// Returns `None` if nothing exists at any resolution point.
pub(super) fn resolve_pipeline_path(name: &str) -> Option<PathBuf> {
    let filename = format!("{name}.yaml");

    if let Ok(cwd) = std::env::current_dir() {
        let candidate = cwd.join("pipeline_defs").join(&filename);
        if candidate.is_file() {
            return Some(candidate);
        }
    }

    if let Ok(env_dir) = std::env::var("WAVELET_PIPELINE_DEFS") {
        let candidate = Path::new(&env_dir).join(&filename);
        if candidate.is_file() {
            return Some(candidate);
        }
    }

    let baked = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("pipeline_defs")
        .join(&filename);
    if baked.is_file() {
        return Some(baked);
    }

    None
}

/// Convenience wrapper used by `default_registry` — wires a fresh
/// empty plan cell (plan mode Off) + the built-in validator registry.
pub fn register(r: &mut ToolRegistry) {
    register_with_plan(
        r,
        crate::agent::session::empty_plan_cell(),
        Arc::new(ValidatorRegistry::with_builtins()),
    );
}

// ─── plan.add ──────────────────────────────────────────────────────


pub mod add;
pub use add::*;
pub mod update;
pub use update::*;
pub mod complete;
pub use complete::*;
pub mod reopen;
pub use reopen::*;
pub mod abandon;
pub use abandon::*;
pub mod fork;
pub use fork::*;
pub mod validate;
pub use validate::*;
pub mod show;
pub use show::*;
pub mod seed;
pub use seed::*;
pub mod done;
pub use done::*;
