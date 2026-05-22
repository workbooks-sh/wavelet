//! `plan.fork` tool.

#![allow(missing_docs)]

use serde_json::{json, Value};
use crate::agent::plan::{Plan, Task, TaskId, TaskStatus};
use crate::agent::tools::{Tool, ToolResult};
use super::PlanToolCtx;
use chrono::Utc;
use super::{parse_task_id};

pub struct PlanFork(pub(super) PlanToolCtx);
impl Tool for PlanFork {
    fn name(&self) -> &str { "plan.fork" }
    fn description(&self) -> &str {
        "Clone a task into a new Todo child. Inherits validators/deps; \
         original is untouched."
    }
    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "required": ["task_id"],
            "properties": {
                "task_id": { "type": "string" },
                "title": { "type": "string" }
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
        let new_title = args.get("title").and_then(|v| v.as_str()).map(String::from);

        let result = self.0.with_plan(name, |plan| {
            let original = match plan.tasks.get(&id).cloned() {
                Some(t) => t,
                None => return Err(ToolResult::local_err(name, format!("unknown task_id {id}"))),
            };
            let now = Utc::now();
            let child = Task {
                task: TaskId::new(),
                title: new_title.unwrap_or_else(|| format!("fork: {}", original.title)),
                status: TaskStatus::Todo,
                description: original.description.clone(),
                deps: original.deps.clone(),
                parent: Some(original.task),
                budget_usd: original.budget_usd,
                budget_wall_s: original.budget_wall_s,
                validators: original.validators.clone(),
                created_at: now,
                updated_at: now,
                cost_usd: 0.0,
                attempts: 0,
                seed_from: original.seed_from.clone(),
                extra: Default::default(),
            };
            let new_id = child.task;
            plan.insert(child, "\n").map_err(|e| {
                ToolResult::local_err(name, format!("insert failed: {e}"))
            })?;
            Ok(new_id)
        });
        match result {
            Ok(new_id) => ToolResult::local_ok(name, json!({ "new_task_id": new_id.to_string() })),
            Err(e) => e,
        }
    }
}

// ─── plan.validate ─────────────────────────────────────────────────

