//! `plan.complete` tool.

#![allow(missing_docs)]

use serde_json::{json, Value};
use crate::agent::plan::{Plan, Task, TaskId, TaskStatus};
use crate::agent::tools::{Tool, ToolResult};
use super::PlanToolCtx;
use crate::agent::plan::validator::ValidatorCtx;
use chrono::Utc;
use crate::agent::plan::validator::check_all;
use super::{parse_task_id};

pub struct PlanComplete(pub(super) PlanToolCtx);
impl Tool for PlanComplete {
    fn name(&self) -> &str { "plan.complete" }
    fn description(&self) -> &str {
        "Mark a task complete. Runs every declared validator first; on any \
         failure the task stays in Doing and the response carries structured \
         failure details for the model to react to."
    }
    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "required": ["task_id"],
            "properties": { "task_id": { "type": "string" } }
        })
    }
    fn dispatch(&self, args: &Value) -> ToolResult {
        let name = self.name();
        let id_str = match args.get("task_id").and_then(|v| v.as_str()) {
            Some(s) => s, None => return ToolResult::local_err(name, "missing `task_id`"),
        };
        let id = match parse_task_id(id_str, name) {
            Ok(id) => id, Err(e) => return e,
        };

        let outer = self.0.plan.lock().expect("plan cell poisoned");
        let inner_arc = match outer.as_ref() {
            Some(p) => p.clone(),
            None => return ToolResult::local_err(name, "plan_mode_off"),
        };
        drop(outer);

        let (task, workdir) = {
            let plan = inner_arc.lock().expect("inner plan poisoned");
            let Some(task) = plan.tasks.get(&id).cloned() else {
                return ToolResult::local_err(name, format!("unknown task_id {id}"));
            };
            if !matches!(task.status, TaskStatus::Doing | TaskStatus::Todo) {
                return ToolResult::local_err(
                    name,
                    format!("task {id} is in status {:?}; only Doing/Todo can be completed", task.status),
                );
            }
            (task, plan.workdir.clone())
        };

        let ctx = ValidatorCtx {
            workdir: &workdir,
            gamut_bin: &self.0.gamut_bin,
            session_cost_usd: 0.0,
        };
        let outcomes = check_all(&task, &self.0.validators, &ctx);

        let failed: Vec<Value> = outcomes
            .iter()
            .filter(|(_, o)| !o.ok)
            .map(|(crit, o)| {
                json!({
                    "kind": crit.kind,
                    "params": serde_json::to_value(&crit.params).unwrap_or(Value::Null),
                    "detail": o.detail,
                })
            })
            .collect();

        let mut plan = inner_arc.lock().expect("inner plan poisoned");
        let mut task = match plan.tasks.get(&id).cloned() {
            Some(t) => t,
            None => return ToolResult::local_err(name, format!("unknown task_id {id}")),
        };

        if failed.is_empty() {
            task.status = TaskStatus::Done;
            task.updated_at = Utc::now();
            if let Err(e) = plan.update(task, "\n") {
                return ToolResult::local_err(name, format!("write failed: {e}"));
            }
            ToolResult::local_ok(name, json!({ "ok": true }))
        } else {
            let body = format!(
                "\nvalidator_failure {}: {}\n",
                Utc::now().to_rfc3339(),
                serde_json::to_string(&failed).unwrap_or_else(|_| "[]".into()),
            );
            task.updated_at = Utc::now();
            if let Err(e) = plan.update(task, &body) {
                return ToolResult::local_err(name, format!("write failed: {e}"));
            }
            ToolResult::local_ok(name, json!({
                "ok": false,
                "failed_validators": failed,
            }))
        }
    }
}

// ─── plan.reopen ───────────────────────────────────────────────────

