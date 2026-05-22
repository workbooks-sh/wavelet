use std::path::Path;
use std::process::ExitCode;
use crate::handlers::util::{image_arg_to_url};

/// (auto-generated placeholder)
pub fn handle_image_verify_shot(
    image: String,
    criteria: Vec<String>,
    backend: &str,
    dry_run: bool,
    max_cost: f32,
    cache: &Path,
    pretty: bool,
) -> ExitCode {
    use crate::backends::fal::{FalClient, FalVisionVerifyAdapter};
    use crate::backends::image::{VisionVerifyBackend, VisionVerifyRequest};
    use crate::backends::{BackendError, RunMode};

    if criteria.is_empty() {
        eprintln!("image verify-shot: at least one --criteria flag is required");
        return ExitCode::from(2);
    }
    let image = match image_arg_to_url(&image) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("image verify-shot: {e}");
            return ExitCode::from(2);
        }
    };
    let req = VisionVerifyRequest::new(image, criteria);
    let mode = if dry_run {
        RunMode::DryRun
    } else {
        RunMode::Live { max_cost_usd: max_cost }
    };
    let outcome = match backend {
        "fal-vision-verify" => {
            let client = if dry_run {
                FalClient::with_key("dry-run", cache)
            } else {
                match FalClient::from_env(cache) {
                    Ok(c) => c,
                    Err(e) => {
                        eprintln!("backend fal-vision-verify: {e}");
                        if let BackendError::MissingCredential(name) = &e {
                            eprintln!("set {name} or pass --dry-run to preview.");
                        }
                        return ExitCode::from(2);
                    }
                }
            };
            let adapter = FalVisionVerifyAdapter::new(client);
            adapter.verify(&req, mode)
        }
        other => {
            eprintln!("unknown --backend '{other}', want fal-vision-verify");
            return ExitCode::from(3);
        }
    };
    match outcome {
        Ok(outcome) => {
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
            println!("{}", formatted.unwrap_or_else(|e| format!(r#"{{"error":"{e}"}}"#)));
            if outcome.response.overall_pass {
                ExitCode::SUCCESS
            } else {
                ExitCode::from(1)
            }
        }
        Err(e) => {
            eprintln!("image verify-shot: {e}");
            ExitCode::from(2)
        }
    }
}
