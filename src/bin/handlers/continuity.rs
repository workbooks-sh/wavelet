//! `wavelet continuity check` handler — 180°-rule / motion / scale gates.

use std::process::ExitCode;

use wavelet::grammar::{check_continuity, CutSeverity};

use super::super::ContinuityOp;

/// Dispatch entrypoint.
pub fn run(op: ContinuityOp) -> ExitCode {
    match op {
        ContinuityOp::Check { storyboard, json } => {
            let src = match std::fs::read_to_string(&storyboard) {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("read {}: {e}", storyboard.display());
                    return ExitCode::from(2);
                }
            };
            let sb: wavelet::storyboard::Storyboard = match serde_json::from_str(&src) {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("parse {}: {e}", storyboard.display());
                    return ExitCode::from(2);
                }
            };
            let report = check_continuity(&sb);
            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&report).unwrap_or_default()
                );
            } else {
                for f in &report.findings {
                    let prefix = match f.level {
                        CutSeverity::Error => "ERROR",
                        CutSeverity::Warning => "WARN ",
                        CutSeverity::Info => "INFO ",
                    };
                    println!(
                        "{prefix}  [cut {} · {} → {}] {:?}: {}",
                        f.cut_index, f.from_shot, f.into_shot, f.rule, f.message
                    );
                }
                println!();
                if report.ok && report.warnings == 0 {
                    println!(
                        "✓ continuity clean across {} cut(s)",
                        report.cuts_examined
                    );
                } else if report.ok {
                    println!(
                        "◐ {} warning(s) across {} cut(s)",
                        report.warnings, report.cuts_examined
                    );
                } else {
                    println!(
                        "✗ {} error(s), {} warning(s) across {} cut(s)",
                        report.errors, report.warnings, report.cuts_examined
                    );
                }
            }
            if report.ok {
                ExitCode::SUCCESS
            } else {
                ExitCode::from(1)
            }
        }
    }
}
