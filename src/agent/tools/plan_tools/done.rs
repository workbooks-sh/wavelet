//! `plan.done` tool.

#![allow(missing_docs)]

use serde_json::{json, Value};
use crate::agent::plan::{Plan, Task, TaskId, TaskStatus};
use crate::agent::tools::{Tool, ToolResult};
use super::PlanToolCtx;
use std::sync::atomic::Ordering;

pub struct PlanDone(pub(super) PlanToolCtx);
impl Tool for PlanDone {
    fn name(&self) -> &str { "plan.done" }
    fn description(&self) -> &str {
        "Signal that the plan is fully complete and the loop should wrap up. \
         No-op other than setting a completion flag the orchestrator polls."
    }
    fn parameters_schema(&self) -> Value {
        json!({ "type": "object", "properties": {} })
    }
    fn dispatch(&self, _args: &Value) -> ToolResult {
        let name = self.name();
        let outer = self.0.plan.lock().expect("plan cell poisoned");
        if outer.is_none() {
            return ToolResult::local_err(name, "plan_mode_off");
        }
        drop(outer);
        self.0.completion.store(true, Ordering::SeqCst);
        ToolResult::local_ok(name, json!({ "ok": true, "completion_signaled": true }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
        use std::path::PathBuf;
        use crate::agent::session::PlanCell;
        use std::sync::Arc;
        use crate::agent::plan::validator::ValidatorRegistry;
        use crate::agent::tools::ToolRegistry;
        use crate::agent::tools::plan_tools::{register_with_plan, register_with_plan_and_completion};
        use crate::agent::session::empty_completion_flag;

    use crate::agent::plan::schema::{Plan, TaskStatus};
    use std::fs;
    use std::sync::Mutex;
    use tempfile::TempDir;

    struct Harness {
        _dir: TempDir,
        workdir: PathBuf,
        plan_cell: PlanCell,
        registry: ToolRegistry,
    }

    fn harness() -> Harness {
        let dir = tempfile::tempdir().unwrap();
        let workdir = dir.path().to_path_buf();
        let plan = Plan::load(&workdir).unwrap();
        let inner = Arc::new(Mutex::new(plan));
        let plan_cell: PlanCell = Arc::new(Mutex::new(Some(inner)));
        let validators = Arc::new(ValidatorRegistry::with_builtins());
        let mut registry = ToolRegistry::new();
        register_with_plan(&mut registry, plan_cell.clone(), validators);
        Harness { _dir: dir, workdir, plan_cell, registry }
    }

    fn call(h: &Harness, name: &str, args: Value) -> ToolResult {
        h.registry.get(name).unwrap_or_else(|| panic!("no tool {name}")).dispatch(&args)
    }

    fn current_status(h: &Harness, id: &str) -> TaskStatus {
        let plan = h.plan_cell.lock().unwrap();
        let inner = plan.as_ref().unwrap().lock().unwrap();
        let id = TaskId(ulid::Ulid::from_string(id).unwrap());
        inner.tasks.get(&id).unwrap().status
    }

    #[test]
    fn add_and_show_round_trip() {
        let h = harness();
        let r = call(&h, "plan.add", json!({
            "title": "draft brief",
            "description": "200 words",
        }));
        assert!(r.ok, "add failed: {:?}", r.response);
        let id = r.response["task_id"].as_str().unwrap().to_string();

        let s = call(&h, "plan.show", json!({}));
        assert!(s.ok);
        let tasks = s.response["tasks"].as_array().unwrap();
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0]["task"], Value::String(id.clone()));
        assert_eq!(tasks[0]["title"], Value::String("draft brief".into()));
    }

    #[test]
    fn complete_with_passing_validator_marks_done() {
        let h = harness();
        fs::write(h.workdir.join("brief.md"), b"hi\n").unwrap();
        let r = call(&h, "plan.add", json!({
            "title": "write brief",
            "validators": [ { "kind": "artifact_exists", "params": { "path": "brief.md" } } ],
        }));
        let id = r.response["task_id"].as_str().unwrap().to_string();

        let c = call(&h, "plan.complete", json!({ "task_id": id }));
        assert!(c.ok);
        assert_eq!(c.response["ok"], Value::Bool(true));
        assert_eq!(current_status(&h, &id), TaskStatus::Done);
    }

    #[test]
    fn complete_with_failing_validator_keeps_status_and_returns_detail() {
        let h = harness();
        let r = call(&h, "plan.add", json!({
            "title": "write brief",
            "validators": [ { "kind": "artifact_exists", "params": { "path": "missing.md" } } ],
        }));
        let id = r.response["task_id"].as_str().unwrap().to_string();

        let c = call(&h, "plan.complete", json!({ "task_id": id }));
        assert!(c.ok);
        assert_eq!(c.response["ok"], Value::Bool(false));
        let failed = c.response["failed_validators"].as_array().unwrap();
        assert_eq!(failed.len(), 1);
        assert_eq!(failed[0]["kind"], Value::String("artifact_exists".into()));
        assert_eq!(failed[0]["detail"]["exists"], Value::Bool(false));
        assert_ne!(current_status(&h, &id), TaskStatus::Done);
    }

    #[test]
    fn update_rejects_status_field() {
        let h = harness();
        let r = call(&h, "plan.add", json!({ "title": "t" }));
        let id = r.response["task_id"].as_str().unwrap().to_string();

        let u = call(&h, "plan.update", json!({
            "task_id": id,
            "fields": { "status": "done" },
        }));
        assert!(!u.ok);
        let err = u.response["error"].as_str().unwrap();
        assert!(err.contains("status"), "err was: {err}");
    }

    #[test]
    fn fork_creates_child_with_parent_link_and_leaves_original() {
        let h = harness();
        let r = call(&h, "plan.add", json!({
            "title": "parent",
            "validators": [ { "kind": "artifact_exists", "params": { "path": "x.md" } } ],
        }));
        let parent_id = r.response["task_id"].as_str().unwrap().to_string();
        let parent_path = h.workdir.join(format!("{parent_id}.task.html"));
        let parent_disk_before = fs::read_to_string(&parent_path).unwrap();

        let f = call(&h, "plan.fork", json!({
            "task_id": parent_id,
            "title": "child path",
        }));
        assert!(f.ok, "fork failed: {:?}", f.response);
        let child_id = f.response["new_task_id"].as_str().unwrap().to_string();
        assert_ne!(child_id, parent_id);

        let plan = h.plan_cell.lock().unwrap();
        let inner = plan.as_ref().unwrap().lock().unwrap();
        let child_tid = TaskId(ulid::Ulid::from_string(&child_id).unwrap());
        let parent_tid = TaskId(ulid::Ulid::from_string(&parent_id).unwrap());
        let child = inner.tasks.get(&child_tid).unwrap();
        assert_eq!(child.parent, Some(parent_tid));
        assert_eq!(child.status, TaskStatus::Todo);
        assert_eq!(child.title, "child path");
        assert_eq!(child.validators.len(), 1);

        let parent_disk_after = fs::read_to_string(&parent_path).unwrap();
        assert_eq!(parent_disk_before, parent_disk_after);
    }

    #[test]
    fn validate_mixed_done_and_doing_returns_per_task_entries() {
        let h = harness();
        fs::write(h.workdir.join("ok.md"), b"hi\n").unwrap();
        let a = call(&h, "plan.add", json!({
            "title": "a",
            "validators": [ { "kind": "artifact_exists", "params": { "path": "ok.md" } } ],
        }));
        let id_a = a.response["task_id"].as_str().unwrap().to_string();
        let b = call(&h, "plan.add", json!({
            "title": "b",
            "validators": [ { "kind": "artifact_exists", "params": { "path": "missing.md" } } ],
        }));
        let _id_b = b.response["task_id"].as_str().unwrap().to_string();

        let _ = call(&h, "plan.complete", json!({ "task_id": id_a }));
        // b stays in Todo — won't be picked up by default validate filter.
        // Flip b to Doing via internal mutation to ensure we hit a Doing
        // task in the validate pass.
        {
            let plan = h.plan_cell.lock().unwrap();
            let mut inner = plan.as_ref().unwrap().lock().unwrap();
            let mut tasks: Vec<TaskId> = inner.tasks.keys().copied().collect();
            tasks.sort();
            let id_b = tasks.iter().find(|tid| tid.to_string() != id_a).copied().unwrap();
            let mut t = inner.tasks.get(&id_b).cloned().unwrap();
            t.status = TaskStatus::Doing;
            inner.update(t, "\n").unwrap();
        }

        let v = call(&h, "plan.validate", json!({}));
        assert!(v.ok);
        let results = v.response["results"].as_array().unwrap();
        assert_eq!(results.len(), 2);
        let oks: Vec<bool> = results.iter().map(|r| r["ok"].as_bool().unwrap()).collect();
        assert!(oks.contains(&true));
        assert!(oks.contains(&false));
    }

    #[test]
    fn abandon_then_reopen_flips_status() {
        let h = harness();
        let r = call(&h, "plan.add", json!({ "title": "t" }));
        let id = r.response["task_id"].as_str().unwrap().to_string();

        let a = call(&h, "plan.abandon", json!({ "task_id": id, "reason": "scope-cut" }));
        assert!(a.ok);
        assert_eq!(current_status(&h, &id), TaskStatus::Abandoned);

        let ro = call(&h, "plan.reopen", json!({ "task_id": id }));
        assert!(ro.ok);
        assert_eq!(current_status(&h, &id), TaskStatus::Todo);
    }

    #[test]
    fn plan_mode_off_returns_structured_error() {
        let plan_cell: PlanCell = Arc::new(Mutex::new(None));
        let validators = Arc::new(ValidatorRegistry::with_builtins());
        let mut registry = ToolRegistry::new();
        register_with_plan(&mut registry, plan_cell, validators);

        for tool in &[
            "plan.add", "plan.update", "plan.complete", "plan.reopen",
            "plan.abandon", "plan.fork", "plan.validate", "plan.show",
            "plan.seed", "plan.done",
        ] {
            let args = match *tool {
                "plan.add" => json!({ "title": "t" }),
                "plan.update" => json!({
                    "task_id": "01JQX0000000000000000000AA",
                    "fields": { "title": "x" },
                }),
                "plan.show" | "plan.validate" | "plan.done" => json!({}),
                "plan.seed" => json!({ "pipeline": "commercial" }),
                _ => json!({ "task_id": "01JQX0000000000000000000AA" }),
            };
            let r = registry.get(tool).unwrap().dispatch(&args);
            assert!(!r.ok, "{tool} should fail when plan mode off");
            assert_eq!(
                r.response["error"], Value::String("plan_mode_off".into()),
                "{tool} should return plan_mode_off, got: {:?}", r.response,
            );
        }
    }

    #[test]
    fn plan_done_sentinel_sets_completion_flag() {
        let dir = tempfile::tempdir().unwrap();
        let plan = Plan::load(dir.path()).unwrap();
        let inner = Arc::new(Mutex::new(plan));
        let plan_cell: PlanCell = Arc::new(Mutex::new(Some(inner)));
        let validators = Arc::new(ValidatorRegistry::with_builtins());
        let completion = empty_completion_flag();
        let mut registry = ToolRegistry::new();
        register_with_plan_and_completion(
            &mut registry, plan_cell, validators, completion.clone(),
        );

        assert!(!completion.load(Ordering::SeqCst));
        let r = registry.get("plan.done").unwrap().dispatch(&json!({}));
        assert!(r.ok, "plan.done dispatch failed: {:?}", r.response);
        assert_eq!(r.response["completion_signaled"], Value::Bool(true));
        assert!(completion.load(Ordering::SeqCst));
    }

    #[test]
    fn add_rejects_invalid_validator_shape() {
        let h = harness();
        let r = call(&h, "plan.add", json!({
            "title": "bad",
            "validators": [ { "missing_kind": true } ],
        }));
        assert!(!r.ok);
    }

    #[test]
    fn plan_seed_tool_round_trip() {
        let h = harness();
        let r = call(&h, "plan.seed", json!({ "pipeline": "commercial" }));
        assert!(r.ok, "seed failed: {:?}", r.response);
        assert_eq!(r.response["ok"], Value::Bool(true));
        assert_eq!(r.response["pipeline"], Value::String("commercial".into()));
        assert_eq!(r.response["stage_count"], Value::Number(8.into()));
        let ids = r.response["seeded_task_ids"].as_array().unwrap();
        assert_eq!(ids.len(), 8);

        let s = call(&h, "plan.show", json!({}));
        let tasks = s.response["tasks"].as_array().unwrap();
        assert_eq!(tasks.len(), 8);
    }

    #[test]
    fn plan_seed_unknown_pipeline_returns_structured_error() {
        let h = harness();
        let r = call(&h, "plan.seed", json!({ "pipeline": "bogus" }));
        assert!(!r.ok);
        assert_eq!(r.response["error"], Value::String("pipeline_not_found".into()));
        assert_eq!(r.response["pipeline"], Value::String("bogus".into()));
    }
}

