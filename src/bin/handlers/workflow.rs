//! `wavelet workflow run` handler — cooperative state-machine walker.

use std::path::PathBuf;
use std::process::ExitCode;

use wavelet::pipelines::{compute_report, StageStatus};

use super::resolve_pipeline;
use super::super::WorkflowOp;

/// Dispatch entrypoint.
pub fn run(op: WorkflowOp) -> ExitCode {
    match op {
        WorkflowOp::Run {
            name_or_path,
            workdir,
            dir,
            text,
        } => run_workflow(name_or_path, workdir, dir, text),
    }
}

fn run_workflow(
    name_or_path: String,
    workdir: Option<PathBuf>,
    dir: Option<PathBuf>,
    text: bool,
) -> ExitCode {
    let pipeline = match resolve_pipeline(&name_or_path, dir.as_deref()) {
        Ok((_, p)) => p,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::FAILURE;
        }
    };
    let workdir = workdir.unwrap_or_else(|| PathBuf::from("."));
    let report = compute_report(&pipeline, &workdir);

    if text {
        // Lead with a one-line all-complete banner when every stage
        // has passed. The 005 v5 post-mortem showed the agent calling
        // `workflow run` four times in a row after gates passed
        // because the "pipeline complete" line was buried at the
        // bottom under per-stage rows. With the banner up top the
        // agent reads ONE line and stops re-querying.
        let all_complete = report
            .stages
            .iter()
            .all(|s| matches!(s.status, StageStatus::Complete));
        if all_complete && !report.stages.is_empty() {
            println!("✓ ALL GATES PASSED — pipeline `{}` complete. No further `workflow run` calls needed.", report.pipeline);
            println!();
        }
        println!("pipeline: {}  workdir: {}", report.pipeline, report.workdir);
        for stage in &report.stages {
            match &stage.status {
                StageStatus::Complete => {
                    println!("  [done]    {}", stage.name);
                }
                StageStatus::Ready { missing_outputs } => {
                    println!(
                        "  [ready]   {}  → produce: {}",
                        stage.name,
                        missing_outputs.join(", ")
                    );
                }
                StageStatus::CriteriaFailed { failed_criteria } => {
                    let summary: Vec<String> = failed_criteria
                        .iter()
                        .map(|f| format!("{}:{}", f.kind, f.reason))
                        .collect();
                    println!(
                        "  [gated]   {}  ✗ criteria: {}",
                        stage.name,
                        summary.join(", ")
                    );
                }
                StageStatus::Blocked { missing_inputs } => {
                    println!(
                        "  [blocked] {}  ← waiting on: {}",
                        stage.name,
                        missing_inputs.join(", ")
                    );
                }
            }
        }
        match report.next_stage {
            Some(name) => println!("next: {name}"),
            None => println!("pipeline complete"),
        }
    } else {
        println!("{}", serde_json::to_string_pretty(&report).unwrap());
    }
    ExitCode::SUCCESS
}
