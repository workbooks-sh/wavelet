//! REPL frontend for `wavelet agent chat`.
//!
//! Plain stdin loop — no rustyline dep. Each line the user types is
//! one prompt. Events stream to stderr (so callers can pipe stdout
//! cleanly); the final text reply lands on stdout. Empty lines and
//! lines starting with `/` are ignored (reserved for future REPL
//! commands like `/exit`, `/session`).

use std::io::{self, BufRead, Write};

use super::events::{Event, EventKind};
use super::orchestrator::run_turn;
use super::session::Session;
use super::{AgentConfig, AgentLoop, AgentResult};

/// Run the REPL until EOF (Ctrl-D). Returns the last `AgentResult`
/// or `None` if the user never sent a prompt.
pub fn run_repl(config: AgentConfig) -> Option<AgentResult> {
    let agent = AgentLoop::new(config);
    let mut session = agent.new_session();
    let mut last: Option<AgentResult> = None;

    eprintln!(
        "wavelet agent — model={} (deep={})",
        agent.config.model, agent.config.deep_model
    );
    eprintln!("type a prompt and press enter. Ctrl-D to exit.");
    eprintln!("session id: {}", session.id);
    eprintln!();

    let stdin = io::stdin();
    let mut stdout = io::stdout();
    loop {
        eprint!("> ");
        let _ = io::stderr().flush();
        let mut line = String::new();
        let n = match stdin.lock().read_line(&mut line) {
            Ok(n) => n,
            Err(e) => {
                eprintln!("[wavelet agent] stdin error: {e}");
                break;
            }
        };
        if n == 0 {
            break;
        }
        let prompt = line.trim();
        if prompt.is_empty() {
            continue;
        }
        if prompt == "/exit" || prompt == "/quit" {
            break;
        }
        if prompt == "/session" {
            eprintln!("session id: {}", session.id);
            eprintln!("cost: ${:.4}", session.cost_usd);
            eprintln!("tool calls: {}", session.tool_ledger.len());
            continue;
        }

        let emit = |event: Event| print_event(&event);
        let result = run_turn(&mut session, prompt, &agent.tools, &agent.config, &emit);
        match result {
            Ok(r) => {
                if let Some(text) = &r.final_text {
                    let _ = writeln!(stdout, "{text}");
                }
                if let Some(note) = &r.note {
                    eprintln!("[note] {note}");
                }
                eprintln!(
                    "[done] cost=${:.4}  wall={}ms  files={}",
                    r.cost_usd,
                    r.wall_ms,
                    r.output_files.len()
                );
                last = Some(r);
            }
            Err(e) => {
                eprintln!("[error] {e}");
            }
        }
    }
    last
}

fn print_event(e: &Event) {
    match e.kind {
        EventKind::Thinking => {
            eprintln!(
                "[thinking] step={} phase={} cost=${:.4}",
                e.step.unwrap_or(0),
                e.phase.as_deref().unwrap_or(""),
                e.cost_usd.unwrap_or(0.0)
            );
        }
        EventKind::ToolCall => {
            eprintln!(
                "[tool_call] {} args={}",
                e.tool.as_deref().unwrap_or("?"),
                e.args
                    .as_ref()
                    .map(|v| serde_json::to_string(v).unwrap_or_default())
                    .unwrap_or_default()
            );
        }
        EventKind::ToolResult => {
            let ok = e.ok.unwrap_or(false);
            eprintln!(
                "[tool_result] {} ok={} {}",
                e.tool.as_deref().unwrap_or("?"),
                ok,
                e.summary.as_deref().unwrap_or("")
            );
        }
        EventKind::Review => {
            eprintln!("[review] {}", e.summary.as_deref().unwrap_or(""));
        }
        EventKind::Final => {
            // The REPL prints the final text directly on stdout via
            // the AgentResult; the event is just an FYI.
            eprintln!("[final] cost=${:.4}", e.cost_usd.unwrap_or(0.0));
        }
        EventKind::Error => {
            eprintln!("[error] {}", e.summary.as_deref().unwrap_or(""));
        }
        EventKind::Progress => {
            eprintln!(
                "[progress] step={} cost=${:.4}",
                e.step.unwrap_or(0),
                e.cost_usd.unwrap_or(0.0)
            );
        }
    }
}
