//! # `wavelet::agent::plan` — canonical Plan schema (wb-mqsb.1)
//!
//! A **Plan** is a directory of `plan/*.task.html` files: each task is an
//! HTML document with YAML front matter. Same shape pattern as the
//! `clipref` module (wb-n33n.1) — one struct, one parser, one writer,
//! round-trip-stable; unknown front-matter keys land in `extra`.
//!
//! The Plan is the on-disk source of truth for what the agent has been
//! asked to do, where each task stands, and how tasks fork/depend on
//! one another. Other children of the wb-mxrk epic build orchestrator,
//! runners, validators, etc. on top of this schema.

pub mod schema;
pub mod seed;
pub mod validator;
pub mod validators;

pub use schema::{Plan, PlanError, Task, TaskId, TaskStatus};
pub use seed::{seed_from_pipeline, SeedError};
pub use validator::{
    check_all, AssertionEq, ArtifactExists, CmdZeroExit, CostUnder, Validator, ValidatorCtx,
    ValidatorOutcome, ValidatorRegistry, ValidatorRegistryError,
};
pub use validators::{
    C2paVerifyPasses, CompVerifyPasses, QueryBeat, QueryPixels, QuerySceneGraph, QuerySnapshot,
    RubricPasses, UnitTestPasses,
};
