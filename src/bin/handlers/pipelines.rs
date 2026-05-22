//! `wavelet pipelines` handler — list / show / validate / run.

use std::path::PathBuf;
use std::process::ExitCode;

use serde::Serialize;

use wavelet::pipelines::{default_search_dir, discover, load_from_path};

use super::resolve_pipeline;
use super::super::PipelinesOp;

/// Dispatch entrypoint.
pub fn run(op: PipelinesOp) -> ExitCode {
    match op {
        PipelinesOp::List { dir, json } => list(dir, json),
        PipelinesOp::Show {
            name_or_path,
            dir,
            json,
        } => show(name_or_path, dir, json),
        PipelinesOp::Validate { path } => validate(path),
        PipelinesOp::Run {
            name_or_path,
            brief,
            dir,
        } => print_plan(name_or_path, brief, dir),
    }
}

fn list(dir: Option<PathBuf>, json: bool) -> ExitCode {
    let search = dir.unwrap_or_else(default_search_dir);
    let entries = discover(&search);
    if json {
        #[derive(Serialize)]
        struct Row<'a> {
            path: String,
            ok: bool,
            name: Option<&'a str>,
            version: Option<&'a str>,
            description: Option<&'a str>,
            stages: Option<usize>,
            error: Option<String>,
        }
        let rows: Vec<Row> = entries
            .iter()
            .map(|e| match &e.result {
                Ok(p) => Row {
                    path: e.path.display().to_string(),
                    ok: true,
                    name: Some(&p.name),
                    version: Some(&p.version),
                    description: Some(&p.description),
                    stages: Some(p.stages.len()),
                    error: None,
                },
                Err(err) => Row {
                    path: e.path.display().to_string(),
                    ok: false,
                    name: None,
                    version: None,
                    description: None,
                    stages: None,
                    error: Some(err.to_string()),
                },
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&rows).unwrap());
    } else if entries.is_empty() {
        println!("(no pipelines found under {})", search.display());
    } else {
        for entry in &entries {
            match &entry.result {
                Ok(p) => println!(
                    "{:<24} v{:<10} {:>2} stages  {}",
                    p.name,
                    p.version,
                    p.stages.len(),
                    entry.path.display()
                ),
                Err(err) => println!("(invalid) {}  -- {err}", entry.path.display()),
            }
        }
    }
    ExitCode::SUCCESS
}

fn show(name_or_path: String, dir: Option<PathBuf>, json: bool) -> ExitCode {
    match resolve_pipeline(&name_or_path, dir.as_deref()) {
        Ok((_, pipeline)) => {
            if json {
                println!("{}", serde_json::to_string_pretty(&pipeline).unwrap());
            } else {
                print!("{}", serde_yaml::to_string(&pipeline).unwrap());
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("{e}");
            ExitCode::FAILURE
        }
    }
}

fn validate(path: PathBuf) -> ExitCode {
    match load_from_path(&path) {
        Ok(p) => {
            println!(
                "ok  {}  ({} stages, budget ${:.2}, wall {}m)",
                p.name,
                p.stages.len(),
                p.orchestration.budget_default_usd,
                p.orchestration.max_wall_time_minutes
            );
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("{e}");
            ExitCode::FAILURE
        }
    }
}

fn print_plan(name_or_path: String, brief: Option<PathBuf>, dir: Option<PathBuf>) -> ExitCode {
    match resolve_pipeline(&name_or_path, dir.as_deref()) {
        Ok((path, pipeline)) => {
            #[derive(Serialize)]
            struct Plan<'a> {
                pipeline: &'a str,
                version: &'a str,
                path: String,
                brief: Option<String>,
                budget_usd: f64,
                max_wall_time_minutes: u32,
                stages: Vec<PlanStage<'a>>,
                note: &'static str,
            }
            #[derive(Serialize)]
            struct PlanStage<'a> {
                name: &'a str,
                description: &'a str,
                inputs: &'a [String],
                outputs: &'a [String],
                tools: &'a [String],
                success_criteria: Vec<&'a str>,
            }
            let plan = Plan {
                pipeline: &pipeline.name,
                version: &pipeline.version,
                path: path.display().to_string(),
                brief: brief.map(|b| b.display().to_string()),
                budget_usd: pipeline.orchestration.budget_default_usd,
                max_wall_time_minutes: pipeline.orchestration.max_wall_time_minutes,
                stages: pipeline
                    .stages
                    .iter()
                    .map(|s| PlanStage {
                        name: &s.name,
                        description: &s.description,
                        inputs: &s.required_artifacts_in,
                        outputs: &s.required_artifacts_out,
                        tools: &s.tools_available,
                        success_criteria: s
                            .success_criteria
                            .iter()
                            .map(|c| c.kind.as_str())
                            .collect(),
                    })
                    .collect(),
                note: "Execution stub. Live runtime ships with `wavelet workflow run` (wb-oemp).",
            };
            println!("{}", serde_json::to_string_pretty(&plan).unwrap());
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("{e}");
            ExitCode::FAILURE
        }
    }
}
