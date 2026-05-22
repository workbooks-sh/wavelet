//! `wavelet diff` handler — per-frame video diff (pixelmatch or SSIM).

use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Instant;

use serde::Serialize;

use wavelet::query::{diff_videos, DiffMetric, DiffOptions, FrameDiff};

use wavelet::handlers::util::parse_rect;

#[derive(Serialize)]
struct DiffSummary {
    ok: bool,
    metric: DiffMetric,
    threshold: f32,
    frames_compared: u32,
    frames_failed: u32,
    median_score: f32,
    p95_score: f32,
    worst: Option<FrameDiff>,
    total_ms: u128,
    report_path: Option<String>,
}

#[allow(clippy::too_many_arguments)]
pub fn run(
    a: PathBuf,
    b: PathBuf,
    metric_s: String,
    threshold: f32,
    clip_s: Option<String>,
    max_diff_ratio: f32,
    report_path: Option<PathBuf>,
) -> ExitCode {
    let total_start = Instant::now();
    let metric = match metric_s.as_str() {
        "pixelmatch" => DiffMetric::Pixelmatch,
        "ssim" => DiffMetric::Ssim,
        other => {
            eprintln!("unknown --metric '{other}', want 'pixelmatch' or 'ssim'");
            return ExitCode::from(3);
        }
    };
    let clip = match clip_s.as_deref() {
        Some(s) => match parse_rect(s) {
            Some(r) => Some(r),
            None => {
                eprintln!("invalid --clip '{s}', want 'x,y,w,h'");
                return ExitCode::from(3);
            }
        },
        None => None,
    };
    let opts = DiffOptions {
        metric,
        threshold,
        clip,
        max_diff_ratio,
    };
    let result = match diff_videos(&a, &b, &opts) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("diff failed: {e}");
            return ExitCode::from(3);
        }
    };

    let mut report_str: Option<String> = None;
    if let Some(p) = report_path.as_ref() {
        let json = serde_json::to_string_pretty(&result).expect("diff report");
        if let Err(e) = std::fs::write(p, &json) {
            eprintln!("write --report {}: {e}", p.display());
            return ExitCode::from(3);
        }
        report_str = Some(p.display().to_string());
    }

    let summary = DiffSummary {
        ok: result.ok,
        metric: result.metric,
        threshold: result.threshold,
        frames_compared: result.frames_compared,
        frames_failed: result.frames_failed,
        median_score: result.median_score,
        p95_score: result.p95_score,
        worst: result.worst.clone(),
        total_ms: total_start.elapsed().as_millis(),
        report_path: report_str,
    };
    let json = serde_json::to_string_pretty(&summary).expect("summary");
    println!("{json}");

    if result.ok {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    }
}
