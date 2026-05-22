use std::path::Path;
use std::process::ExitCode;
use crate::handlers::util::{image_arg_to_url};

/// (auto-generated placeholder)
#[allow(clippy::too_many_arguments)]
pub fn handle_image_isolate(
    image: String,
    prompt: String,
    backend: &str,
    dry_run: bool,
    max_cost: f32,
    cache: &Path,
    out: Option<&Path>,
    pretty: bool,
) -> ExitCode {
    use crate::backends::fal::{FalClient, FalEvfSamAdapter};
    use crate::backends::image::{SegmentByTextBackend, SegmentByTextRequest};
    use crate::backends::{BackendError, RunMode};

    let image = match image_arg_to_url(&image) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("image isolate: {e}");
            return ExitCode::from(2);
        }
    };
    let req = SegmentByTextRequest::new(image, prompt);
    let mode = if dry_run {
        RunMode::DryRun
    } else {
        RunMode::Live { max_cost_usd: max_cost }
    };
    let outcome = match backend {
        "fal-evf-sam" => {
            let client = if dry_run {
                FalClient::with_key("dry-run", cache)
            } else {
                match FalClient::from_env(cache) {
                    Ok(c) => c,
                    Err(e) => {
                        eprintln!("backend fal-evf-sam: {e}");
                        if let BackendError::MissingCredential(name) = &e {
                            eprintln!("set {name} or pass --dry-run to preview.");
                        }
                        return ExitCode::from(2);
                    }
                }
            };
            let adapter = FalEvfSamAdapter::new(client);
            adapter.segment(&req, mode)
        }
        "replicate-grounded-sam" | "grounded-sam" | "sam-3" => {
            use crate::backends::replicate::{ReplicateClient, ReplicateGroundedSamAdapter};
            let client = if dry_run {
                ReplicateClient::with_token("dry-run", cache)
            } else {
                match ReplicateClient::from_env(cache) {
                    Ok(c) => c,
                    Err(e) => {
                        eprintln!("backend replicate-grounded-sam: {e}");
                        if let BackendError::MissingCredential(name) = &e {
                            eprintln!("set {name} or pass --dry-run to preview.");
                        }
                        return ExitCode::from(2);
                    }
                }
            };
            let adapter = ReplicateGroundedSamAdapter::new(client);
            adapter.segment(&req, mode)
        }
        other => {
            eprintln!(
                "unknown --backend '{other}', want fal-evf-sam | replicate-grounded-sam|sam-3"
            );
            return ExitCode::from(3);
        }
    };
    match outcome {
        Ok(outcome) => {
            if let Some(dest) = out {
                if outcome.response.image_bytes > 0 {
                    if let Err(e) = std::fs::copy(&outcome.response.image_path, dest) {
                        eprintln!(
                            "copy {} â {}: {e}",
                            outcome.response.image_path.display(),
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
            println!("{}", formatted.unwrap_or_else(|e| format!(r#"{{"error":"{e}"}}"#)));
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("image isolate: {e}");
            ExitCode::from(2)
        }
    }
}
