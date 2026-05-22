//! `plan.update` tool.

#![allow(missing_docs)]

use serde_json::{json, Value};
use crate::agent::plan::{Plan, Task, TaskId, TaskStatus};
use crate::agent::tools::{Tool, ToolResult};
use super::PlanToolCtx;
use crate::pipelines::schema::StageSuccessCriterion;
use chrono::Utc;
use super::{parse_task_id, task_to_json};

pub struct PlanUpdate(pub(super) PlanToolCtx);
impl Tool for PlanUpdate {
    fn name(&self) -> &str { "plan.update" }
    fn description(&self) -> &str {
        "Update one task's editable fields (title/description/deps/validators/budgets). \
         Status transitions go through plan.complete/reopen/abandon."
    }
    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "required": ["task_id", "fields"],
            "properties": {
                "task_id": { "type": "string" },
                "fields": {
                    "type": "object",
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
                        "budget_wall_s": { "type": "integer" }
                    }
                }
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
        let fields = match args.get("fields") {
            Some(Value::Object(m)) => m.clone(),
            _ => return ToolResult::local_err(name, "missing `fields` object"),
        };

        if fields.contains_key("status") {
            return ToolResult::local_err(
                name,
                "cannot set `status` directly; use plan.complete / plan.reopen / plan.abandon",
            );
        }

        let result = self.0.with_plan(name, |plan| {
            let mut task = match plan.tasks.get(&id).cloned() {
                Some(t) => t,
                None => return Err(ToolResult::local_err(name, format!("unknown task_id {id}"))),
            };

            if let Some(v) = fields.get("title").and_then(|v| v.as_str()) {
                task.title = v.to_string();
            }
            if let Some(v) = fields.get("description") {
                task.description = v.as_str().map(String::from);
            }
            if let Some(v) = fields.get("deps") {
                let arr = match v.as_array() {
                    Some(a) => a,
                    None => return Err(ToolResult::local_err(name, "deps must be an array")),
                };
                let mut out = Vec::with_capacity(arr.len());
                for v in arr {
                    let Some(s) = v.as_str() else {
                        return Err(ToolResult::local_err(name, "deps entries must be strings"));
                    };
                    out.push(parse_task_id(s, name)?);
                }
                task.deps = out;
            }
            if let Some(v) = fields.get("validators") {
                match serde_json::from_value::<Vec<StageSuccessCriterion>>(v.clone()) {
                    Ok(c) => task.validators = c,
                    Err(e) => return Err(ToolResult::local_err(
                        name,
                        format!("validators must match StageSuccessCriterion: {e}"),
                    )),
                }
            }
            if let Some(v) = fields.get("budget_usd") {
                task.budget_usd = v.as_f64().map(|x| x as f32);
            }
            if let Some(v) = fields.get("budget_wall_s") {
                task.budget_wall_s = v.as_u64().map(|x| x as u32);
            }

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

// ─── plan.complete ─────────────────────────────────────────────────

