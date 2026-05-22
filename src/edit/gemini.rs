//! Minimal Gemini `generateContent` + Files-API client for the edit loop.
//!
//! Reuses `ureq` (already a wavelet dep — see `backends::google::client`)
//! so the edit module doesn't introduce a new HTTP stack or async
//! runtime.

use std::path::Path;
use std::time::{Duration, Instant};

use serde::Deserialize;
use serde_json::{json, Value};

use super::EditError;

const BASE: &str = "https://generativelanguage.googleapis.com";
const FILE_POLL_TIMEOUT_SECS: u64 = 180;
const FILE_POLL_INTERVAL_MS: u64 = 1500;

/// Read the API key from env. Errors with `EditError::NoKey` if
/// missing / blank.
pub fn api_key_from_env() -> Result<String, EditError> {
    let key = std::env::var("GOOGLE_API_KEY").map_err(|_| EditError::NoKey)?;
    if key.trim().is_empty() {
        return Err(EditError::NoKey);
    }
    Ok(key)
}

/// Call `models/<model>:generateContent` with a single text part.
/// Returns the raw `candidates[0].content.parts[0].text` string.
pub fn generate_text(model: &str, prompt: &str, api_key: &str) -> Result<String, EditError> {
    let url = format!("{BASE}/v1beta/models/{model}:generateContent?key={api_key}");
    let body = json!({
        "contents": [ { "parts": [ { "text": prompt } ] } ],
        "generationConfig": {
            "responseMimeType": "application/json",
            "temperature": 0.2
        }
    });
    post_json_extract_text(&url, &body)
}

/// Upload a local file via the resumable Files-API protocol, then
/// poll until `state == ACTIVE`. Returns the file URI (suitable for
/// embedding in a `file_data` part).
pub fn upload_file(path: &Path, mime: &str, api_key: &str) -> Result<String, EditError> {
    let bytes = std::fs::read(path).map_err(|e| {
        EditError::Transport(format!("read {} for upload: {e}", path.display()))
    })?;
    let display_name = path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("file");

    let start_url = format!("{BASE}/upload/v1beta/files?key={api_key}");
    let start_body = json!({ "file": { "display_name": display_name } });
    let start_resp = ureq::post(&start_url)
        .set("X-Goog-Upload-Protocol", "resumable")
        .set("X-Goog-Upload-Command", "start")
        .set("X-Goog-Upload-Header-Content-Length", &bytes.len().to_string())
        .set("X-Goog-Upload-Header-Content-Type", mime)
        .set("Content-Type", "application/json")
        .send_string(&start_body.to_string())
        .map_err(|e| EditError::Transport(format!("files upload start: {e}")))?;

    let upload_url = start_resp
        .header("x-goog-upload-url")
        .ok_or_else(|| EditError::Transport("files upload start: no x-goog-upload-url".into()))?
        .to_string();
    // Drain the response so the connection can be reused.
    let _ = start_resp.into_string();

    let finalize_resp = ureq::post(&upload_url)
        .set("Content-Length", &bytes.len().to_string())
        .set("X-Goog-Upload-Offset", "0")
        .set("X-Goog-Upload-Command", "upload, finalize")
        .send_bytes(&bytes)
        .map_err(|e| EditError::Transport(format!("files upload finalize: {e}")))?;
    let finalize_json: Value = finalize_resp
        .into_json()
        .map_err(|e| EditError::Transport(format!("files finalize decode: {e}")))?;
    let file_name = finalize_json["file"]["name"]
        .as_str()
        .ok_or_else(|| EditError::Transport("files finalize: no file.name".into()))?
        .to_string();
    let mut file_uri = finalize_json["file"]["uri"]
        .as_str()
        .ok_or_else(|| EditError::Transport("files finalize: no file.uri".into()))?
        .to_string();

    let deadline = Instant::now() + Duration::from_secs(FILE_POLL_TIMEOUT_SECS);
    loop {
        let poll_url = format!("{BASE}/v1beta/{file_name}?key={api_key}");
        let poll_resp = ureq::get(&poll_url)
            .call()
            .map_err(|e| EditError::Transport(format!("files poll: {e}")))?;
        let poll: Value = poll_resp
            .into_json()
            .map_err(|e| EditError::Transport(format!("files poll decode: {e}")))?;
        match poll["state"].as_str() {
            Some("ACTIVE") => {
                if let Some(u) = poll["uri"].as_str() {
                    file_uri = u.to_string();
                }
                return Ok(file_uri);
            }
            Some("FAILED") => {
                return Err(EditError::Transport(format!(
                    "files processing FAILED: {poll}"
                )))
            }
            _ => {}
        }
        if Instant::now() >= deadline {
            return Err(EditError::Transport(format!(
                "files did not reach ACTIVE within {FILE_POLL_TIMEOUT_SECS}s"
            )));
        }
        std::thread::sleep(Duration::from_millis(FILE_POLL_INTERVAL_MS));
    }
}

/// `generateContent` with both a video file_data part + a text part.
pub fn generate_with_video(
    model: &str,
    prompt: &str,
    file_uri: &str,
    mime: &str,
    api_key: &str,
) -> Result<String, EditError> {
    let url = format!("{BASE}/v1beta/models/{model}:generateContent?key={api_key}");
    let body = json!({
        "contents": [ {
            "parts": [
                { "file_data": { "mime_type": mime, "file_uri": file_uri } },
                { "text": prompt }
            ]
        } ],
        "generationConfig": {
            "responseMimeType": "application/json",
            "temperature": 0.2
        }
    });
    post_json_extract_text(&url, &body)
}

fn post_json_extract_text(url: &str, body: &Value) -> Result<String, EditError> {
    let resp = ureq::post(url)
        .set("Content-Type", "application/json")
        .send_string(&body.to_string());
    let resp = match resp {
        Ok(r) => r,
        Err(ureq::Error::Status(code, r)) => {
            let body = r.into_string().unwrap_or_default();
            return Err(EditError::Transport(format!(
                "HTTP {code}: {}",
                body.chars().take(400).collect::<String>()
            )));
        }
        Err(e) => return Err(EditError::Transport(e.to_string())),
    };
    let payload: GenerateContentResponse = resp
        .into_json()
        .map_err(|e| EditError::Transport(format!("decode generateContent: {e}")))?;
    payload
        .candidates
        .into_iter()
        .find_map(|c| c.content.parts.into_iter().find_map(|p| p.text))
        .ok_or_else(|| {
            EditError::Transport("generateContent: no candidates[0].content.parts[*].text".into())
        })
}

#[derive(Debug, Deserialize)]
struct GenerateContentResponse {
    #[serde(default)]
    candidates: Vec<Candidate>,
}

#[derive(Debug, Deserialize)]
struct Candidate {
    content: Content,
}

#[derive(Debug, Deserialize)]
struct Content {
    #[serde(default)]
    parts: Vec<Part>,
}

#[derive(Debug, Deserialize)]
struct Part {
    #[serde(default)]
    text: Option<String>,
}

// ---------------------------------------------------------------------------
// Function-calling round-trip used by `wavelet agent`.
//
// The agent module needs full conversation state plus declared tools.
// Rather than fork the whole HTTP client, we expose `generate_with_tools`
// — a thin wrapper that posts a pre-built `contents` array (Gemini's
// turn history) alongside a `tools` declarations array, and surfaces
// whichever the model returns: a text reply, one-or-more
// `functionCall` parts, or both.
// ---------------------------------------------------------------------------

/// One chunk of a model response — either user-visible text or a
/// function call the orchestrator must dispatch and reply to.
#[derive(Debug, Clone)]
pub enum GeminiPart {
    /// Final or interim text response from the model.
    Text(String),
    /// The model asked the runtime to invoke a tool.
    FunctionCall {
        /// Tool name (registry key).
        name: String,
        /// JSON arguments object (validated by the tool's schema).
        args: serde_json::Value,
        /// Opaque thought signature Gemini emits in 3.5 thinking mode.
        /// Must be echoed verbatim when the agent appends the model's
        /// `functionCall` part back into `contents` for the next round
        /// trip; without it Gemini returns HTTP 400.
        thought_signature: Option<String>,
    },
}

/// A single parsed response from `generateContent`.
#[derive(Debug, Clone, Default)]
pub struct GeminiResponse {
    /// Ordered parts the model returned.
    pub parts: Vec<GeminiPart>,
    /// Coarse token usage if the API surfaced it. Used by the
    /// agent's cost tracker. `(prompt_tokens, output_tokens)`.
    pub usage: Option<(u32, u32)>,
}

impl GeminiResponse {
    /// Convenience: concatenated text across all `Text` parts.
    pub fn text(&self) -> Option<String> {
        let mut s = String::new();
        for p in &self.parts {
            if let GeminiPart::Text(t) = p {
                s.push_str(t);
            }
        }
        if s.is_empty() { None } else { Some(s) }
    }

    /// Convenience: every function call the model emitted in this turn.
    pub fn function_calls(&self) -> Vec<(&str, &serde_json::Value, Option<&str>)> {
        self.parts
            .iter()
            .filter_map(|p| match p {
                GeminiPart::FunctionCall { name, args, thought_signature } => Some((
                    name.as_str(),
                    args,
                    thought_signature.as_deref(),
                )),
                _ => None,
            })
            .collect()
    }
}

/// POST `generateContent` with a caller-built `contents` history,
/// optional `tools.function_declarations`, optional `system_instruction`,
/// and a thinkingLevel hint. Returns the parsed response.
pub fn generate_with_tools(
    model: &str,
    contents: &Value,
    tools: Option<&Value>,
    system_instruction: Option<&str>,
    thinking_level: &str,
    api_key: &str,
) -> Result<GeminiResponse, EditError> {
    let url = format!("{BASE}/v1beta/models/{model}:generateContent?key={api_key}");
    let mut body = json!({
        "contents": contents,
        "generationConfig": {
            "temperature": 0.2,
            "thinkingConfig": { "thinkingBudget": thinking_budget_for(thinking_level) }
        }
    });
    if let Some(t) = tools {
        body["tools"] = t.clone();
    }
    if let Some(si) = system_instruction {
        body["systemInstruction"] = json!({ "parts": [ { "text": si } ] });
    }

    let body_str = body.to_string();
    let mut last_err: Option<String> = None;
    let mut resp = None;
    for attempt in 0..4 {
        if attempt > 0 {
            std::thread::sleep(std::time::Duration::from_secs(1u64 << attempt));
        }
        match ureq::post(&url)
            .set("Content-Type", "application/json")
            .send_string(&body_str)
        {
            Ok(r) => { resp = Some(r); break; }
            Err(ureq::Error::Status(code, r)) => {
                let body = r.into_string().unwrap_or_default();
                let snippet = body.chars().take(600).collect::<String>();
                last_err = Some(format!("HTTP {code}: {snippet}"));
                if !matches!(code, 429 | 500 | 502 | 503 | 504) { break; }
            }
            Err(e) => {
                last_err = Some(e.to_string());
                break;
            }
        }
    }
    let resp = match resp {
        Some(r) => r,
        None => return Err(EditError::Transport(last_err.unwrap_or_else(|| "unknown".into()))),
    };

    let payload: Value = resp
        .into_json()
        .map_err(|e| EditError::Transport(format!("decode generateContent: {e}")))?;

    let mut out = GeminiResponse::default();
    if let Some(cands) = payload.get("candidates").and_then(|v| v.as_array()) {
        if let Some(parts) = cands
            .first()
            .and_then(|c| c.get("content"))
            .and_then(|c| c.get("parts"))
            .and_then(|p| p.as_array())
        {
            for p in parts {
                if let Some(text) = p.get("text").and_then(|t| t.as_str()) {
                    out.parts.push(GeminiPart::Text(text.to_string()));
                } else if let Some(fc) = p.get("functionCall") {
                    let name = fc
                        .get("name")
                        .and_then(|n| n.as_str())
                        .unwrap_or("")
                        .to_string();
                    let args = fc.get("args").cloned().unwrap_or_else(|| json!({}));
                    let thought_signature = p
                        .get("thoughtSignature")
                        .and_then(|s| s.as_str())
                        .map(|s| s.to_string());
                    out.parts.push(GeminiPart::FunctionCall {
                        name,
                        args,
                        thought_signature,
                    });
                }
            }
        }
    }

    if let Some(usage) = payload.get("usageMetadata") {
        let pt = usage
            .get("promptTokenCount")
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as u32;
        let ot = usage
            .get("candidatesTokenCount")
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as u32;
        if pt != 0 || ot != 0 {
            out.usage = Some((pt, ot));
        }
    }

    Ok(out)
}

fn thinking_budget_for(level: &str) -> i32 {
    match level {
        "low" => 1024,
        "medium" => 4096,
        "high" => 16384,
        _ => 4096,
    }
}

/// Probe `v1beta/models` for any model whose name starts with
/// `models/gemini-omni`. Returns the first matching model id (e.g.
/// `models/gemini-omni-flash`) so the executor can swap it in once
/// the model ships, or `None` if no match is found.
///
/// Cheap fallback for the v1 "OmniEdit unavailable" path — lets us
/// surface the live model slug in the error message.
pub fn probe_omni_model(api_key: &str) -> Option<String> {
    let url = format!("{BASE}/v1beta/models?key={api_key}");
    let resp = ureq::get(&url).call().ok()?;
    let body: Value = resp.into_json().ok()?;
    let arr = body.get("models")?.as_array()?;
    for m in arr {
        if let Some(name) = m.get("name").and_then(|n| n.as_str()) {
            if name.starts_with("models/gemini-omni") {
                return Some(name.to_string());
            }
        }
    }
    None
}
