//! `plan.reopen` tool.

#![allow(missing_docs)]

use serde_json::{json, Value};
use crate::agent::plan::{Plan, Task, TaskId, TaskStatus};
use crate::agent::tools::{Tool, ToolResult};
use super::PlanToolCtx;
use chrono::Utc;
use super::{parse_task_id, task_to_json};

pub struct PlanReopen(pub(super) PlanToolCtx);
impl Tool for PlanReopen {
    fn name(&self) -> &str { "plan.reopen" }
    fn description(&self) -> &str {
        "Flip a Done/Abandoned/Blocked task back to Todo."
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
        let result = self.0.with_plan(name, |plan| {
            let mut task = match plan.tasks.get(&id).cloned() {
                Some(t) => t,
                None => return Err(ToolResult::local_err(name, format!("unknown task_id {id}"))),
            };
            task.status = TaskStatus::Todo;
            task.updated_at = Utc::now();
            let task_json = task_to_json(&task);
            plan.update(task, "\n").map_err(|e| {
                ToolResult::local_err(name, format!("update failed: {e}"))
            })?;
            Ok(task_json)
        });
        match result {
            Ok(task_json) => ToolResult::local_ok(name, json!({ "task": task_json })),
            Err(e) => e,
        }
    }
}

// ─── plan.abandon ──────────────────────────────────────────────────

