//! Tool trait, registry, and shared dispatch helpers.
//!
//! Every wavelet verb is wrapped as a `Tool` — JSON-schema in,
//! `ToolResult` out. The default impl spawns a subprocess of the
//! `wavelet` binary, captures stdout/stderr, and returns whichever
//! representation Gemini can parse most directly.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::{Command, Output};

use serde_json::{json, Value};

pub mod brand_tools;
pub mod c2pa_tools;
pub mod fs_tools;
pub mod image_tools;
pub mod music_tools;
pub mod plan_tools;
pub mod query_shader_tool;
pub mod query_tools;
pub mod render_tools;
pub mod shot_tools;
pub mod web_tools;
pub mod workflow_tools;

/// Outcome of a tool dispatch.
#[derive(Debug, Clone)]
pub struct ToolResult {
    /// Was the dispatch successful?
    pub ok: bool,
    /// JSON value Gemini receives as `functionResponse.response`. For
    /// non-JSON output this wraps `{ stdout, stderr, exit_code }`.
    pub response: Value,
    /// One-line summary the event stream surfaces.
    pub summary: String,
    /// Files the tool produced (forwarded to `AgentResult.output_files`).
    pub output_files: Vec<PathBuf>,
    /// Cost attributed to this call. Local tools = 0; backend-calling
    /// verbs surface their estimate via the parsed JSON when available.
    pub cost_usd: f32,
}

impl ToolResult {
    /// Build a ToolResult from a completed subprocess.
    ///
    /// If stdout parses as JSON we surface that object directly; if
    /// the JSON has an `output_files` array (wavelet convention) we
    /// lift it. Non-JSON output is wrapped into a `{stdout, stderr,
    /// exit_code}` envelope.
    pub fn from_subprocess(name: &str, out: Output) -> Self {
        let exit_code = out.status.code().unwrap_or(-1);
        let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
        let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
        let ok = out.status.success();

        let (response, output_files, cost_usd) =
            if let Some(parsed) = try_parse_json(&stdout) {
                let files = collect_files(&parsed);
                let cost = parsed
                    .get("cost_usd")
                    .and_then(|v| v.as_f64())
                    .unwrap_or(0.0) as f32;
                (parsed, files, cost)
            } else {
                (
                    json!({
                        "stdout": stdout.chars().take(8000).collect::<String>(),
                        "stderr": stderr.chars().take(2000).collect::<String>(),
                        "exit_code": exit_code,
                    }),
                    Vec::new(),
                    0.0,
                )
            };

        let summary = if ok {
            format!(
                "{name}: exit=0, {} bytes stdout, {} output file(s)",
                stdout.len(),
                output_files.len()
            )
        } else {
            format!(
                "{name}: exit={exit_code}, stderr={}",
                stderr.chars().take(160).collect::<String>()
            )
        };

        ToolResult {
            ok,
            response,
            summary,
            output_files,
            cost_usd,
        }
    }

    /// Build a ToolResult for a purely-local tool (fs / web helpers).
    pub fn local_ok(name: &str, response: Value) -> Self {
        let summary = format!("{name}: ok");
        ToolResult {
            ok: true,
            response,
            summary,
            output_files: Vec::new(),
            cost_usd: 0.0,
        }
    }

    /// Build a ToolResult for a local-tool error.
    pub fn local_err(name: &str, detail: impl Into<String>) -> Self {
        let detail = detail.into();
        let summary = format!("{name}: {detail}");
        ToolResult {
            ok: false,
            response: json!({ "error": detail }),
            summary,
            output_files: Vec::new(),
            cost_usd: 0.0,
        }
    }
}

fn try_parse_json(s: &str) -> Option<Value> {
    let trimmed = s.trim();
    if trimmed.is_empty() {
        return None;
    }
    // Only consider it JSON if it starts with `{` or `[` to avoid
    // mistaking plain-text logs for JSON.
    let first = trimmed.as_bytes()[0];
    if first != b'{' && first != b'[' {
        return None;
    }
    serde_json::from_str(trimmed).ok()
}

fn collect_files(v: &Value) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Some(arr) = v.get("output_files").and_then(|a| a.as_array()) {
        for f in arr {
            if let Some(s) = f.as_str() {
                out.push(PathBuf::from(s));
            }
        }
    }
    if let Some(s) = v.get("output").and_then(|s| s.as_str()) {
        out.push(PathBuf::from(s));
    }
    if let Some(s) = v.get("out").and_then(|s| s.as_str()) {
        out.push(PathBuf::from(s));
    }
    out
}

/// The Tool trait every wrapper implements.
pub trait Tool: Send + Sync {
    /// Stable name (registry key + Gemini function-call name).
    fn name(&self) -> &str;
    /// Short description Gemini sees when deciding to call it.
    fn description(&self) -> &str;
    /// JSON-schema for the args object Gemini must produce.
    fn parameters_schema(&self) -> Value;
    /// Dispatch the call.
    fn dispatch(&self, args: &Value) -> ToolResult;
}

/// Resolve the absolute path to the wavelet binary the agent calls.
/// Order: `$WAVELET_BIN` → `current_exe()` → `wavelet` on PATH.
pub fn resolve_gamut_bin() -> PathBuf {
    if let Ok(p) = std::env::var("WAVELET_BIN") {
        return PathBuf::from(p);
    }
    if let Ok(exe) = std::env::current_exe() {
        return exe;
    }
    PathBuf::from("wavelet")
}

/// Spawn the wavelet binary with the given args and capture output.
pub fn spawn_gamut(args: &[String]) -> std::io::Result<Output> {
    Command::new(resolve_gamut_bin()).args(args).output()
}

/// Extract a string field from a JSON args object.
pub fn arg_str(args: &Value, key: &str) -> Option<String> {
    args.get(key).and_then(|v| v.as_str()).map(|s| s.to_string())
}

/// Append the `out` path to `result.output_files` when the dispatch
/// succeeded and the file actually exists on disk. Gen subcommands
/// don't emit structured JSON, so `from_subprocess` can't lift their
/// outputs — the dispatcher already knows the path the caller asked
/// for, so we attach it here as a post-hoc correction.
pub fn attach_out_file(result: &mut ToolResult, out: &str) {
    if result.ok && std::fs::metadata(out).is_ok() {
        result.output_files.push(PathBuf::from(out));
    }
}

/// Translate one JSON arg into a `--flag value` (or bare `--flag` for
/// booleans) and push onto the command vector. No-op when absent.
pub fn push_flag(cmd: &mut Vec<String>, args: &Value, key: &str, flag: &str) {
    if let Some(v) = args.get(key) {
        match v {
            Value::String(s) => {
                cmd.push(flag.to_string());
                cmd.push(s.clone());
            }
            Value::Number(n) => {
                cmd.push(flag.to_string());
                cmd.push(n.to_string());
            }
            Value::Bool(b) => {
                if *b {
                    cmd.push(flag.to_string());
                }
            }
            _ => {}
        }
    }
}

/// Registry of tools — held by `AgentLoop`. Maintains insertion order
/// via `BTreeMap` on name; consumers don't depend on iteration order.
pub struct ToolRegistry {
    tools: BTreeMap<String, Box<dyn Tool>>,
}

impl ToolRegistry {
    /// Empty registry.
    pub fn new() -> Self {
        Self {
            tools: BTreeMap::new(),
        }
    }

    /// Insert a tool.
    pub fn register<T: Tool + 'static>(&mut self, tool: T) {
        self.tools.insert(tool.name().to_string(), Box::new(tool));
    }

    /// Look a tool up by name.
    pub fn get(&self, name: &str) -> Option<&dyn Tool> {
        self.tools.get(name).map(|b| b.as_ref())
    }

    /// All registered tool names.
    pub fn names(&self) -> Vec<&str> {
        self.tools.keys().map(|s| s.as_str()).collect()
    }

    /// Number of registered tools.
    pub fn len(&self) -> usize {
        self.tools.len()
    }

    /// Is the registry empty?
    pub fn is_empty(&self) -> bool {
        self.tools.is_empty()
    }

    /// Build the Gemini `tools.function_declarations` JSON for the
    /// whole registry.
    pub fn function_declarations(&self) -> Value {
        let decls: Vec<Value> = self
            .tools
            .values()
            .map(|t| {
                json!({
                    "name": t.name(),
                    "description": t.description(),
                    "parameters": t.parameters_schema(),
                })
            })
            .collect();
        json!([ { "function_declarations": decls } ])
    }

    /// Schemas only — useful for `agent.list_tools` JSON-RPC method.
    pub fn schemas(&self) -> Vec<Value> {
        self.tools
            .values()
            .map(|t| {
                json!({
                    "name": t.name(),
                    "description": t.description(),
                    "parameters": t.parameters_schema(),
                })
            })
            .collect()
    }
}

impl Default for ToolRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Build a fresh registry with every shipped tool. Used by both
/// `AgentLoop::new` (with real plan handles) and `default_registry`
/// (with empty handles for tests + introspection). Single source of
/// truth for the tool list — add new tool modules here, nowhere else.
pub fn build_registry(
    plan_cell: crate::agent::session::PlanCell,
    completion: crate::agent::session::CompletionFlag,
    validators: std::sync::Arc<crate::agent::plan::validator::ValidatorRegistry>,
) -> ToolRegistry {
    let mut r = ToolRegistry::new();
    shot_tools::register(&mut r);
    image_tools::register(&mut r);
    music_tools::register(&mut r);
    render_tools::register(&mut r);
    workflow_tools::register(&mut r);
    query_tools::register(&mut r);
    query_shader_tool::register(&mut r);
    c2pa_tools::register(&mut r);
    fs_tools::register(&mut r);
    web_tools::register(&mut r);
    brand_tools::register(&mut r);
    plan_tools::register_with_plan_and_completion(&mut r, plan_cell, validators, completion);
    r
}

/// Construct the default registry — every shipped tool, with empty
/// plan handles. Used by tests + `wavelet agent tools` introspection.
/// Production callers use `AgentLoop::new` which threads in real
/// plan handles.
pub fn default_registry() -> ToolRegistry {
    build_registry(
        crate::agent::session::empty_plan_cell(),
        crate::agent::session::empty_completion_flag(),
        std::sync::Arc::new(crate::agent::plan::validator::ValidatorRegistry::with_builtins()),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_registry_has_all_clusters() {
        let r = default_registry();
        // Spot-check coverage: at least one per cluster.
        assert!(r.get("wavelet.shot.edit").is_some());
        assert!(r.get("wavelet.image.composite").is_some());
        assert!(r.get("wavelet.music.gen").is_some());
        assert!(r.get("wavelet.render").is_some());
        assert!(r.get("workflow.run").is_some());
        assert!(r.get("query.snapshot").is_some());
        assert!(r.get("query.shader").is_some());
        assert!(r.get("c2pa.sign").is_some());
        assert!(r.get("fs.read").is_some());
        assert!(r.get("web.search").is_some());
        for t in [
            "plan.add", "plan.update", "plan.complete", "plan.reopen",
            "plan.abandon", "plan.fork", "plan.validate", "plan.show",
            "plan.seed", "plan.done",
        ] {
            assert!(r.get(t).is_some(), "missing {t}");
        }
        let brand_tools = ["brand.brief", "brand.fetch", "brand.catalog", "brand.product", "brand.ads"];
        for t in brand_tools {
            assert!(r.get(t).is_some(), "missing {t}");
        }
        let brand_count = r.names().iter().filter(|n| n.starts_with("brand.")).count();
        assert_eq!(brand_count, 5, "expected 5 brand.* tools, got {brand_count}");
    }

    #[test]
    fn function_declarations_shape() {
        let r = default_registry();
        let decls = r.function_declarations();
        let arr = decls.as_array().expect("array");
        assert_eq!(arr.len(), 1);
        let fds = arr[0]["function_declarations"].as_array().expect("array");
        assert!(fds.len() >= 25);
        for fd in fds {
            assert!(fd["name"].is_string());
            assert!(fd["description"].is_string());
            assert!(fd["parameters"].is_object());
        }
    }
}
