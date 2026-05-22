//! `web.search` + `web.fetch` — backed by Gemini's grounding tool.
//!
//! `web.search` issues a small `generate_content` call with the
//! built-in `google_search` grounding tool enabled and surfaces the
//! grounded text + citations to the agent. `web.fetch` is a plain
//! ureq HTTP GET with a body size cap.

use serde_json::{json, Value};

use super::{arg_str as s, Tool, ToolRegistry, ToolResult};
use crate::edit::gemini::api_key_from_env;

const FETCH_CAP: usize = 256 * 1024;

pub fn register(r: &mut ToolRegistry) {
    r.register(WebSearch);
    r.register(WebFetch);
}

pub struct WebSearch;
impl Tool for WebSearch {
    fn name(&self) -> &str { "web.search" }
    fn description(&self) -> &str {
        "Search the web via Gemini-grounded search. Returns a text \
         summary plus the citations the model used."
    }
    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "required": ["query"],
            "properties": {
                "query": { "type": "string" }
            }
        })
    }
    fn dispatch(&self, args: &Value) -> ToolResult {
        let query = match s(args, "query") {
            Some(v) => v, None => return ToolResult::local_err(self.name(), "missing `query`"),
        };
        let api_key = match api_key_from_env() {
            Ok(k) => k,
            Err(_) => return ToolResult::local_err(self.name(), "GOOGLE_API_KEY not set"),
        };
        let url = format!(
            "https://generativelanguage.googleapis.com/v1beta/models/gemini-3.5-flash:generateContent?key={api_key}"
        );
        let body = json!({
            "contents": [ { "role": "user", "parts": [ { "text": query } ] } ],
            "tools": [ { "google_search": {} } ]
        });
        match ureq::post(&url)
            .set("Content-Type", "application/json")
            .send_string(&body.to_string())
        {
            Ok(resp) => {
                let v: Value = match resp.into_json() {
                    Ok(j) => j,
                    Err(e) => return ToolResult::local_err(self.name(), e.to_string()),
                };
                let text = extract_grounded_text(&v).unwrap_or_default();
                let citations = extract_citations(&v);
                ToolResult::local_ok(self.name(), json!({
                    "query": query,
                    "text": text,
                    "citations": citations,
                }))
            }
            Err(e) => ToolResult::local_err(self.name(), e.to_string()),
        }
    }
}

fn extract_grounded_text(v: &Value) -> Option<String> {
    let parts = v
        .get("candidates")?
        .as_array()?
        .first()?
        .get("content")?
        .get("parts")?
        .as_array()?;
    let mut out = String::new();
    for p in parts {
        if let Some(t) = p.get("text").and_then(|s| s.as_str()) {
            out.push_str(t);
        }
    }
    if out.is_empty() { None } else { Some(out) }
}

fn extract_citations(v: &Value) -> Vec<Value> {
    v.get("candidates")
        .and_then(|c| c.as_array())
        .and_then(|a| a.first())
        .and_then(|c| c.get("groundingMetadata"))
        .and_then(|gm| gm.get("groundingChunks"))
        .and_then(|gc| gc.as_array())
        .cloned()
        .unwrap_or_default()
}

pub struct WebFetch;
impl Tool for WebFetch {
    fn name(&self) -> &str { "web.fetch" }
    fn description(&self) -> &str {
        "Fetch a URL and return up to 256 KiB of body. Plain HTTP/HTTPS via ureq."
    }
    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "required": ["url"],
            "properties": {
                "url": { "type": "string" }
            }
        })
    }
    fn dispatch(&self, args: &Value) -> ToolResult {
        let url = match s(args, "url") {
            Some(v) => v, None => return ToolResult::local_err(self.name(), "missing `url`"),
        };
        match ureq::get(&url).call() {
            Ok(resp) => {
                let status = resp.status();
                let body = resp.into_string().unwrap_or_default();
                let truncated = body.len() > FETCH_CAP;
                let body = body.chars().take(FETCH_CAP).collect::<String>();
                ToolResult::local_ok(self.name(), json!({
                    "url": url,
                    "status": status,
                    "body": body,
                    "truncated": truncated,
                }))
            }
            Err(e) => ToolResult::local_err(self.name(), e.to_string()),
        }
    }
}
