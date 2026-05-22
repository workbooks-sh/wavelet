//! Declarative pipeline definitions — YAML on disk, typed in Rust.
//!
//! Every pipeline is a stage list with required input/output artifacts,
//! tools each stage may call, success criteria, and orchestration
//! controls (budget, retry caps, wall-time). The runtime that actually
//! executes a pipeline lives behind `wavelet workflow run` (wb-oemp);
//! this module owns the on-disk schema + loader + registry only.
//!
//! ## Files
//!
//! - [`schema`] — typed schema (serde structs).
//! - [`loader`] — `load_from_path` / `load_from_str` with validation.
//! - [`registry`] — discovery of `*.yaml` pipelines under a search dir.
//!
//! See `packages/wavelet/pipeline_defs/*.yaml` for the canonical search
//! root. The schema mirrors OpenMontage's `pipeline_defs/cinematic.yaml`
//! shape so authors familiar with that project see the same fields.

pub mod loader;
pub mod registry;
pub mod schema;
pub mod workflow;

pub use loader::{load_from_path, load_from_str, LoadError};
pub use registry::{discover, default_search_dir, PipelineEntry};
pub use schema::{
    Orchestration, Pipeline, ReferenceInput, Stage, StageSuccessCriterion, TierPolicy,
};
pub use workflow::{compute_report, StageReport, StageStatus, WorkflowReport};
