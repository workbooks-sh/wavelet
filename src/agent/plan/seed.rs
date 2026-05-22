//! Seed a Plan from a `pipeline_defs/*.yaml` declaration (wb-mqsb.6).
//!
//! One stage → one `Task`. `success_criteria` lift verbatim into the
//! task's `validators` (both sides use `StageSuccessCriterion`). Deps
//! form a linear chain over stage order — the on-disk pipeline already
//! carries `required_artifacts_in/out` for the topology, but the agent
//! reads the pipeline as a sequence and we mirror that.
//!
//! Calling `seed_from_pipeline` twice overlays a fresh batch of tasks
//! on top of the existing plan with new ULIDs — no dedupe, no removal
//! of stale tasks. Callers that want a clean slate should clear the
//! workdir first.

use std::collections::BTreeMap;
use std::path::Path;

use chrono::Utc;

use super::schema::{Plan, PlanError, Task, TaskId, TaskStatus};
use crate::pipelines::loader::{load_from_path, LoadError};

/// Errors surfaced when seeding a plan from a pipeline YAML.
#[derive(Debug, thiserror::Error)]
pub enum SeedError {
    /// The pipeline YAML could not be loaded.
    #[error("load pipeline: {0}")]
    Load(#[from] LoadError),
    /// A task write or in-memory invariant failed.
    #[error("plan: {0}")]
    Plan(#[from] PlanError),
}

/// Load `pipeline_def_path` and insert one `Task` per stage into
/// `plan`. Returns the new task IDs in stage order.
pub fn seed_from_pipeline(
    plan: &mut Plan,
    pipeline_def_path: &Path,
) -> Result<Vec<TaskId>, SeedError> {
    let pipeline = load_from_path(pipeline_def_path)?;
    let pipeline_label = pipeline_def_path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("pipeline.yaml")
        .to_string();

    let mut ids = Vec::with_capacity(pipeline.stages.len());
    let mut previous: Option<TaskId> = None;
    let now = Utc::now();

    for stage in &pipeline.stages {
        let id = TaskId::new();
        let mut extra: BTreeMap<String, serde_yaml::Value> = BTreeMap::new();
        if !stage.required_artifacts_in.is_empty() {
            extra.insert(
                "required-artifacts-in".into(),
                serde_yaml::to_value(&stage.required_artifacts_in)
                    .unwrap_or(serde_yaml::Value::Null),
            );
        }
        if !stage.required_artifacts_out.is_empty() {
            extra.insert(
                "required-artifacts-out".into(),
                serde_yaml::to_value(&stage.required_artifacts_out)
                    .unwrap_or(serde_yaml::Value::Null),
            );
        }
        if !stage.optional_artifacts_out.is_empty() {
            extra.insert(
                "optional-artifacts-out".into(),
                serde_yaml::to_value(&stage.optional_artifacts_out)
                    .unwrap_or(serde_yaml::Value::Null),
            );
        }
        if !stage.tools_available.is_empty() {
            extra.insert(
                "tools-available".into(),
                serde_yaml::to_value(&stage.tools_available)
                    .unwrap_or(serde_yaml::Value::Null),
            );
        }

        let task = Task {
            task: id,
            title: stage.name.clone(),
            status: TaskStatus::Todo,
            description: Some(stage.description.clone()),
            deps: previous.map(|p| vec![p]).unwrap_or_default(),
            parent: None,
            budget_usd: None,
            budget_wall_s: None,
            validators: stage.success_criteria.clone(),
            created_at: now,
            updated_at: now,
            cost_usd: 0.0,
            attempts: 0,
            seed_from: Some(format!("{pipeline_label}/{}", stage.name)),
            extra,
        };

        plan.insert(task, "\n")?;
        ids.push(id);
        previous = Some(id);
    }

    Ok(ids)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn commercial_yaml_path() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("pipeline_defs")
            .join("commercial.yaml")
    }

    #[test]
    fn seed_from_commercial_yaml_creates_8_tasks() {
        let dir = tempfile::tempdir().unwrap();
        let mut plan = Plan::load(dir.path()).unwrap();
        let ids = seed_from_pipeline(&mut plan, &commercial_yaml_path()).unwrap();

        assert_eq!(ids.len(), 8);
        assert_eq!(plan.tasks.len(), 8);

        let first = plan.tasks.get(&ids[0]).unwrap();
        assert!(first.deps.is_empty(), "first stage must have no deps");

        let last = plan.tasks.get(&ids[7]).unwrap();
        assert_eq!(last.deps, vec![ids[6]]);

        // Walk back through the chain — 7 hops from publish to research.
        let mut cur = ids[7];
        let mut hops = 0;
        while let Some(prev) = plan.tasks.get(&cur).and_then(|t| t.deps.first().copied()) {
            cur = prev;
            hops += 1;
            if hops > 16 {
                panic!("dep walk runaway");
            }
        }
        assert_eq!(hops, 7);
        assert_eq!(cur, ids[0]);
    }

    #[test]
    fn seeded_tasks_carry_success_criteria() {
        let dir = tempfile::tempdir().unwrap();
        let mut plan = Plan::load(dir.path()).unwrap();
        let ids = seed_from_pipeline(&mut plan, &commercial_yaml_path()).unwrap();

        let research = plan.tasks.get(&ids[0]).unwrap();
        assert_eq!(research.title, "research");
        let kinds: Vec<&str> = research.validators.iter().map(|c| c.kind.as_str()).collect();
        assert!(kinds.contains(&"artifact_exists"), "got kinds: {kinds:?}");
        assert!(kinds.contains(&"brief_check_passes"), "got kinds: {kinds:?}");

        assert!(research.extra.contains_key("required-artifacts-out"));
        assert!(research.extra.contains_key("tools-available"));
        assert_eq!(
            research.seed_from.as_deref(),
            Some("commercial.yaml/research"),
        );
    }

    #[test]
    fn double_seed_overlays_new_tasks() {
        let dir = tempfile::tempdir().unwrap();
        let mut plan = Plan::load(dir.path()).unwrap();
        let _ = seed_from_pipeline(&mut plan, &commercial_yaml_path()).unwrap();
        let _ = seed_from_pipeline(&mut plan, &commercial_yaml_path()).unwrap();
        assert_eq!(plan.tasks.len(), 16);
    }
}
