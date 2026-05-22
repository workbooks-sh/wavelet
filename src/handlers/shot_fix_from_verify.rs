use std::path::PathBuf;
use std::process::ExitCode;
use crate::handlers::util::image_arg_to_url;
use crate::handlers::util::parse_verify_findings;

/// (auto-generated placeholder)
pub fn run_shot_fix_from_verify(
    input: String,
    verify_report: PathBuf,
    backend: String,
    dry_run: bool,
    max_cost: f32,
    cache: PathBuf,
    out: Option<PathBuf>,
    pretty: bool,
) -> ExitCode {
    use crate::backends::fal::{FalClient, FalKontextMaxAdapter};
    use crate::backends::image::{
        finding_to_kontext_instruction, Finding, InstructionEditBackend, InstructionEditRequest,
        VisionVerifyResult,
    };
    use crate::backends::{BackendError, RunMode};

    let raw = match std::fs::read_to_string(&verify_report) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("shot fix-from-verify --verify-report: {e}");
            return ExitCode::from(2);
        }
    };
    let findings: Vec<Finding> = match parse_verify_findings(&raw) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("shot fix-from-verify: cannot decode report â {e}");
            return ExitCode::from(2);
        }
    };
    let instructions: Vec<(String, String)> = findings
        .iter()
        .filter_map(|f| {
            finding_to_kontext_instruction(f).map(|i| (f.criterion.clone(), i))
        })
        .collect();

    if instructions.is_empty() {
        let payload = serde_json::json!({
            "mode": "skipped",
            "reason": "no Fail findings in report",
            "input": input,
        });
        let formatted = if pretty {
            serde_json::to_string_pretty(&payload)
        } else {
            serde_json::to_string(&payload)
        };
        println!("{}", formatted.unwrap_or_default());
        return ExitCode::SUCCESS;
    }

    let mode = if dry_run {
        RunMode::DryRun
    } else {
        RunMode::Live {
            max_cost_usd: max_cost,
        }
    };

    if backend != "fal-flux-kontext-max" {
        eprintln!("unknown --backend '{backend}', want fal-flux-kontext-max");
        return ExitCode::from(3);
    }

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
    let adapter = FalKontextMaxAdapter::new(client);

    let mut current_source = match image_arg_to_url(&input) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("shot fix-from-verify --input: {e}");
            return ExitCode::from(2);
        }
    };
    let mut steps_out = Vec::with_capacity(instructions.len());
    let mut final_path: Option<PathBuf> = None;
    let mut total_cost = 0.0f32;

    for (criterion, instr) in &instructions {
        let mut req = InstructionEditRequest::new(current_source.clone(), instr.clone());
        req.seed = None;
        match adapter.instruction_edit(&req, mode) {
            Ok(outcome) => {
                total_cost += outcome.cost_estimate_usd;
                let img_path = outcome.response.image_path.clone();
                let next_source = if mode.is_live() && outcome.response.image_bytes > 0 {
                    // Re-feed: convert the produced image (local cache
                    // path) into a `data:` URL so the next call sees it.
                    match image_arg_to_url(&img_path.to_string_lossy()) {
                        Ok(v) => v,
                        Err(e) => {
                            eprintln!("shot fix-from-verify: chain {criterion}: {e}");
                            return ExitCode::from(2);
                        }
                    }
                } else {
                    current_source.clone()
                };
                steps_out.push(serde_json::json!({
                    "criterion": criterion,
                    "instruction": instr,
                    "provider": outcome.provider,
                    "request_hash": outcome.request_hash,
                    "cached": outcome.cached,
                    "cost_estimate_usd": outcome.cost_estimate_usd,
                    "image_path": img_path,
                }));
                final_path = Some(img_path);
                current_source = next_source;
            }
            Err(e) => {
                eprintln!("shot fix-from-verify: step '{criterion}' failed: {e}");
                return ExitCode::from(2);
            }
        }
    }

    if let (Some(dest), Some(src)) = (out.as_ref(), final_path.as_ref()) {
        if src.exists() {
            if let Err(e) = std::fs::copy(src, dest) {
                eprintln!("copy {} â {}: {e}", src.display(), dest.display());
                return ExitCode::from(2);
            }
        }
    }

    let payload = serde_json::json!({
        "mode": if dry_run { "dry-run" } else { "live" },
        "input": input,
        "steps": steps_out,
        "step_count": instructions.len(),
        "final_image": final_path,
        "total_cost_estimate_usd": total_cost,
    });
    // Reference the type to keep the import live even in dry-run.
    let _ = std::marker::PhantomData::<VisionVerifyResult>;
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
