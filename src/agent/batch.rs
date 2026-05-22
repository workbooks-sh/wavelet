//! Non-interactive frontend for `wavelet agent run`.
//!
//! Reads one prompt (from --prompt, --prompt-file, or stdin), runs a
//! single agent turn, prints events to stderr, and prints the model's
//! final text reply to stdout. Exits 0 on `Done`, 1 on
//! `StepsExhausted` / `BudgetExhausted` / orchestrator error.
//!
//! Used by the workbench eval harness (`agent: "wavelet"` in
//! `wavelet.commercial`) so evals can drive the Gemini-native agent
//! loop the same way they drive `claude` / `codex` / `workhorse`.
//!
//! Reuses the same `run_turn` + event printing as `chat::run_repl` so
//! the two surfaces never drift.

use std::io::{self, Read, Write};
use std::path::Path;
use std::process::ExitCode;

use super::events::{Event, EventKind};
use super::orchestrator::run_turn;
use super::session::Session;
use super::{AgentConfig, AgentLoop};

/// Options for one batch invocation. Mirrors the CLI args at
/// `bin/wavelet.rs::AgentOp::Run`.
pub struct BatchOpts {
    /// Inline prompt (mutually exclusive with prompt_file / stdin).
    pub prompt: Option<String>,
    /// Read prompt from this file (mutually exclusive with prompt / stdin).
    pub prompt_file: Option<std::path::PathBuf>,
    /// If set, chdir here before running so file ops resolve relative
    /// to the eval workdir.
    pub workdir: Option<std::path::PathBuf>,
    /// Emit events as JSON lines on stderr instead of the human format
    /// used by the REPL.
    pub json: bool,
}


fn load_prompt(opts: &BatchOpts) -> Result<String, String> {
    match (&opts.prompt, &opts.prompt_file) {
        (Some(_), Some(_)) => Err("pass only one of --prompt / --prompt-file".into()),
        (Some(p), None) => Ok(p.clone()),
        (None, Some(path)) => read_file(path),
        (None, None) => {
            let mut buf = String::new();
            io::stdin()
                .read_to_string(&mut buf)
                .map_err(|e| format!("stdin: {e}"))?;
            Ok(buf)
        }
    }
}

fn read_file(path: &Path) -> Result<String, String> {
    std::fs::read_to_string(path).map_err(|e| format!("read {}: {e}", path.display()))
}

fn print_event(e: &Event) {
    match e.kind {
        EventKind::Thinking => eprintln!(
            "[thinking] step={} phase={} cost=${:.4}",
            e.step.unwrap_or(0),
            e.phase.as_deref().unwrap_or(""),
            e.cost_usd.unwrap_or(0.0)
        ),
        EventKind::ToolCall => eprintln!(
            "[tool_call] {} args={}",
            e.tool.as_deref().unwrap_or("?"),
            e.args
                .as_ref()
                .map(|v| serde_json::to_string(v).unwrap_or_default())
                .unwrap_or_default()
        ),
        EventKind::ToolResult => eprintln!(
            "[tool_result] {} ok={} {}",
            e.tool.as_deref().unwrap_or("?"),
            e.ok.unwrap_or(false),
            e.summary.as_deref().unwrap_or("")
        ),
        EventKind::Review => eprintln!("[review] {}", e.summary.as_deref().unwrap_or("")),
        EventKind::Final => eprintln!("[final] cost=${:.4}", e.cost_usd.unwrap_or(0.0)),
        EventKind::Error => eprintln!("[error] {}", e.summary.as_deref().unwrap_or("")),
        EventKind::Progress => eprintln!(
            "[progress] step={} cost=${:.4}",
            e.step.unwrap_or(0),
            e.cost_usd.unwrap_or(0.0)
        ),
    }
}
