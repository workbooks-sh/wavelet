use std::path::PathBuf;
use std::process::ExitCode;

/// `wavelet shot txt2vid` — text-to-video generation routed exclusively
/// through Google Veo (`Txt2VidGen` cluster).
#[allow(clippy::too_many_arguments)]
pub fn run_shot_txt2vid(
    prompt: String,
    backend: Option<String>,
    duration: f32,
    aspect: String,
    negative: Option<String>,
    no_default_negatives: bool,
    seed: Option<u64>,
    variants: u32,
    _select: String,
    _max_variants_cost: Option<f32>,
    dry_run: bool,
    max_cost: f32,
    cache: PathBuf,
    out: Option<PathBuf>,
    pretty: bool,
) -> ExitCode {
    use crate::backends::google::{GoogleAiClient, GoogleVeoAdapter, VeoModel};
    use crate::backends::video::{Txt2VidGenBackend, Txt2VidRequest};
    use crate::backends::{BackendError, RunMode};

    let backend = crate::config::resolve_backend(
        crate::config::BackendKind::Video,
        backend.as_deref(),
    );
    let req = Txt2VidRequest {
        prompt,
        negative_prompt: negative,
        apply_default_negatives: !no_default_negatives,
        duration_secs: duration,
        aspect_ratio: aspect,
        seed,
    };
    let mode = if dry_run {
        RunMode::DryRun
    } else {
        RunMode::Live {
            max_cost_usd: max_cost,
        }
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
        eprintln!("shot txt2vid: --variants is not supported on the Veo-only cluster yet (got {variants})");
        return ExitCode::from(3);
    }
    let model = match VeoModel::parse(&backend) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("backend: {e}");
            return ExitCode::from(3);
        }
    };
    let client = if dry_run {
        GoogleAiClient::with_key("dry-run", &cache)
    } else {
        match GoogleAiClient::from_env(&cache) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("backend google-veo: {e}");
                if let BackendError::MissingCredential(name) = &e {
                    eprintln!("set {name} or pass --dry-run to preview.");
                }
                return ExitCode::from(2);
            }
        }
    };
    let adapter = GoogleVeoAdapter::new(client, model);
    let outcome = <GoogleVeoAdapter as Txt2VidGenBackend>::generate(&adapter, &req, mode);
    match outcome {
        Ok(outcome) => {
            if let Some(dest) = out.as_ref() {
                if outcome.response.video_bytes > 0 {
                    if let Err(e) = std::fs::copy(&outcome.response.video_path, dest) {
                        eprintln!(
                            "copy {} -> {}: {e}",
                            outcome.response.video_path.display(),
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
            eprintln!("shot txt2vid: {e}");
            ExitCode::from(2)
        }
    }
}
