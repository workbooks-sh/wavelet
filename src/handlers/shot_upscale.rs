use std::path::PathBuf;
use std::process::ExitCode;
use crate::handlers::util::UpscaleModel;
use crate::handlers::util::UpscaleTarget;
use crate::handlers::util::parse_target;
use crate::handlers::util::resolve_model;

/// (auto-generated placeholder)
#[allow(clippy::too_many_arguments)]
pub fn run_shot_upscale(
    input: String,
    model: String,
    target: String,
    dry_run: bool,
    max_cost: f32,
    cache: PathBuf,
    out: Option<PathBuf>,
    pretty: bool,
) -> ExitCode {
    use crate::backends::fal::{FalClient, FalSupirAdapter};
    use crate::backends::image::{UpscaleBackend, UpscaleRequest};
    use crate::backends::{BackendError, RunMode};

    let picked = match resolve_model(&model, &input) {
        Ok(m) => m,
        Err(msg) => {
            eprintln!("shot upscale: {msg}");
            return ExitCode::from(3);
        }
    };
    let target = match parse_target(&target) {
        Ok(t) => t,
        Err(msg) => {
            eprintln!("shot upscale: {msg}");
            return ExitCode::from(3);
        }
    };
    let mut req = UpscaleRequest::new(input);
    match target {
        UpscaleTarget::Scale(s) => req.target_scale = s,
        UpscaleTarget::Resolution(w, h) => {
            req.target_resolution = Some((w, h));
            req.target_scale = 2.0;
        }
    }
    let mode = if dry_run {
        RunMode::DryRun
    } else {
        RunMode::Live {
            max_cost_usd: max_cost,
        }
    };
    let client = if dry_run {
        FalClient::with_key("dry-run", &cache)
    } else {
        match FalClient::from_env(&cache) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("shot upscale: {e}");
                if let BackendError::MissingCredential(name) = &e {
                    eprintln!("set {name} or pass --dry-run to preview.");
                }
                return ExitCode::from(2);
            }
        }
    };
    let outcome = match picked {
        UpscaleModel::Supir => FalSupirAdapter::new(client).upscale(&req, mode),
    };
    match outcome {
        Ok(outcome) => {
            if let Some(dest) = out.as_ref() {
                if outcome.response.output_bytes > 0 {
                    if let Err(e) = std::fs::copy(&outcome.response.output_path, dest) {
                        eprintln!(
                            "copy {} â {}: {e}",
                            outcome.response.output_path.display(),
                            dest.display()
                        );
                        return ExitCode::from(2);
                    }
                }
            }
            let payload = serde_json::json!({
                "mode": outcome.mode,
                "provider": outcome.provider,
                "request_hash": outcome.request_hash,
                "cached": outcome.cached,
                "cost_estimate_usd": outcome.cost_estimate_usd,
                "result": outcome.response,
            });
            let formatted = if pretty {
                serde_json::to_string_pretty(&payload)
            } else {
                serde_json::to_string(&payload)
            };
            println!(
                "{}",
                formatted.unwrap_or_else(|e| format!(r#"{{"error":"{e}"}}"#))
            );
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("shot upscale: {e}");
            ExitCode::from(2)
        }
    }
}
