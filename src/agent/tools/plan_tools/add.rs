//! `plan.add` tool.

#![allow(missing_docs)]

use serde_json::{json, Value};
use crate::agent::plan::{Plan, Task, TaskId, TaskStatus};
use crate::agent::tools::{Tool, ToolResult};
use super::PlanToolCtx;
use crate::pipelines::schema::StageSuccessCriterion;
use chrono::Utc;
use super::{parse_task_id};

pub struct PlanAdd(pub(super) PlanToolCtx);
impl Tool for PlanAdd {
    fn name(&self) -> &str { "plan.add" }
    fn description(&self) -> &str {
        "Add a new task to the plan. Returns the freshly minted task_id."
    }
    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "required": ["title"],
            "properties": {
                "title": { "type": "string" },
                "description": { "type": "string" },
                "deps": { "type": "array", "items": { "type": "string" } },
                "validators": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "required": ["kind"],
                        "properties": {
                            "kind": { "type": "string" },
                            "params": {}
                        }
                    }
                },
                "budget_usd": { "type": "number" },
                "budget_wall_s": { "type": "integer" },
                "parent": { "type": "string" }
            }
        })
    }
    fn dispatch(&self, args: &Value) -> ToolResult {
        let name = self.name();
        let title = match args.get("title").and_then(|v| v.as_str()) {
            Some(s) if !s.is_empty() => s.to_string(),
            _ => return ToolResult::local_err(name, "missing `title`"),
        };

        let description = args.get("description").and_then(|v| v.as_str()).map(String::from);

        let deps: Vec<TaskId> = match args.get("deps") {
            Some(Value::Array(arr)) => {
                let mut out = Vec::with_capacity(arr.len());
                for v in arr {
                    let Some(s) = v.as_str() else {
                        return ToolResult::local_err(name, "deps entries must be strings");
                    };
                    match parse_task_id(s, name) {
                        Ok(id) => out.push(id),
                        Err(e) => return e,
                    }
                }
                out
            }
            Some(_) => return ToolResult::local_err(name, "deps must be an array"),
            None => Vec::new(),
        };

        let validators: Vec<StageSuccessCriterion> = match args.get("validators") {
            Some(v) => match serde_json::from_value::<Vec<StageSuccessCriterion>>(v.clone()) {
                Ok(crits) => crits,
                Err(e) => return ToolResult::local_err(
                    name,
                    format!("validators must match StageSuccessCriterion: {e}"),
                ),
            },
            None => Vec::new(),
        };

        let parent = match args.get("parent").and_then(|v| v.as_str()) {
            Some(s) => match parse_task_id(s, name) {
                Ok(id) => Some(id),
                Err(e) => return e,
            },
            None => None,
        };

        let budget_usd = args.get("budget_usd").and_then(|v| v.as_f64()).map(|x| x as f32);
        let budget_wall_s = args.get("budget_wall_s").and_then(|v| v.as_u64()).map(|x| x as u32);

        let now = Utc::now();
        let task = Task {
            task: TaskId::new(),
            title,
            status: TaskStatus::Todo,
            description,
            deps,
            parent,
            budget_usd,
            budget_wall_s,
            validators,
            created_at: now,
            updated_at: now,
            cost_usd: 0.0,
            attempts: 0,
            seed_from: None,
            extra: Default::default(),
        };

        let id = task.task;
        let result = self.0.with_plan(name, |plan| {
            plan.insert(task, "\n").map_err(|e| {
                ToolResult::local_err(name, format!("insert failed: {e}"))
            })
        });
        match result {
            Ok(()) => ToolResult::local_ok(name, json!({ "task_id": id.to_string() })),
            Err(e) => e,
        }
    }
}

// ─── plan.update ───────────────────────────────────────────────────

