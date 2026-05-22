use std::process::ExitCode;
use crate::cli_args::AgentOp;

/// (auto-generated placeholder)
pub fn run_agent(op: AgentOp) -> ExitCode {
    use crate::agent::{server, AgentConfig, AgentLoop};
    let mut config = AgentConfig::default();
    match op {
        AgentOp::Chat {
            model,
            max_cost,
            plan_mode,
            plan_workdir,
            max_wall_seconds,
        } => {
            if let Some(m) = model {
                config.model = m;
            }
            config.max_cost_usd = max_cost;
            config.plan_mode = plan_mode.into();
            config.plan_workdir = plan_workdir;
            config.max_wall_seconds = max_wall_seconds;
            crate::agent::chat::run_repl(config);
            ExitCode::SUCCESS
        }
        AgentOp::Serve {
            port,
            bind,
            model,
            max_cost,
            plan_mode,
            plan_workdir,
            max_wall_seconds,
        } => {
            if let Some(m) = model {
                config.model = m;
            }
            config.max_cost_usd = max_cost;
            config.plan_mode = plan_mode.into();
            config.plan_workdir = plan_workdir;
            config.max_wall_seconds = max_wall_seconds;
            let state = server::ServerState::new(config);
            let rt = match tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
            {
                Ok(rt) => rt,
                Err(e) => {
                    eprintln!("agent serve: tokio runtime: {e}");
                    return ExitCode::from(2);
                }
            };
            if let Err(e) = rt.block_on(server::serve(&bind, port, state)) {
                eprintln!("agent serve: {e}");
                return ExitCode::from(2);
            }
            ExitCode::SUCCESS
        }
        AgentOp::Tools => {
            let agent = AgentLoop::new(AgentConfig::default());
            let schemas = agent.tools.schemas();
            match serde_json::to_string_pretty(&schemas) {
                Ok(s) => {
                    println!("{s}");
                    ExitCode::SUCCESS
                }
                Err(e) => {
                    eprintln!("agent tools: serialize: {e}");
                    ExitCode::from(2)
                }
            }
        }
    }
}
