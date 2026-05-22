//! Director pre-pipeline glue — the surface the wavelet-director skill writes
//! against before the screenplay / storyboard / velocity stages run.
//!
//! ## Module map
//!
//! - [`brief`] — the 9-line ad creative brief parser
//!   (`PRODUCT / AUDIENCE / INSIGHT / PROMISE / PROOF / TONE / MUSIC / CALL /
//!   RUNTIME`). Validated via `wavelet brief check`.
//! - [`creative_director`] — LLM-as-creative-director orchestrator. Takes a
//!   brief + shot skeletons and fills the seven L-Storyboard
//!   `ShotAttributes` slots per shot. Parses + validates the LLM JSON
//!   response, retries once on empty slots, returns
//!   `Vec<(shot_id, ShotAttributes)>`.
//! - [`prompts`] — system + user prompt templates for the LLM director. The
//!   system prompt is byte-stable; a snapshot test locks it.
//! - [`backend`] — concrete [`creative_director::LlmBackend`] impl over
//!   `fal-ai/any-llm` (Gemini 2.5 Pro default, Claude Opus 4.7 fallback).
//!
//! Per the May-2026 ComfyUI commercial-workflow audit, 9 of 15 surveyed
//! pipelines use an LLM in the creative-director role; the result is
//! consistently more specific, more consistent across shots in a spot, and
//! more responsive to creative direction than deterministic template
//! assembly.
//!
//! See [`vendor/workbooks/skills/wavelet-director/SKILL.md`] Steps 1, 3.25,
//! and 3.26 for the agent-facing workflow.

pub mod backend;
pub mod brief;
pub mod creative_director;
pub mod grader;
pub mod prompts;

pub use backend::{
    resolve_model_flag, FalAnyLlmBackend, ANY_LLM_PATH, MODEL_CLAUDE_OPUS,
    MODEL_GEMINI_PRO,
};
pub use creative_director::{
    synthesize_shot_attributes, DirectorRequest, LlmBackend, ShotSkeleton,
};
pub use grader::{mutate_prompt, GraderError, GraderRequest, GraderResult};
pub use prompts::{build_retry_prompt, build_user_prompt, SYSTEM_PROMPT};
