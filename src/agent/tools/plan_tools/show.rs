//! `plan.show` tool.

#![allow(missing_docs)]

use serde_json::{json, Value};
use crate::agent::plan::{Plan, Task, TaskId, TaskStatus};
use crate::agent::tools::{Tool, ToolResult};
use super::PlanToolCtx;
use super::{task_to_json};

pub struct PlanShow(pub(super) PlanToolCtx);
impl Tool for PlanShow {
    fn name(&self) -> &str { "plan.show" }
    fn description(&self) -> &str {
        "List tasks in the plan, optionally filtered by status or \
         validator kind."
    }
    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "filter": {
                    "type": "object",
                    "properties": {
                        "status": {
                            "type": "string",
                            "enum": ["todo", "doing", "done", "blocked", "abandoned"]
                        },
                        "has_validator_kind": { "type": "string" }
                    }
                }
            }
        })
    }
    fn dispatch(&self, args: &Value) -> ToolResult {
        let name = self.name();
        let filter_status = args
            .get("filter")
            .and_then(|f| f.get("status"))
            .and_then(|v| v.as_str())
            .map(String::from);
        let filter_kind = args
            .get("filter")
            .and_then(|f| f.get("has_validator_kind"))
            .and_then(|v| v.as_str())
            .map(String::from);

        let status_match = |t: &Task| -> bool {
            let Some(want) = &filter_status else { return true };
            let want = want.to_lowercase();
            let got = match t.status {
                TaskStatus::Todo => "todo",
                TaskStatus::Doing => "doing",
                TaskStatus::Done => "done",
                TaskStatus::Blocked => "blocked",
                TaskStatus::Abandoned => "abandoned",
            };
            got == want
        };
        let kind_match = |t: &Task| -> bool {
            let Some(k) = &filter_kind else { return true };
            t.validators.iter().any(|c| &c.kind == k)
        };

        let result = self.0.with_plan(name, |plan| {
            let mut tasks: Vec<Task> = plan
                .tasks
                .values()
                .filter(|t| status_match(t) && kind_match(t))
                .cloned()
                .collect();
            tasks.sort_by_key(|t| t.task);
            let json_tasks: Vec<Value> = tasks.iter().map(task_to_json).collect();
            Ok(json!({ "tasks": json_tasks }))
        });
        match result {
            Ok(v) => ToolResult::local_ok(name, v),
            Err(e) => e,
        }
    }
}

// ─── plan.seed ─────────────────────────────────────────────────────

