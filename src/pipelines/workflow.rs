//! Cooperative workflow runner.
//!
//! `wavelet workflow run` is *not* an autonomous executor that calls
//! backend models on the agent's behalf — it is an idempotent
//! state-machine walker. Given a pipeline definition and a working
//! directory, it answers exactly one question:
//!
//!   "What's the next thing that needs to happen?"
//!
//! The agent (or a human) does the actual stage work, then re-runs
//! `wavelet workflow run` to advance. Stage completion is inferred from
//! files appearing in the workdir — every stage's
//! `required_artifacts_out` must exist before the runner considers it
//! done. This keeps the runner's responsibility narrow (state + gating
//! + reporting) and avoids encoding fragile recipes for what each
//! stage's tools actually do.

use std::path::{Path, PathBuf};

use serde::Serialize;

use super::schema::{Pipeline, Stage};

/// Step-level verdict for a single stage.
#[derive(Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case", tag = "status")]
pub enum StageStatus {
    /// Every `required_artifacts_out` is present on disk; stage is done.
    Complete,
    /// `required_artifacts_in` are all satisfied; outputs are missing.
    /// The agent should now do the stage work.
    Ready {
        /// Outputs the stage still owes.
        missing_outputs: Vec<String>,
    },
    /// One or more `required_artifacts_in` are missing — an upstream
    /// stage hasn't produced them yet.
    Blocked {
        /// Inputs missing on disk.
        missing_inputs: Vec<String>,
    },
}

/// Per-stage report inside a [`WorkflowReport`].
#[derive(Debug, Serialize)]
pub struct StageReport<'a> {
    /// Stage name as declared in the pipeline.
    pub name: &'a str,
    /// Tools the stage may call (capability allowlist).
    pub tools: &'a [String],
    /// Verdict for this stage.
    #[serde(flatten)]
    pub status: StageStatus,
}

/// Aggregate report — what to do next, and the per-stage state.
#[derive(Debug, Serialize)]
pub struct WorkflowReport<'a> {
    /// Pipeline name.
    pub pipeline: &'a str,
    /// Working directory the report was computed against.
    pub workdir: String,
    /// Name of the next stage the agent should work on; `None` when
    /// every stage is complete.
    pub next_stage: Option<&'a str>,
    /// `true` when every stage is complete.
    pub complete: bool,
    /// Per-stage state.
    pub stages: Vec<StageReport<'a>>,
}

/// Compute the workflow report.
///
/// `workdir` is the root every `required_artifacts_in/out` path is
/// resolved against. Paths ending in `/` (a trailing slash) are
/// treated as directories — present iff the directory exists and is
/// non-empty.
pub fn compute_report<'a>(pipeline: &'a Pipeline, workdir: &Path) -> WorkflowReport<'a> {
    let mut stages = Vec::with_capacity(pipeline.stages.len());
    let mut next_stage: Option<&str> = None;

    for stage in &pipeline.stages {
        let status = classify_stage(stage, workdir);
        if next_stage.is_none() && !matches!(status, StageStatus::Complete) {
            next_stage = Some(stage.name.as_str());
        }
        stages.push(StageReport {
            name: stage.name.as_str(),
            tools: stage.tools_available.as_slice(),
            status,
        });
    }

    let complete = next_stage.is_none();
    WorkflowReport {
        pipeline: pipeline.name.as_str(),
        workdir: workdir.display().to_string(),
        next_stage,
        complete,
        stages,
    }
}

fn classify_stage(stage: &Stage, workdir: &Path) -> StageStatus {
    let missing_inputs: Vec<String> = stage
        .required_artifacts_in
        .iter()
        .filter(|p| !artifact_present(workdir, p))
        .cloned()
        .collect();
    if !missing_inputs.is_empty() {
        return StageStatus::Blocked { missing_inputs };
    }

    let missing_outputs: Vec<String> = stage
        .required_artifacts_out
        .iter()
        .filter(|p| !artifact_present(workdir, p))
        .cloned()
        .collect();
    if missing_outputs.is_empty() {
        StageStatus::Complete
    } else {
        StageStatus::Ready { missing_outputs }
    }
}

fn artifact_present(workdir: &Path, artifact: &str) -> bool {
    if let Some(rest) = artifact.strip_prefix("refs:") {
        return refs_present(workdir, rest);
    }
    if let Some(alts) = artifact.strip_prefix("any:") {
        return alts.split('|').any(|alt| artifact_present(workdir, alt.trim()));
    }
    let path: PathBuf = workdir.join(artifact);
    if artifact.ends_with('/') {
        // Directory artifact: present iff the dir exists and is non-empty.
        match std::fs::read_dir(&path) {
            Ok(mut rd) => rd.next().is_some(),
            Err(_) => false,
        }
    } else {
        path.exists()
    }
}

/// Resolve a `refs:<kind>[:min=N]` virtual artifact. Returns `true` when
/// at least `min` (default `1`) `.clip.html` files of the requested kind
/// exist under `<workdir>/refs/<kind>/`.
fn refs_present(workdir: &Path, spec: &str) -> bool {
    let mut parts = spec.split(':');
    let kind = match parts.next() {
        Some(k) if !k.is_empty() => k,
        _ => return false,
    };
    let mut min: usize = 1;
    for p in parts {
        if let Some(v) = p.strip_prefix("min=") {
            if let Ok(n) = v.parse() {
                min = n;
            }
        }
    }
    let dir = workdir.join("refs").join(kind);
    let count = std::fs::read_dir(&dir)
        .map(|rd| {
            rd.filter_map(Result::ok)
                .filter(|e| {
                    e.path()
                        .to_string_lossy()
                        .ends_with(".clip.html")
                })
                .count()
        })
        .unwrap_or(0);
    count >= min
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pipelines::load_from_str;
    use std::fs;

    const FIXTURE: &str = r#"
name: chain
version: "0.1"
description: x
stages:
  - name: a
    description: x
    required_artifacts_out: ["alpha.json"]
    tools_available: ["brief check"]
    success_criteria: []
  - name: b
    description: x
    required_artifacts_in: ["alpha.json"]
    required_artifacts_out: ["beta.json", "shots/"]
    tools_available: ["render"]
    success_criteria: []
  - name: c
    description: x
    required_artifacts_in: ["beta.json"]
    required_artifacts_out: ["gamma.mp4"]
    tools_available: ["c2pa sign"]
    success_criteria: []
orchestration:
  budget_default_usd: 1.0
  max_revisions_per_stage: 1
  max_send_backs: 1
  max_wall_time_minutes: 10
"#;

    #[test]
    fn empty_workdir_starts_at_first_stage() {
        let pipeline = load_from_str(FIXTURE).unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let report = compute_report(&pipeline, tmp.path());
        assert_eq!(report.next_stage, Some("a"));
        assert!(!report.complete);
        assert!(matches!(report.stages[0].status, StageStatus::Ready { .. }));
        assert!(matches!(report.stages[1].status, StageStatus::Blocked { .. }));
    }

    #[test]
    fn first_stage_output_advances_to_second() {
        let pipeline = load_from_str(FIXTURE).unwrap();
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join("alpha.json"), "{}").unwrap();
        let report = compute_report(&pipeline, tmp.path());
        assert_eq!(report.next_stage, Some("b"));
        assert!(matches!(report.stages[0].status, StageStatus::Complete));
        match &report.stages[1].status {
            StageStatus::Ready { missing_outputs } => {
                assert!(missing_outputs.contains(&"beta.json".to_string()));
                assert!(missing_outputs.contains(&"shots/".to_string()));
            }
            other => panic!("expected Ready, got {other:?}"),
        }
    }

    #[test]
    fn directory_artifact_requires_non_empty() {
        let pipeline = load_from_str(FIXTURE).unwrap();
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join("alpha.json"), "{}").unwrap();
        fs::write(tmp.path().join("beta.json"), "{}").unwrap();
        fs::create_dir_all(tmp.path().join("shots")).unwrap();
        // Empty directory: stage b still Ready (missing shots/)
        let report = compute_report(&pipeline, tmp.path());
        match &report.stages[1].status {
            StageStatus::Ready { missing_outputs } => {
                assert_eq!(missing_outputs, &vec!["shots/".to_string()]);
            }
            other => panic!("expected Ready, got {other:?}"),
        }
        // Add a file to shots/: stage b now Complete
        fs::write(tmp.path().join("shots/shot-1.mp4"), "x").unwrap();
        let report = compute_report(&pipeline, tmp.path());
        assert!(matches!(report.stages[1].status, StageStatus::Complete));
        assert_eq!(report.next_stage, Some("c"));
    }

    #[test]
    fn optional_artifacts_out_dont_gate_completion() {
        let fixture = r#"
name: chain
version: "0.1"
description: x
stages:
  - name: edit
    description: x
    required_artifacts_out: ["cuts.edl"]
    optional_artifacts_out: ["captions.json"]
    tools_available: ["captions align"]
    success_criteria: []
orchestration:
  budget_default_usd: 1.0
  max_revisions_per_stage: 1
  max_send_backs: 1
  max_wall_time_minutes: 10
"#;
        let pipeline = load_from_str(fixture).unwrap();
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join("cuts.edl"), "x").unwrap();
        let report = compute_report(&pipeline, tmp.path());
        assert!(report.complete);
        assert!(matches!(report.stages[0].status, StageStatus::Complete));
    }

    #[test]
    fn all_outputs_present_means_complete() {
        let pipeline = load_from_str(FIXTURE).unwrap();
        let tmp = tempfile::tempdir().unwrap();
        for name in ["alpha.json", "beta.json", "gamma.mp4"] {
            fs::write(tmp.path().join(name), "x").unwrap();
        }
        fs::create_dir_all(tmp.path().join("shots")).unwrap();
        fs::write(tmp.path().join("shots/shot-1.mp4"), "x").unwrap();
        let report = compute_report(&pipeline, tmp.path());
        assert!(report.complete);
        assert_eq!(report.next_stage, None);
    }

    const REFS_FIXTURE: &str = r#"
name: chain
version: "0.1"
description: x
stages:
  - name: script
    description: x
    required_artifacts_out: ["script.fountain", "any:screenplay.json|refs:screenplay-scene"]
    tools_available: ["screenplay parse"]
    success_criteria: []
  - name: assets
    description: x
    required_artifacts_in: ["any:screenplay.json|refs:screenplay-scene"]
    required_artifacts_out: ["any:music/track.wav|refs:music"]
    tools_available: ["music gen"]
    success_criteria: []
orchestration:
  budget_default_usd: 1.0
  max_revisions_per_stage: 1
  max_send_backs: 1
  max_wall_time_minutes: 10
"#;

    #[test]
    fn refs_artifact_satisfies_when_clipref_exists() {
        let pipeline = load_from_str(REFS_FIXTURE).unwrap();
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join("script.fountain"), "INT. X - DAY\n").unwrap();
        fs::create_dir_all(tmp.path().join("refs/screenplay-scene")).unwrap();
        fs::write(
            tmp.path().join("refs/screenplay-scene/001-x-abc123.clip.html"),
            "x",
        )
        .unwrap();
        let report = compute_report(&pipeline, tmp.path());
        assert!(matches!(report.stages[0].status, StageStatus::Complete));
        assert_eq!(report.next_stage, Some("assets"));
    }

    #[test]
    fn any_alternative_falls_back_to_legacy_path() {
        let pipeline = load_from_str(REFS_FIXTURE).unwrap();
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join("script.fountain"), "x").unwrap();
        fs::write(tmp.path().join("screenplay.json"), "{}").unwrap();
        let report = compute_report(&pipeline, tmp.path());
        assert!(matches!(report.stages[0].status, StageStatus::Complete));
    }

    #[test]
    fn refs_min_count_enforced() {
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir_all(tmp.path().join("refs/shot")).unwrap();
        fs::write(tmp.path().join("refs/shot/a.clip.html"), "x").unwrap();
        assert!(refs_present(tmp.path(), "shot"));
        assert!(refs_present(tmp.path(), "shot:min=1"));
        assert!(!refs_present(tmp.path(), "shot:min=2"));
        fs::write(tmp.path().join("refs/shot/b.clip.html"), "x").unwrap();
        assert!(refs_present(tmp.path(), "shot:min=2"));
    }
}
