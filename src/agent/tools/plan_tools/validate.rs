//! `plan.validate` tool.

#![allow(missing_docs)]

use serde_json::{json, Value};
use crate::agent::plan::{Plan, Task, TaskId, TaskStatus};
use crate::agent::tools::{Tool, ToolResult};
use super::PlanToolCtx;
use crate::agent::plan::validator::ValidatorCtx;
use crate::agent::plan::validator::check_all;
use super::{parse_task_id};

pub struct PlanValidate(pub(super) PlanToolCtx);
impl Tool for PlanValidate {
    fn name(&self) -> &str { "plan.validate" }
    fn description(&self) -> &str {
        "Run validators over one task or every Doing/Done task. Does not \
         mutate status — pure inspection."
    }
    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": { "task_id": { "type": "string" } }
        })
    }
    fn dispatch(&self, args: &Value) -> ToolResult {
        let name = self.name();
        let id_filter = match args.get("task_id").and_then(|v| v.as_str()) {
            Some(s) => match parse_task_id(s, name) {
                Ok(id) => Some(id),
                Err(e) => return e,
            },
            None => None,
        };

        let outer = self.0.plan.lock().expect("plan cell poisoned");
        let inner_arc = match outer.as_ref() {
            Some(p) => p.clone(),
            None => return ToolResult::local_err(name, "plan_mode_off"),
        };
        drop(outer);

        let (targets, workdir) = {
            let plan = inner_arc.lock().expect("inner plan poisoned");
            let mut targets: Vec<Task> = match id_filter {
                Some(id) => match plan.tasks.get(&id).cloned() {
                    Some(t) => vec![t],
                    None => return ToolResult::local_err(name, format!("unknown task_id {id}")),
                },
                None => plan
                    .tasks
                    .values()
                    .filter(|t| matches!(t.status, TaskStatus::Doing | TaskStatus::Done))
                    .cloned()
                    .collect(),
            };
            targets.sort_by_key(|t| t.task);
            (targets, plan.workdir.clone())
        };

        let ctx = ValidatorCtx {
            workdir: &workdir,
            gamut_bin: &self.0.gamut_bin,
            session_cost_usd: 0.0,
        };

        let results: Vec<Value> = targets
            .iter()
            .map(|task| {
                let outcomes = check_all(task, &self.0.validators, &ctx);
                let ok = outcomes.iter().all(|(_, o)| o.ok);
                let details: Vec<Value> = outcomes
                    .iter()
                    .map(|(crit, o)| json!({
                        "kind": crit.kind,
                        "ok": o.ok,
                        "detail": o.detail,
                    }))
                    .collect();
                json!({
                    "task_id": task.task.to_string(),
                    "ok": ok,
                    "details": details,
                })
            })
            .collect();

        ToolResult::local_ok(name, json!({ "results": results }))
    }
}

// ─── plan.show ─────────────────────────────────────────────────────

