//! Typed schema for a `pipeline.yaml`. Fields mirror OpenMontage's
//! `pipeline_defs/cinematic.yaml` so authors familiar with that project
//! see the same shape.
//!
//! Unknown fields are rejected (`deny_unknown_fields`) — typos in a
//! pipeline definition should fail loud instead of silently dropping
//! intended config.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// Top-level pipeline definition.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Pipeline {
    /// Human-readable pipeline name. Used as the lookup key by
    /// `wavelet pipelines run <name>`.
    pub name: String,

    /// Semver-ish version string. No parsing — opaque to the loader,
    /// surfaced in `pipelines list`.
    pub version: String,

    /// One-paragraph description shown in `pipelines list` / `show`.
    pub description: String,

    /// Ordered list of stages. Execution order = list order.
    pub stages: Vec<Stage>,

    /// Budget / retry / wall-time caps applied across every stage.
    pub orchestration: Orchestration,

    /// Optional reference-input declaration (a video the pipeline
    /// conditions on — e.g. style-transfer or shot-by-shot remake).
    /// Absent for spec/script-driven pipelines.
    #[serde(default)]
    pub reference_input: Option<ReferenceInput>,

    /// Optional cost-tier policy. Maps tier name (e.g. `draft`, `hero`)
    /// to artifact-kind → provider. The standard pattern is two tiers:
    /// `draft` (cheap models, used to assemble a previewable rough cut)
    /// and `hero` (premium models, used only on agent/user-approved
    /// shots after draft review). 5–10x cost reduction on a finished
    /// commercial without losing perceived quality. The runtime
    /// resolves a stage's tier policy when invoking backend adapters;
    /// this schema field carries the declaration only.
    #[serde(default)]
    pub tier_policy: Option<TierPolicy>,
}

/// One pipeline stage. The pipeline runtime hands the agent the
/// `tools_available` and asks it to produce every entry in
/// `required_artifacts_out`; the listed `success_criteria` decide
/// whether the stage passes.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Stage {
    /// Stage identifier. Unique within the pipeline.
    pub name: String,

    /// One-line summary of what this stage does.
    pub description: String,

    /// Artifact names (free-form strings) the stage expects to exist
    /// before it runs. The first stage typically has an empty list.
    #[serde(default)]
    pub required_artifacts_in: Vec<String>,

    /// Artifact names the stage must produce. Validated by the runtime
    /// before declaring the stage complete.
    pub required_artifacts_out: Vec<String>,

    /// Artifacts the stage may produce but isn't gated on — e.g. a VO
    /// captions track that's only meaningful when the brief includes
    /// dialogue. The workflow runner never reports a stage incomplete
    /// for a missing entry here; downstream stages that depend on these
    /// must guard with their own logic.
    #[serde(default)]
    pub optional_artifacts_out: Vec<String>,

    /// Wavelet subcommands (or external tool names) the stage may call.
    /// Acts as a capability allowlist for the agent driving the stage.
    pub tools_available: Vec<String>,

    /// Conditions that must all hold for the stage to pass.
    pub success_criteria: Vec<StageSuccessCriterion>,
}

/// A single success condition. Kept structured (not a free-form string)
/// so a runtime can grade outcomes mechanically. `kind` names a
/// well-known check; `params` carries its arguments.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct StageSuccessCriterion {
    /// Check identifier — e.g. `artifact_exists`, `vlm_verify_passes`,
    /// `cost_below_usd`, `wall_time_below_minutes`. Loader does not
    /// validate the set; the runtime decides what it knows how to grade.
    pub kind: String,

    /// Free-form parameters. Anything the named check needs.
    #[serde(default)]
    pub params: serde_yaml::Value,
}

/// Orchestration-level controls. The runtime enforces these across all
/// stages.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Orchestration {
    /// Total budget (USD) the pipeline is allowed to spend. The runtime
    /// hard-stops when cumulative spend crosses this.
    pub budget_default_usd: f64,

    /// Maximum regen attempts per stage on failed `success_criteria`.
    pub max_revisions_per_stage: u32,

    /// How many times a stage may send work back to its predecessor for
    /// rework before the pipeline escalates / aborts.
    pub max_send_backs: u32,

    /// Hard wall-time cap on the whole pipeline.
    pub max_wall_time_minutes: u32,
}

/// Cost-tier policy. Outer key = tier name (`draft`, `hero`, …).
/// Inner key = artifact-kind (`i2v`, `still`, `music`, `tts`, …).
/// Value = provider identifier the backend dispatcher recognizes.
///
/// The schema does not validate which tier or artifact-kind names are
/// allowed — pipelines own that taxonomy. The runtime decides what to
/// do with unknown values (typically: report and fall back to default).
pub type TierPolicy = BTreeMap<String, BTreeMap<String, String>>;

/// Reference-video / reference-image input the pipeline conditions on.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ReferenceInput {
    /// Whether this pipeline supports being driven by a reference asset.
    pub supported: bool,

    /// Tools the pipeline uses to analyze the reference (e.g.
    /// `screenplay parse`, `query --on-beat`, `image_analysis`).
    #[serde(default)]
    pub analysis_tools: Vec<String>,
}
