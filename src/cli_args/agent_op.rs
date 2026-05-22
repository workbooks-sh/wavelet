//! AgentOp — extracted from the wavelet CLI clap definitions.

use std::path::PathBuf;
use clap::{Parser, Subcommand, ValueEnum, Args};
use super::{PlanModeArg};



/// `wavelet agent` subcommands.
#[derive(Subcommand)]
pub enum AgentOp {
    /// Interactive REPL — one prompt per line, events stream to stderr.
    Chat {
        /// Override the Gemini model slug (default `gemini-3.5-flash`).
        #[arg(long)]
        model: Option<String>,
        /// USD cost cap per turn (default 1.00).
        #[arg(long, default_value_t = 1.0)]
        max_cost: f32,
        /// Plan substrate engagement. `off` keeps the legacy step-bounded
        /// loop; `shadow` loads + mutates plan files but still terminates
        /// on `max_steps`; `on` lets plan terminality + wall-clock + budget
        /// gate the turn.
        #[arg(long, value_enum, default_value_t = PlanModeArg::Off)]
        plan_mode: PlanModeArg,
        /// Directory the plan loads from and writes back to. Defaults to
        /// `$cwd/plan` when omitted.
        #[arg(long)]
        plan_workdir: Option<PathBuf>,
        /// Wall-clock cap (seconds) — only consulted in `--plan-mode on`.
        #[arg(long, default_value_t = 1800)]
        max_wall_seconds: u64,
    },
    /// Bind a WebSocket JSON-RPC server.
    Serve {
        /// TCP port to listen on.
        #[arg(long, default_value_t = 8787)]
        port: u16,
        /// Address to bind. Defaults to `127.0.0.1` — flip to `0.0.0.0`
        /// only when behind a TLS-terminating proxy.
        #[arg(long, default_value = "127.0.0.1")]
        bind: String,
        /// Override the Gemini model slug.
        #[arg(long)]
        model: Option<String>,
        /// USD cost cap per turn (default 1.00).
        #[arg(long, default_value_t = 1.0)]
        max_cost: f32,
        /// Plan substrate engagement. See `agent chat --help` for semantics.
        #[arg(long, value_enum, default_value_t = PlanModeArg::Off)]
        plan_mode: PlanModeArg,
        /// Directory the plan loads from and writes back to. Defaults to
        /// `$cwd/plan` when omitted.
        #[arg(long)]
        plan_workdir: Option<PathBuf>,
        /// Wall-clock cap (seconds) — only consulted in `--plan-mode on`.
        #[arg(long, default_value_t = 1800)]
        max_wall_seconds: u64,
    },
    /// Print the JSON tool registry the agent advertises to Gemini.
    Tools,
}
