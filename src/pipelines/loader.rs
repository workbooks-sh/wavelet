//! YAML loader for pipeline definitions.

use std::collections::HashSet;
use std::fs;
use std::path::Path;

use thiserror::Error;

use super::schema::Pipeline;

/// Errors a load can produce. Parse errors surface the underlying
/// serde_yaml message; semantic errors carry a static reason string.
#[derive(Debug, Error)]
pub enum LoadError {
    /// File could not be read off disk.
    #[error("read {path}: {source}")]
    Io {
        /// File the loader tried to read.
        path: String,
        /// Underlying io error.
        #[source]
        source: std::io::Error,
    },

    /// YAML didn't parse, or didn't match the schema.
    #[error("parse {path}: {source}")]
    Parse {
        /// File whose contents failed to parse.
        path: String,
        /// Underlying serde_yaml error.
        #[source]
        source: serde_yaml::Error,
    },

    /// Schema parsed but failed a structural invariant.
    #[error("invalid pipeline `{name}`: {reason}")]
    Invalid {
        /// Pipeline name as declared in the YAML.
        name: String,
        /// What was wrong.
        reason: String,
    },
}

/// Load + validate a pipeline from a path.
pub fn load_from_path(path: &Path) -> Result<Pipeline, LoadError> {
    let contents = fs::read_to_string(path).map_err(|source| LoadError::Io {
        path: path.display().to_string(),
        source,
    })?;
    let pipeline: Pipeline =
        serde_yaml::from_str(&contents).map_err(|source| LoadError::Parse {
            path: path.display().to_string(),
            source,
        })?;
    validate(&pipeline)?;
    Ok(pipeline)
}

/// Load + validate from a YAML string (test helper / stdin path).
pub fn load_from_str(yaml: &str) -> Result<Pipeline, LoadError> {
    let pipeline: Pipeline =
        serde_yaml::from_str(yaml).map_err(|source| LoadError::Parse {
            path: "<inline>".into(),
            source,
        })?;
    validate(&pipeline)?;
    Ok(pipeline)
}

fn validate(p: &Pipeline) -> Result<(), LoadError> {
    if p.name.trim().is_empty() {
        return Err(LoadError::Invalid {
            name: p.name.clone(),
            reason: "name is empty".into(),
        });
    }
    if p.stages.is_empty() {
        return Err(LoadError::Invalid {
            name: p.name.clone(),
            reason: "no stages declared".into(),
        });
    }

    let mut seen_names: HashSet<&str> = HashSet::new();
    let mut produced: HashSet<&str> = HashSet::new();
    for stage in &p.stages {
        if !seen_names.insert(stage.name.as_str()) {
            return Err(LoadError::Invalid {
                name: p.name.clone(),
                reason: format!("duplicate stage `{}`", stage.name),
            });
        }
        if stage.required_artifacts_out.is_empty() {
            return Err(LoadError::Invalid {
                name: p.name.clone(),
                reason: format!(
                    "stage `{}` declares no required_artifacts_out",
                    stage.name
                ),
            });
        }
        for needed in &stage.required_artifacts_in {
            if !produced.contains(needed.as_str()) {
                return Err(LoadError::Invalid {
                    name: p.name.clone(),
                    reason: format!(
                        "stage `{}` requires artifact `{}` which no earlier stage produces",
                        stage.name, needed
                    ),
                });
            }
        }
        for produced_name in &stage.required_artifacts_out {
            produced.insert(produced_name.as_str());
        }
        for produced_name in &stage.optional_artifacts_out {
            produced.insert(produced_name.as_str());
        }
    }

    if p.orchestration.budget_default_usd <= 0.0 {
        return Err(LoadError::Invalid {
            name: p.name.clone(),
            reason: "orchestration.budget_default_usd must be > 0".into(),
        });
    }
    if p.orchestration.max_wall_time_minutes == 0 {
        return Err(LoadError::Invalid {
            name: p.name.clone(),
            reason: "orchestration.max_wall_time_minutes must be > 0".into(),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const MINIMAL: &str = r#"
name: minimal
version: "0.1.0"
description: "Smallest valid pipeline"
stages:
  - name: alpha
    description: "first stage"
    required_artifacts_out: ["foo.json"]
    tools_available: ["brief check"]
    success_criteria:
      - kind: artifact_exists
        params: { path: foo.json }
  - name: beta
    description: "second stage"
    required_artifacts_in: ["foo.json"]
    required_artifacts_out: ["bar.mp4"]
    tools_available: ["render"]
    success_criteria:
      - kind: artifact_exists
        params: { path: bar.mp4 }
orchestration:
  budget_default_usd: 5.0
  max_revisions_per_stage: 2
  max_send_backs: 1
  max_wall_time_minutes: 30
"#;

    #[test]
    fn parses_minimal_pipeline() {
        let p = load_from_str(MINIMAL).unwrap();
        assert_eq!(p.name, "minimal");
        assert_eq!(p.stages.len(), 2);
        assert_eq!(p.stages[1].required_artifacts_in, vec!["foo.json"]);
    }

    #[test]
    fn rejects_unknown_field() {
        let yaml = MINIMAL.replace("version:", "verison:");
        let err = load_from_str(&yaml).unwrap_err();
        assert!(matches!(err, LoadError::Parse { .. }));
    }

    #[test]
    fn rejects_missing_upstream_artifact() {
        let yaml = r#"
name: bad
version: "0.1"
description: "x"
stages:
  - name: a
    description: "x"
    required_artifacts_in: ["never.json"]
    required_artifacts_out: ["out"]
    tools_available: []
    success_criteria: []
orchestration:
  budget_default_usd: 1.0
  max_revisions_per_stage: 1
  max_send_backs: 1
  max_wall_time_minutes: 1
"#;
        let err = load_from_str(yaml).unwrap_err();
        match err {
            LoadError::Invalid { reason, .. } => {
                assert!(reason.contains("never.json"), "got: {reason}");
            }
            other => panic!("expected Invalid, got {other:?}"),
        }
    }

    #[test]
    fn parses_tier_policy() {
        let yaml = format!(
            "{MINIMAL}\ntier_policy:\n  draft:\n    i2v: veo-3.1-fast\n    still: google-nano-banana-3\n  hero:\n    i2v: veo-3.1\n    still: google-nano-banana-3\n"
        );
        let p = load_from_str(&yaml).unwrap();
        let tier = p.tier_policy.as_ref().expect("tier_policy missing");
        assert_eq!(tier.get("draft").and_then(|t| t.get("i2v")).map(String::as_str), Some("veo-3.1-fast"));
        assert_eq!(tier.get("hero").and_then(|t| t.get("still")).map(String::as_str), Some("google-nano-banana-3"));
    }

    #[test]
    fn rejects_duplicate_stage_name() {
        let yaml = r#"
name: dup
version: "0.1"
description: "x"
stages:
  - name: a
    description: "x"
    required_artifacts_out: ["x"]
    tools_available: []
    success_criteria: []
  - name: a
    description: "y"
    required_artifacts_out: ["y"]
    tools_available: []
    success_criteria: []
orchestration:
  budget_default_usd: 1.0
  max_revisions_per_stage: 1
  max_send_backs: 1
  max_wall_time_minutes: 1
"#;
        let err = load_from_str(yaml).unwrap_err();
        assert!(matches!(err, LoadError::Invalid { .. }));
    }
}
