use std::path::Path;
use std::process::ExitCode;
use crate::handlers::util::{image_arg_to_url};

/// (auto-generated placeholder)
#[allow(clippy::too_many_arguments)]
pub fn handle_image_identity_check(
    reference: String,
    candidate: String,
    threshold: f32,
    backend: &str,
    dry_run: bool,
    max_cost: f32,
    cache: &Path,
    pretty: bool,
) -> ExitCode {
    use crate::backends::fal::{FalClient, FalClipSimilarityAdapter, RoboflowClient};
    use crate::backends::image::{IdentityCheckRequest, IdentitySimilarityBackend};
    use crate::backends::roboflow::RoboflowClipAdapter;
    use crate::backends::{BackendError, RunMode};

    let reference = match image_arg_to_url(&reference) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("image identity-check (reference): {e}");
            return ExitCode::from(2);
        }
    };
    let candidate = match image_arg_to_url(&candidate) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("image identity-check (candidate): {e}");
            return ExitCode::from(2);
        }
    };
    let req = IdentityCheckRequest::new(reference, candidate);
    let mode = if dry_run {
        RunMode::DryRun
    } else {
        RunMode::Live { max_cost_usd: max_cost }
    };
    let outcome = match backend {
        "fal-clip-similarity" => {
            let client = if dry_run {
                FalClient::with_key("dry-run", cache)
            } else {
                match FalClient::from_env(cache) {
                    Ok(c) => c,
                    Err(e) => {
                        eprintln!("backend fal-clip-similarity: {e}");
                        if let BackendError::MissingCredential(name) = &e {
                            eprintln!("set {name} or pass --dry-run to preview.");
                        }
                        return ExitCode::from(2);
                    }
                }
            };
            let adapter = FalClipSimilarityAdapter::new(client);
            adapter.check(&req, threshold, mode)
        }
        "roboflow-clip" => {
            let client = if dry_run {
                RoboflowClient::with_key("dry-run", cache)
            } else {
                match RoboflowClient::from_env(cache) {
                    Ok(c) => c,
                    Err(e) => {
                        eprintln!("backend roboflow-clip: {e}");
                        if let BackendError::MissingCredential(name) = &e {
                            eprintln!("set {name} or pass --dry-run to preview.");
                        }
                        return ExitCode::from(2);
                    }
                }
            };
            let adapter = RoboflowClipAdapter::new(client);
            adapter.check(&req, threshold, mode)
        }
        other => {
            eprintln!("unknown --backend '{other}', want roboflow-clip|fal-clip-similarity");
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
            println!(
                "{}",
                formatted.unwrap_or_else(|e| format!(r#"{{"error":"{e}"}}"#))
            );
            if outcome.mode == "live" && !outcome.response.passes_threshold {
                return ExitCode::from(1);
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("image identity-check: {e}");
            ExitCode::from(2)
        }
    }
}
