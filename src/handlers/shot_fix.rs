use std::path::PathBuf;
use std::process::ExitCode;
use crate::handlers::util::image_arg_to_url;
use crate::handlers::util::parse_bbox;
use crate::handlers::util::resolve_image_dims;

/// (auto-generated placeholder)
pub fn run_shot_fix(
    input: String,
    instruction: String,
    region: Option<String>,
    image_w: Option<u32>,
    image_h: Option<u32>,
    guidance: Option<f32>,
    steps: Option<u32>,
    seed: Option<u64>,
    backend: String,
    dry_run: bool,
    max_cost: f32,
    cache: PathBuf,
    out: Option<PathBuf>,
    pretty: bool,
) -> ExitCode {
    use crate::backends::fal::{FalClient, FalKontextMaxAdapter};
    use crate::backends::image::{
        region_to_instruction_hint, InstructionEditBackend, InstructionEditRequest,
    };
    use crate::backends::{BackendError, RunMode};

    let source_url = match image_arg_to_url(&input) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("shot fix --input: {e}");
            return ExitCode::from(2);
        }
    };

    let final_instruction = match region.as_deref() {
        None => instruction,
        Some(spec) => {
            let bbox = match parse_bbox(spec) {
                Ok(b) => b,
                Err(e) => {
                    eprintln!("shot fix --region: {e}");
                    return ExitCode::from(2);
                }
            };
            let (w, h) = match resolve_image_dims(&input, image_w, image_h) {
                Ok(d) => d,
                Err(e) => {
                    eprintln!("shot fix --region: {e}");
                    return ExitCode::from(2);
                }
            };
            let hint = region_to_instruction_hint(bbox, w, h);
            format!("{hint}, {instruction}")
        }
    };

    let mut req = InstructionEditRequest::new(source_url, final_instruction);
    req.guidance_scale = guidance;
    req.num_inference_steps = steps;
    req.seed = seed;

    let mode = if dry_run {
        RunMode::DryRun
    } else {
        RunMode::Live {
            max_cost_usd: max_cost,
        }
    };

    let outcome = match backend.as_str() {
        "fal-flux-kontext-max" => {
            let client = if dry_run {
                FalClient::with_key("dry-run", &cache)
            } else {
                match FalClient::from_env(&cache) {
                    Ok(c) => c,
                    Err(e) => {
                        eprintln!("backend fal-flux-kontext-max: {e}");
                        if let BackendError::MissingCredential(name) = &e {
                            eprintln!("set {name} or pass --dry-run to preview.");
                        }
                        return ExitCode::from(2);
                    }
                }
            };
            FalKontextMaxAdapter::new(client).instruction_edit(&req, mode)
        }
        other => {
            eprintln!("unknown --backend '{other}', want fal-flux-kontext-max");
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
                "instruction": req.instruction,
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
            eprintln!("shot fix: {e}");
            ExitCode::from(2)
        }
    }
}
