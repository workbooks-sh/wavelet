//! `plan.seed` tool.

#![allow(missing_docs)]

use serde_json::{json, Value};
use crate::agent::plan::{Plan, Task, TaskId, TaskStatus};
use crate::agent::tools::{Tool, ToolResult};
use super::PlanToolCtx;
use crate::agent::plan::seed::seed_from_pipeline;
use super::{resolve_pipeline_path};

pub struct PlanSeed(pub(super) PlanToolCtx);
impl Tool for PlanSeed {
    fn name(&self) -> &str { "plan.seed" }
    fn description(&self) -> &str {
        "Seed the plan from a pipeline_defs/*.yaml definition — one task per \
         stage, validators lifted from success_criteria, deps form a linear \
         chain. Overlays on top of existing tasks; no dedupe."
    }
    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "required": ["pipeline"],
            "properties": {
                "pipeline": {
                    "type": "string",
                    "description": "Pipeline definition to seed from. Resolves to `pipeline_defs/<pipeline>.yaml` in the wavelet crate. Known: 'commercial'.",
                }
            }
        })
    }
    fn dispatch(&self, args: &Value) -> ToolResult {
        let name = self.name();
        let pipeline = match args.get("pipeline").and_then(|v| v.as_str()) {
            Some(s) if !s.is_empty() => s.to_string(),
            _ => return ToolResult::local_err(name, "missing `pipeline`"),
        };
        let path = match resolve_pipeline_path(&pipeline) {
            Some(p) => p,
            None => {
                return ToolResult {
                    ok: false,
                    response: json!({
                        "error": "pipeline_not_found",
                        "pipeline": pipeline,
                    }),
                    summary: format!("{name}: pipeline_not_found `{pipeline}`"),
                    output_files: Vec::new(),
                    cost_usd: 0.0,
                };
            }
        };

        let result = self.0.with_plan(name, |plan| {
            seed_from_pipeline(plan, &path).map_err(|e| {
                ToolResult::local_err(name, format!("seed failed: {e}"))
            })
        });
        match result {
            Ok(ids) => {
                let id_strs: Vec<String> = ids.iter().map(|i| i.to_string()).collect();
                let count = id_strs.len();
                ToolResult::local_ok(name, json!({
                    "ok": true,
                    "pipeline": pipeline,
                    "stage_count": count,
                    "seeded_task_ids": id_strs,
                }))
            }
            Err(e) => e,
        }
    }
}

// ─── plan.done ─────────────────────────────────────────────────────

