use std::path::PathBuf;
use std::process::ExitCode;
use crate::handlers::shot_still_variants::run_shot_still_variants;
use crate::handlers::util::ShotStillVariantArgs;

/// (auto-generated placeholder)
#[allow(clippy::too_many_arguments)]
pub fn run_shot_still(
    prompt: String,
    backend: Option<String>,
    image_size: String,
    seed: Option<u64>,
    variants: u32,
    select: String,
    criteria: Vec<String>,
    max_variants_cost: Option<f32>,
    dry_run: bool,
    max_cost: f32,
    cache: PathBuf,
    out: Option<PathBuf>,
    pretty: bool,
) -> ExitCode {
    use crate::backends::fal::{FalClient, FalFluxSchnellAdapter};
    use crate::backends::image::{Txt2ImgBackend, Txt2ImgRequest};
    use crate::backends::{BackendError, RunMode};

    let backend = crate::config::resolve_backend(
                    crate::config::BackendKind::Image,
                    backend.as_deref(),
                );
                let mut req = Txt2ImgRequest::new(prompt);
                req.image_size = image_size;
                req.seed = seed;
                let mode = if dry_run {
                    RunMode::DryRun
                } else {
                    RunMode::Live { max_cost_usd: max_cost }
                };
                if variants == 0 || variants > crate::variants::MAX_VARIANTS {
                    eprintln!(
                        "--variants must be 1..={} (got {})",
                        crate::variants::MAX_VARIANTS,
                        variants
                    );
                    return ExitCode::from(3);
                }
                if variants > 1 {
                    return run_shot_still_variants(ShotStillVariantArgs {
                        req,
                        backend,
                        variants,
                        select_raw: select,
                        criteria,
                        max_variants_cost,
                        mode,
                        cache,
                        out,
                        pretty,
                        dry_run,
                    });
                }
                let outcome = match backend.as_str() {
                    "fal-flux-schnell" => {
                        let client = if dry_run {
                            FalClient::with_key("dry-run", &cache)
                        } else {
                            match FalClient::from_env(&cache) {
                                Ok(c) => c,
                                Err(e) => {
                                    eprintln!("backend fal-flux-schnell: {e}");
                                    if let BackendError::MissingCredential(name) = &e {
                                        eprintln!("set {name} or pass --dry-run to preview.");
                                    }
                                    return ExitCode::from(2);
                                }
                            }
                        };
                        let adapter = FalFluxSchnellAdapter::new(client);
                        adapter.generate(&req, mode)
                    }
                    other => {
                        eprintln!("unknown --backend '{other}', want fal-flux-schnell");
                        return ExitCode::from(3);
                    }
                };
                match outcome {
                    Ok(outcome) => {
                        if let Some(dest) = out.as_ref() {
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
                        eprintln!("shot still: {e}");
                        ExitCode::from(2)
                    }
                }
}
