use std::path::PathBuf;
use std::process::ExitCode;

/// (auto-generated placeholder)
#[allow(clippy::too_many_arguments)]
pub fn run_shot_edit(
    input: PathBuf,
    intent: String,
    out: Option<PathBuf>,
    report: Option<PathBuf>,
    max_attempts: u32,
    max_cost: f32,
    pass_threshold: f32,
    planner_model: String,
    reviewer_model: String,
    dry_run: bool,
) -> ExitCode {
    use crate::edit::intent::{EditConfig, EditRequest, InputKind};

    let kind = match InputKind::classify(&input) {
        Some(k) => k,
        None => {
            eprintln!(
                "wavelet shot edit: input {} must be an .mp4 or .html",
                input.display()
            );
            return ExitCode::from(3);
        }
    };
    let stem_path = input.with_extension("");
    let stem = stem_path.display().to_string();
    let out_path = out.unwrap_or_else(|| PathBuf::from(format!("{stem}-edited.mp4")));
    let report_path =
        report.unwrap_or_else(|| PathBuf::from(format!("{stem}-edit-report.json")));

    let req = EditRequest {
        input: input.clone(),
        kind,
        intent,
        cfg: EditConfig {
            max_attempts,
            max_cost_usd: max_cost,
            pass_threshold,
            planner_model,
            reviewer_model,
            out_path: out_path.clone(),
            report_path: report_path.clone(),
            dry_run,
        },
    };

    if dry_run {
        // Dry-run: plan only. Build the planner prompt and emit the
        // plan JSON to stdout (or the error if the planner failed),
        // and write the report alongside.
        match crate::edit::gemini::api_key_from_env() {
            Ok(api_key) => {
                let prompt = crate::edit::plan::build_planner_prompt(&req, None);
                match crate::edit::gemini::generate_text(&req.cfg.planner_model, &prompt, &api_key)
                {
                    Ok(raw) => match crate::edit::plan::parse_plan(&raw) {
                        Ok(plan) => {
                            println!("{}", serde_json::to_string_pretty(&plan).unwrap_or(raw));
                            return ExitCode::SUCCESS;
                        }
                        Err(e) => {
                            eprintln!("plan parse: {e}");
                            return ExitCode::from(2);
                        }
                    },
                    Err(e) => {
                        eprintln!("planner: {e}");
                        return ExitCode::from(2);
                    }
                }
            }
            Err(e) => {
                eprintln!("{e}");
                return ExitCode::from(2);
            }
        }
    }

    let result = crate::edit::run_edit(req);
    if let Err(e) = crate::edit::write_report(&result, &report_path) {
        eprintln!("warning: could not write report: {e}");
    }
    if result.shipped.is_some() {
        println!(
            "shipped: {} (attempt {}, score {:.2})",
            result.shipped.as_ref().unwrap().display(),
            result.shipped_attempt.unwrap_or(0),
            result.shipped_score.unwrap_or(0.0)
        );
        if let Some(n) = &result.note {
            println!("note: {n}");
        }
        ExitCode::SUCCESS
    } else {
        eprintln!(
            "wavelet shot edit: no attempt shipped. {}",
            result.note.as_deref().unwrap_or("(no note)")
        );
        ExitCode::from(1)
    }
}
