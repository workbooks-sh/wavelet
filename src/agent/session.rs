//! Conversation state for the agent loop.
//!
//! A `Session` owns the Gemini `contents[]` array (the conversational
//! history) and a sidecar ledger of tool calls + cost. The
//! orchestrator mutates it in place; the server keeps sessions in a
//! `HashMap<Uuid, Mutex<Session>>` so successive `agent.chat` calls
//! continue the same thread.

use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::agent::plan::schema::Plan;

/// Shared mutable cell holding the optional plan handle. Tools clone
/// the outer `Arc` to share the slot with the `Session`. Plan-mode is
/// "off" when the inner `Option` is `None` (default).
pub type PlanCell = Arc<Mutex<Option<Arc<Mutex<Plan>>>>>;

/// Shared flag flipped by `plan.done`. The orchestrator reads it in
/// `PlanMode::On` to detect "model signaled completion".
pub type CompletionFlag = Arc<AtomicBool>;

/// Construct a fresh empty plan cell — plan mode Off.
pub fn empty_plan_cell() -> PlanCell {
    Arc::new(Mutex::new(None))
}

/// Construct a fresh unset completion flag.
pub fn empty_completion_flag() -> CompletionFlag {
    Arc::new(AtomicBool::new(false))
}

/// One entry in the tool-call ledger — kept separately from the
/// Gemini history so cost / observability code can inspect calls
/// without parsing the protocol-shaped `contents[]`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallEntry {
    /// Tool name.
    pub name: String,
    /// Args passed to the tool.
    pub args: Value,
    /// Compact result summary returned by the dispatcher.
    pub summary: String,
    /// Was the dispatch successful?
    pub ok: bool,
    /// USD cost attributed to this tool call (0.0 for local tools).
    pub cost_usd: f32,
}

/// Per-session conversation history + bookkeeping.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    /// Stable session id (UUID v4).
    pub id: String,
    /// Gemini-shaped contents history. We keep it as a raw JSON array
    /// so future protocol extensions (multimodal parts, etc.) don't
    /// require touching this module.
    pub contents: Vec<Value>,
    /// All tool calls dispatched in this session.
    pub tool_ledger: Vec<ToolCallEntry>,
    /// Files surfaced by any tool — useful for the final
    /// `agent.chat` result.
    pub output_files: Vec<PathBuf>,
    /// Running USD cost across the entire session.
    pub cost_usd: f32,
    /// Active system instruction, if any.
    pub system_instruction: Option<String>,
    /// Plan handle. Inner `Option` is `None` when plan mode is off
    /// (wb-mqsb.5 wires this on). The cell is shared with the
    /// `plan.*` tools so they can mutate the plan in place.
    #[serde(skip)]
    pub plan: PlanCell,
    /// Set by the `plan.done` sentinel tool. `run_turn` in `PlanMode::On`
    /// consults this together with `final_text` to decide condition (d).
    /// Shared with the tool ctx so the tool can flip it from a `&self`
    /// dispatch.
    #[serde(skip)]
    pub completion_signaled: CompletionFlag,
}

impl Session {
    /// Spin up an empty session with a fresh UUID.
    pub fn new() -> Self {
        Self::with_plan_cell(empty_plan_cell())
    }

    /// Build a session that shares both the given plan cell and
    /// completion flag with whoever else holds them (typically the
    /// `plan.*` tool registration).
    pub fn with_plan_handles(plan: PlanCell, completion_signaled: CompletionFlag) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            contents: Vec::new(),
            tool_ledger: Vec::new(),
            output_files: Vec::new(),
            cost_usd: 0.0,
            system_instruction: None,
            plan,
            completion_signaled,
        }
    }

    /// Restore a session from a given id (used when the client passes
    /// `session_id` back in an `agent.chat` call).
    pub fn with_id(id: String) -> Self {
        Self {
            id,
            ..Self::new()
        }
    }

    /// Create a session that shares the given plan cell. Mints a fresh
    /// completion flag. Most callers want `with_plan_handles` so the
    /// flag is shared with the `plan.*` tools.
    pub fn with_plan_cell(plan: PlanCell) -> Self {
        Self::with_plan_handles(plan, empty_completion_flag())
    }

    /// Set / replace the system instruction.
    pub fn set_system(&mut self, instruction: impl Into<String>) {
        self.system_instruction = Some(instruction.into());
    }

    /// Append a user-text turn.
    pub fn push_user(&mut self, text: &str) {
        self.contents.push(json!({
            "role": "user",
            "parts": [ { "text": text } ]
        }));
    }

    /// Append an assistant-text turn (no function calls).
    pub fn push_assistant_text(&mut self, text: &str) {
        self.contents.push(json!({
            "role": "model",
            "parts": [ { "text": text } ]
        }));
    }

    /// Append an assistant turn containing one or more function calls.
    /// The orchestrator emits one of these whenever Gemini returned
    /// `functionCall` parts; the matching `push_tool_response` calls
    /// follow in the same iteration.
    pub fn push_assistant_function_calls(&mut self, calls: &[(String, Value, Option<String>)]) {
        let parts: Vec<Value> = calls
            .iter()
            .map(|(name, args, sig)| {
                let mut part = json!({
                    "functionCall": { "name": name, "args": args }
                });
                if let Some(s) = sig {
                    part["thoughtSignature"] = Value::String(s.clone());
                }
                part
            })
            .collect();
        self.contents.push(json!({
            "role": "model",
            "parts": parts
        }));
    }

    /// Append a `functionResponse` part — Gemini expects the role to
    /// be `user` for tool responses (it treats responses as
    /// "environment feedback").
    pub fn push_tool_response(&mut self, name: &str, response: &Value) {
        self.contents.push(json!({
            "role": "user",
            "parts": [ {
                "functionResponse": {
                    "name": name,
                    "response": response
                }
            } ]
        }));
    }

    /// Record a tool call in the sidecar ledger.
    pub fn record_tool_call(&mut self, entry: ToolCallEntry) {
        self.cost_usd += entry.cost_usd;
        self.tool_ledger.push(entry);
    }

    /// Accumulate a Gemini call's cost into the running total. The
    /// USD figure comes from a coarse `(prompt, output)`
    /// token estimate.
    pub fn record_gemini_cost(&mut self, prompt_tokens: u32, output_tokens: u32, model: &str) {
        self.cost_usd += estimate_cost_usd(prompt_tokens, output_tokens, model);
    }
}

impl Default for Session {
    fn default() -> Self {
        Self::new()
    }
}

/// Coarse Gemini pricing. Flash is ~$0.075/1M prompt + $0.30/1M
/// output. The Pro tier is ~10x. Adjust when Google publishes the
/// 3.5 numbers.
fn estimate_cost_usd(prompt_tokens: u32, output_tokens: u32, model: &str) -> f32 {
    let (in_per_mil, out_per_mil) = if model.contains("pro") {
        (1.25_f32, 5.0_f32)
    } else {
        (0.075_f32, 0.30_f32)
    };
    (prompt_tokens as f32 / 1_000_000.0) * in_per_mil
        + (output_tokens as f32 / 1_000_000.0) * out_per_mil
}
