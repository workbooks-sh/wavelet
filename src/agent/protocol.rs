//! JSON-RPC 2.0 message shapes for the `wavelet agent serve` WebSocket
//! API.
//!
//! Methods:
//! - `agent.chat`         params: `{ prompt, session_id?, attachments? }`
//! - `agent.list_tools`   params: `{}`
//! - `agent.session.new`  params: `{}`
//! - `agent.session.history` params: `{ session_id }`
//!
//! Notifications (server → client):
//! - `agent.event` params: `Event`

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// JSON-RPC 2.0 request envelope.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RpcRequest {
    /// Always `"2.0"`.
    pub jsonrpc: String,
    /// Request id. Strings, integers, or null.
    #[serde(default)]
    pub id: Option<Value>,
    /// Method name.
    pub method: String,
    /// Method params.
    #[serde(default)]
    pub params: Option<Value>,
}

/// JSON-RPC 2.0 successful response.
#[derive(Debug, Clone, Serialize)]
pub struct RpcResponse {
    /// Always `"2.0"`.
    pub jsonrpc: String,
    /// Request id (echoed).
    pub id: Value,
    /// Result payload.
    pub result: Value,
}

/// JSON-RPC 2.0 error response.
#[derive(Debug, Clone, Serialize)]
pub struct RpcError {
    /// Always `"2.0"`.
    pub jsonrpc: String,
    /// Request id (echoed; `null` for parse errors).
    pub id: Value,
    /// Error envelope.
    pub error: RpcErrorBody,
}

/// JSON-RPC error body.
#[derive(Debug, Clone, Serialize)]
pub struct RpcErrorBody {
    /// Error code (use the registered set + custom range).
    pub code: i32,
    /// Human-readable message.
    pub message: String,
    /// Optional structured detail.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

/// Notification — has no id.
#[derive(Debug, Clone, Serialize)]
pub struct RpcNotification {
    /// Always `"2.0"`.
    pub jsonrpc: String,
    /// Method name (e.g. `"agent.event"`).
    pub method: String,
    /// Notification params.
    pub params: Value,
}

impl RpcResponse {
    /// Construct a successful response.
    pub fn new(id: Value, result: Value) -> Self {
        Self {
            jsonrpc: "2.0".into(),
            id,
            result,
        }
    }
}

impl RpcError {
    /// Construct an error response.
    pub fn new(id: Value, code: i32, message: impl Into<String>) -> Self {
        Self {
            jsonrpc: "2.0".into(),
            id,
            error: RpcErrorBody {
                code,
                message: message.into(),
                data: None,
            },
        }
    }
}

impl RpcNotification {
    /// Construct a notification.
    pub fn new(method: impl Into<String>, params: Value) -> Self {
        Self {
            jsonrpc: "2.0".into(),
            method: method.into(),
            params,
        }
    }
}

/// Standard JSON-RPC error codes.
pub mod codes {
    /// JSON-RPC 2.0 parse-error.
    pub const PARSE_ERROR: i32 = -32700;
    /// JSON-RPC 2.0 method-not-found.
    pub const METHOD_NOT_FOUND: i32 = -32601;
    /// JSON-RPC 2.0 invalid-params.
    pub const INVALID_PARAMS: i32 = -32602;
    /// JSON-RPC 2.0 internal-error.
    pub const INTERNAL_ERROR: i32 = -32603;
    /// Application: agent loop failed.
    pub const AGENT_ERROR: i32 = -32000;
    /// Application: session not found.
    pub const SESSION_NOT_FOUND: i32 = -32002;
}
