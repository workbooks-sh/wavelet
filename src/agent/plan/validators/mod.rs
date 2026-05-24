//! Heavy validator impls (wb-mqsb.3) that build on the
//! `Validator`/`ValidatorRegistry` substrate from wb-mqsb.2.
//!
//! Discipline (per `vendor/workbooks/packages/workbench/EVAL_PRINCIPLES.md`):
//! objective gates run before vision rubrics. The dispatcher in
//! `super::validator::check_all` runs criteria in declared order and
//! the caller short-circuits on first failure — order tasks so that
//! `artifact_exists` / `query.*` / `comp_verify_passes` /
//! `c2pa_verify_passes` / `unit_test_passes` all run before
//! `rubric_passes`. `rubric_passes` is the only validator that charges
//! the session budget.

pub mod query;
pub mod render;
pub mod rubric;
pub mod shader;
pub mod trace;
pub mod unit;
mod util;

pub use query::{QueryBeat, QueryPixels, QuerySceneGraph, QuerySnapshot};
pub use render::{C2paVerifyPasses, CompVerifyPasses};
pub use rubric::RubricPasses;
pub use shader::QueryShader;
pub use trace::{AdalignResearchDone, BrandworkResearchDone, ScreenplayDurationFits, WaveletLintPasses};
pub use unit::UnitTestPasses;

use super::validator::{ValidatorRegistry, ValidatorRegistryError};

/// Register every heavy validator on a registry. Idempotent only if
/// the registry didn't already contain them — caller controls.
pub fn register_all(reg: &mut ValidatorRegistry) -> Result<(), ValidatorRegistryError> {
    reg.register(Box::new(QuerySceneGraph))?;
    reg.register(Box::new(QueryPixels))?;
    reg.register(Box::new(QuerySnapshot))?;
    reg.register(Box::new(QueryBeat))?;
    reg.register(Box::new(QueryShader))?;
    reg.register(Box::new(CompVerifyPasses))?;
    reg.register(Box::new(C2paVerifyPasses))?;
    reg.register(Box::new(UnitTestPasses))?;
    reg.register(Box::new(RubricPasses))?;
    reg.register(Box::new(BrandworkResearchDone))?;
    // Transitional alias — `adalign_research_done` is still a valid criterion
    // kind in pipeline YAML files during the migration window. Register a
    // separate instance under the old name so existing pipeline definitions
    // keep working without edits.
    reg.register(Box::new(crate::agent::plan::validators::trace::AdalignResearchDoneAlias))?;
    reg.register(Box::new(WaveletLintPasses))?;
    reg.register(Box::new(ScreenplayDurationFits))?;
    Ok(())
}

#[cfg(test)]
mod tests;
