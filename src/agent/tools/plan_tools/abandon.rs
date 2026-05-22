//! `plan.abandon` tool.

#![allow(missing_docs)]

use serde_json::{json, Value};
use crate::agent::plan::{Plan, Task, TaskId, TaskStatus};
use crate::agent::tools::{Tool, ToolResult};
use super::PlanToolCtx;
use chrono::Utc;
use super::{parse_task_id, task_to_json};

pub struct PlanAbandon(pub(super) PlanToolCtx);
impl Tool for PlanAbandon {
    fn name(&self) -> &str { "plan.abandon" }
    fn description(&self) -> &str {
        "Mark a task as Abandoned (terminal but unsuccessful)."
    }
    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "required": ["task_id"],
            "properties": {
                "task_id": { "type": "string" },
                "reason": { "type": "string" }
            }
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
        let reason = args.get("reason").and_then(|v| v.as_str()).map(String::from);
        let result = self.0.with_plan(name, |plan| {
            let mut task = match plan.tasks.get(&id).cloned() {
                Some(t) => t,
                None => return Err(ToolResult::local_err(name, format!("unknown task_id {id}"))),
            };
            task.status = TaskStatus::Abandoned;
            task.updated_at = Utc::now();
            let task_json = task_to_json(&task);
            let body = match reason {
                Some(r) => format!("\nabandoned {}: {}\n", Utc::now().to_rfc3339(), r),
                None => "\n".to_string(),
            };
            plan.update(task, &body).map_err(|e| {
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

// ─── plan.fork ─────────────────────────────────────────────────────

