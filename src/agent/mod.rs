//! `wavelet agent` — Gemini-3.5-native agent loop.
//!
//! Two frontends share one orchestrator: an interactive REPL
//! (`wavelet agent chat`) and a JSON-RPC 2.0 WebSocket server
//! (`wavelet agent serve`). The tool surface is the full `wavelet`
//! CLI plus `fs.*` and `web.*` helpers.
//!
//! Architecture overview:
//!
//! ```text
//! ┌─────────────┐   ┌─────────────────┐   ┌───────────────┐
//! │ chat (REPL) │──▶│  AgentLoop /    │──▶│ Tool registry │
//! │ serve (WS)  │   │  orchestrator   │   │ (subprocess)  │
//! └─────────────┘   └─────────────────┘   └───────────────┘
//!                          │  ▲
//!                          ▼  │
//!                    Gemini generateContent
//!                       (function-call)
//! ```

#![allow(missing_docs)]

use std::path::PathBuf;
use std::sync::Arc;

use thiserror::Error;

pub mod chat;
pub mod events;
pub mod orchestrator;
pub mod plan;
pub mod prompt_builder;
pub mod protocol;
pub mod server;
pub mod session;
pub mod tools;

pub use events::{Event, EventKind};
pub use orchestrator::{run_turn, TurnOutcome};
pub use session::{PlanCell, Session};
pub use tools::{Tool, ToolRegistry, ToolResult};

/// Plan substrate engagement level. `Off` keeps the legacy step-bounded
/// loop; `Shadow` loads + mutates the plan and emits events but still
/// terminates on `max_steps`; `On` lets plan terminality + wall-clock +
/// budget gate the loop (and `max_steps` is ignored).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlanMode {
    Off,
    Shadow,
    On,
}

impl Default for PlanMode {
    fn default() -> Self {
        PlanMode::Off
    }
}

/// Configuration knobs every agent surface shares.
#[derive(Debug, Clone)]
pub struct AgentConfig {
    /// Override the default `gemini-3.5-flash` slug (env `WAVELET_AGENT_MODEL`).
    pub model: String,
    /// Slug used when a tool dispatches `Role::Deep`. Env
    /// `WAVELET_AGENT_DEEP_MODEL`.
    pub deep_model: String,
    /// Max USD spent across all Gemini + tool calls in one turn.
    pub max_cost_usd: f32,
    /// Cap on Gemini round-trips per turn — guards against infinite
    /// function-call loops on a buggy model. Consulted in `PlanMode::Off`
    /// and `Shadow`; ignored in `On` (a `MAX_RUNAWAY` sanity cap inside
    /// `run_turn` keeps a broken model from looping forever).
    pub max_steps: u32,
    /// Optional override for the system prompt.
    pub system_prompt: Option<String>,
    /// Absolute path to the `wavelet` binary used for tool dispatch.
    /// Falls back to `$WAVELET_BIN` → `current_exe()` → `which wavelet`.
    pub gamut_bin: Option<PathBuf>,
    /// How the plan substrate engages this turn (wb-mqsb.5).
    pub plan_mode: PlanMode,
    /// Root the plan loads from / writes back to. `None` → `current_dir`.
    /// Plan files live at `<plan_workdir>/plan/*.task.html`.
    pub plan_workdir: Option<PathBuf>,
    /// Wall-clock cap (seconds) — only consulted in `PlanMode::On`.
    pub max_wall_seconds: u64,
}

impl Default for AgentConfig {
    fn default() -> Self {
        Self {
            model: std::env::var("WAVELET_AGENT_MODEL")
                .unwrap_or_else(|_| "gemini-3.5-flash".to_string()),
            deep_model: std::env::var("WAVELET_AGENT_DEEP_MODEL")
                .unwrap_or_else(|_| "gemini-3.1-pro-preview".to_string()),
            max_cost_usd: 1.00,
            max_steps: 24,
            system_prompt: None,
            gamut_bin: None,
            plan_mode: PlanMode::Off,
            plan_workdir: None,
            max_wall_seconds: 1800,
        }
    }
}

/// Final result of `AgentLoop::run_turn`.
#[derive(Debug, Clone)]
pub struct AgentResult {
    /// Final assistant text response (None if the loop bailed before
    /// producing one).
    pub final_text: Option<String>,
    /// Output files surfaced by any tool call.
    pub output_files: Vec<PathBuf>,
    /// Running USD cost accumulated across this turn.
    pub cost_usd: f32,
    /// Wall-clock duration.
    pub wall_ms: u128,
    /// Session identifier — supplied so a follow-up `agent.chat` can
    /// continue the same conversation.
    pub session_id: String,
    /// Optional note for budget/step exhaustion.
    pub note: Option<String>,
}

/// All errors the agent loop can surface.
#[derive(Debug, Error)]
pub enum AgentError {
    /// `GOOGLE_API_KEY` is missing or blank.
    #[error("GOOGLE_API_KEY not set. `wavelet agent` requires Gemini access.")]
    NoKey,
    /// The orchestrator hit `max_steps` without a terminal text reply.
    #[error("agent loop exceeded max_steps={0}")]
    LoopOverflow(u32),
    /// Budget cap blew before the loop terminated.
    #[error("budget exhausted at ${0:.4} (cap ${1:.4})")]
    BudgetExhausted(f32, f32),
    /// Tool dispatch failed.
    #[error("tool `{name}` failed: {detail}")]
    ToolDispatch {
        /// Tool name.
        name: String,
        /// Detail from the tool wrapper.
        detail: String,
    },
    /// Unknown tool name surfaced by the model.
    #[error("unknown tool requested by model: `{0}`")]
    UnknownTool(String),
    /// Gemini transport error.
    #[error("gemini: {0}")]
    Gemini(String),
    /// JSON-RPC protocol violation.
    #[error("rpc: {0}")]
    Rpc(String),
}

impl From<crate::edit::EditError> for AgentError {
    fn from(value: crate::edit::EditError) -> Self {
        match value {
            crate::edit::EditError::NoKey => AgentError::NoKey,
            other => AgentError::Gemini(other.to_string()),
        }
    }
}

/// Public agent handle. Owns the tool registry + config and lets
/// callers run a turn on a session it stores.
#[derive(Clone)]
pub struct AgentLoop {
    /// Tool registry — declarations + dispatch handlers.
    pub tools: Arc<ToolRegistry>,
    /// Static configuration.
    pub config: Arc<AgentConfig>,
    /// Shared plan slot — both the `plan.*` tools and any session this
    /// loop spawns clone this cell, so populating the inner Option in
    /// `run_turn` makes the plan visible to every tool dispatch.
    pub plan_cell: PlanCell,
    /// Shared completion flag for `plan.done` ↔ orchestrator handshake.
    pub completion_signaled: session::CompletionFlag,
}

impl AgentLoop {
    /// Build a new agent with the default tool registry and the given
    /// configuration.
    pub fn new(config: AgentConfig) -> Self {
        let plan_cell = session::empty_plan_cell();
        let completion_signaled = session::empty_completion_flag();
        let reg = tools::build_registry(
            plan_cell.clone(),
            completion_signaled.clone(),
            Arc::new(plan::validator::ValidatorRegistry::with_builtins()),
        );
        Self {
            tools: Arc::new(reg),
            config: Arc::new(config),
            plan_cell,
            completion_signaled,
        }
    }

    /// Spin up a session that shares this loop's plan cell and
    /// completion flag. Use this instead of `Session::new()` so plan
    /// tools and the orchestrator observe the same shared slots.
    pub fn new_session(&self) -> Session {
        Session::with_plan_handles(self.plan_cell.clone(), self.completion_signaled.clone())
    }
}
