//! Typed phase events emitted by the agent loop.
//!
//! Both frontends consume these via a `Fn(Event)` callback. The chat
//! REPL pretty-prints them to stdout; the server marshals them into
//! `agent.event` JSON-RPC notifications.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Kind tag — keep this stable, it's part of the JSON-RPC wire format.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventKind {
    /// Model is planning the next step.
    Thinking,
    /// Tool dispatch starting.
    ToolCall,
    /// Tool dispatch completed (success or failure).
    ToolResult,
    /// Model is reviewing tool output.
    Review,
    /// Final user-visible reply.
    Final,
    /// Recoverable or terminal error.
    Error,
    /// Periodic cost / step bookkeeping. Useful for budget UIs.
    Progress,
}

/// One streamed event from the orchestrator.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Event {
    /// Categorical kind.
    pub kind: EventKind,
    /// Free-form phase / sub-status tag (e.g. `"planning"`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub phase: Option<String>,
    /// Tool name when relevant.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool: Option<String>,
    /// Tool args when `kind == ToolCall`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub args: Option<Value>,
    /// Short human-readable summary.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    /// Final text on `kind == Final`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// Running cost so far in USD.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cost_usd: Option<f32>,
    /// Step index inside the current turn.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub step: Option<u32>,
    /// Was the tool result ok? Only set on `ToolResult`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ok: Option<bool>,
}

impl Event {
    /// Builder: thinking phase.
    pub fn thinking(phase: impl Into<String>, step: u32, cost: f32) -> Self {
        Self {
            kind: EventKind::Thinking,
            phase: Some(phase.into()),
            tool: None,
            args: None,
            summary: None,
            text: None,
            cost_usd: Some(cost),
            step: Some(step),
            ok: None,
        }
    }

    /// Builder: tool call starting.
    pub fn tool_call(name: impl Into<String>, args: Value, step: u32, cost: f32) -> Self {
        Self {
            kind: EventKind::ToolCall,
            phase: None,
            tool: Some(name.into()),
            args: Some(args),
            summary: None,
            text: None,
            cost_usd: Some(cost),
            step: Some(step),
            ok: None,
        }
    }

    /// Builder: tool result.
    pub fn tool_result(
        name: impl Into<String>,
        ok: bool,
        summary: impl Into<String>,
        step: u32,
        cost: f32,
    ) -> Self {
        Self {
            kind: EventKind::ToolResult,
            phase: None,
            tool: Some(name.into()),
            args: None,
            summary: Some(summary.into()),
            text: None,
            cost_usd: Some(cost),
            step: Some(step),
            ok: Some(ok),
        }
    }

    /// Builder: final reply.
    pub fn final_text(text: impl Into<String>, cost: f32) -> Self {
        Self {
            kind: EventKind::Final,
            phase: None,
            tool: None,
            args: None,
            summary: None,
            text: Some(text.into()),
            cost_usd: Some(cost),
            step: None,
            ok: None,
        }
    }

    /// Builder: error event.
    pub fn error(detail: impl Into<String>) -> Self {
        Self {
            kind: EventKind::Error,
            phase: None,
            tool: None,
            args: None,
            summary: Some(detail.into()),
            text: None,
            cost_usd: None,
            step: None,
            ok: None,
        }
    }
}
