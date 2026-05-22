//! WebSocket JSON-RPC 2.0 server for `wavelet agent serve`.
//!
//! Each connection has a writer task that drains an `mpsc::UnboundedReceiver<Message>`
//! and an inbound dispatch loop that hands each `RpcRequest` to a
//! handler. The agent loop runs on a `spawn_blocking` worker and
//! emits events into the sender, which the writer relays to the
//! client as `agent.event` notifications.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;

use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, Mutex};
use tokio_tungstenite::tungstenite::Message;

use super::events::Event;
use super::orchestrator::run_turn;
use super::protocol::{codes, RpcError, RpcNotification, RpcRequest, RpcResponse};
use super::session::Session;
use super::{AgentConfig, AgentLoop};

/// Shared server state — sessions + agent.
#[derive(Clone)]
pub struct ServerState {
    /// Sessions keyed by id.
    pub sessions: Arc<Mutex<HashMap<String, Session>>>,
    /// Tool registry + config.
    pub agent: AgentLoop,
}

impl ServerState {
    /// Build a fresh server state.
    pub fn new(config: AgentConfig) -> Self {
        Self {
            sessions: Arc::new(Mutex::new(HashMap::new())),
            agent: AgentLoop::new(config),
        }
    }
}

/// Bind and serve the WebSocket endpoint.
pub async fn serve(bind: &str, port: u16, state: ServerState) -> std::io::Result<()> {
    let addr: SocketAddr = format!("{bind}:{port}")
        .parse()
        .expect("invalid bind address");
    let listener = TcpListener::bind(addr).await?;
    eprintln!("[wavelet agent] listening on ws://{addr}");

    loop {
        let (stream, peer) = listener.accept().await?;
        let st = state.clone();
        tokio::spawn(async move {
            if let Err(e) = handle_connection(stream, peer, st).await {
                eprintln!("[wavelet agent] connection {peer} error: {e}");
            }
        });
    }
}

async fn handle_connection(
    stream: TcpStream,
    peer: SocketAddr,
    state: ServerState,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let ws = tokio_tungstenite::accept_async(stream).await?;
    eprintln!("[wavelet agent] {peer} connected");
    let (mut sink, mut rx) = ws.split();

    let (tx_out, mut rx_out) = mpsc::unbounded_channel::<Message>();
    // Writer task — drains the channel to the WebSocket.
    let writer = tokio::spawn(async move {
        while let Some(m) = rx_out.recv().await {
            if sink.send(m).await.is_err() {
                break;
            }
        }
        let _ = sink.close().await;
    });

    while let Some(frame) = rx.next().await {
        let msg = match frame {
            Ok(m) => m,
            Err(e) => {
                eprintln!("[wavelet agent] {peer} read error: {e}");
                break;
            }
        };
        let text = match msg {
            Message::Text(t) => t,
            Message::Close(_) => break,
            Message::Ping(p) => {
                let _ = tx_out.send(Message::Pong(p));
                continue;
            }
            _ => continue,
        };

        let req: RpcRequest = match serde_json::from_str(&text) {
            Ok(r) => r,
            Err(e) => {
                let err = RpcError::new(
                    Value::Null,
                    codes::PARSE_ERROR,
                    format!("parse error: {e}"),
                );
                let _ = tx_out.send(Message::Text(serde_json::to_string(&err)?));
                continue;
            }
        };

        let id = req.id.clone().unwrap_or(Value::Null);
        let tx_for_handler = tx_out.clone();
        let state_for_handler = state.clone();

        // Per-request dispatch — runs on the same task. Long ops
        // (agent.chat) defer their work to spawn_blocking themselves.
        let response = handle_rpc(req, &state_for_handler, tx_for_handler).await;

        let frame = match response {
            Ok(v) => serde_json::to_string(&RpcResponse::new(id, v))?,
            Err((code, msg)) => serde_json::to_string(&RpcError::new(id, code, msg))?,
        };
        let _ = tx_out.send(Message::Text(frame));
    }
    drop(tx_out);
    let _ = writer.await;
    eprintln!("[wavelet agent] {peer} disconnected");
    Ok(())
}

type RpcOutcome = Result<Value, (i32, String)>;

async fn handle_rpc(
    req: RpcRequest,
    state: &ServerState,
    tx: mpsc::UnboundedSender<Message>,
) -> RpcOutcome {
    match req.method.as_str() {
        "agent.list_tools" => Ok(json!({ "tools": state.agent.tools.schemas() })),
        "agent.session.new" => {
            let mut sessions = state.sessions.lock().await;
            let s = state.agent.new_session();
            let id = s.id.clone();
            sessions.insert(id.clone(), s);
            Ok(json!({ "session_id": id }))
        }
        "agent.session.history" => {
            let params = req.params.unwrap_or(json!({}));
            let sid = params
                .get("session_id")
                .and_then(|s| s.as_str())
                .ok_or((codes::INVALID_PARAMS, "missing `session_id`".to_string()))?;
            let sessions = state.sessions.lock().await;
            match sessions.get(sid) {
                Some(s) => Ok(json!({
                    "session_id": s.id,
                    "contents": s.contents,
                    "tool_ledger": s.tool_ledger,
                    "cost_usd": s.cost_usd,
                })),
                None => Err((codes::SESSION_NOT_FOUND, format!("no session `{sid}`"))),
            }
        }
        "agent.chat" => handle_chat(req, state, tx).await,
        other => Err((codes::METHOD_NOT_FOUND, format!("unknown method `{other}`"))),
    }
}

async fn handle_chat(
    req: RpcRequest,
    state: &ServerState,
    tx: mpsc::UnboundedSender<Message>,
) -> RpcOutcome {
    let params = req.params.unwrap_or(json!({}));
    let prompt = params
        .get("prompt")
        .and_then(|s| s.as_str())
        .ok_or((codes::INVALID_PARAMS, "missing `prompt`".to_string()))?
        .to_string();
    let session_id = params
        .get("session_id")
        .and_then(|s| s.as_str())
        .map(String::from);

    let session = {
        let mut sessions = state.sessions.lock().await;
        let id = match session_id {
            Some(id) => id,
            None => {
                let s = state.agent.new_session();
                let id = s.id.clone();
                sessions.insert(id.clone(), s);
                id
            }
        };
        sessions
            .get(&id)
            .cloned()
            .ok_or((codes::SESSION_NOT_FOUND, format!("no session `{id}`")))?
    };

    let tools = state.agent.tools.clone();
    let config = state.agent.config.clone();
    let sessions_for_save = state.sessions.clone();

    // Run the agent loop on a blocking worker.
    let (result, updated_session) = tokio::task::spawn_blocking(move || {
        let mut session = session;
        let emit_tx = tx.clone();
        let emit = move |event: Event| {
            let frame = serde_json::to_string(&RpcNotification::new(
                "agent.event",
                serde_json::to_value(&event).unwrap_or(json!({})),
            ))
            .unwrap_or_default();
            let _ = emit_tx.send(Message::Text(frame));
        };
        let result = run_turn(&mut session, &prompt, &tools, &config, &emit);
        (result, session)
    })
    .await
    .map_err(|e| (codes::INTERNAL_ERROR, format!("task join: {e}")))?;

    {
        let mut sessions = sessions_for_save.lock().await;
        sessions.insert(updated_session.id.clone(), updated_session);
    }

    match result {
        Ok(r) => Ok(json!({
            "session_id": r.session_id,
            "final_text": r.final_text,
            "output_files": r.output_files,
            "cost_usd": r.cost_usd,
            "wall_ms": r.wall_ms,
            "note": r.note,
        })),
        Err(e) => Err((codes::AGENT_ERROR, e.to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures_util::SinkExt;
    use tokio_tungstenite::connect_async;
    use tokio_tungstenite::tungstenite::Message;

    #[test]
    fn state_constructs() {
        let st = ServerState::new(AgentConfig::default());
        assert!(st.agent.tools.len() >= 25);
    }

    /// Smoke test — spin the server on a random port, send
    /// `agent.list_tools`, assert the registry comes back. Gated
    /// behind `#[ignore]` so unit-test runs stay hermetic (no need
    /// for an API key — list_tools doesn't call Gemini).
    #[test]
    #[ignore]
    fn list_tools_over_websocket() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async move {
            // Bind on :0 then read back the actual port via TcpListener.
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let port = listener.local_addr().unwrap().port();
            let state = ServerState::new(AgentConfig::default());

            tokio::spawn(async move {
                loop {
                    let (stream, peer) = listener.accept().await.unwrap();
                    let st = state.clone();
                    tokio::spawn(async move {
                        let _ = handle_connection(stream, peer, st).await;
                    });
                }
            });

            let (mut ws, _) = connect_async(&format!("ws://127.0.0.1:{port}"))
                .await
                .expect("ws connect");
            ws.send(Message::Text(
                r#"{"jsonrpc":"2.0","id":1,"method":"agent.list_tools"}"#.into(),
            ))
            .await
            .unwrap();
            let resp = ws.next().await.unwrap().unwrap();
            let text = match resp {
                Message::Text(t) => t,
                other => panic!("unexpected frame: {other:?}"),
            };
            let v: Value = serde_json::from_str(&text).unwrap();
            assert_eq!(v["jsonrpc"], "2.0");
            assert_eq!(v["id"], 1);
            let tools = v["result"]["tools"].as_array().unwrap();
            assert!(tools.len() >= 25);
        });
    }
}
