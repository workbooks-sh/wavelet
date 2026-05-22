//! `wavelet brief check` handler — parse + validate the 9-line ad brief.

use std::process::ExitCode;

use wavelet::director::brief::AdBrief;

use super::super::BriefOp;

/// Dispatch entrypoint.
pub fn run(op: BriefOp) -> ExitCode {
    match op {
        BriefOp::Check { path, json, pretty } => {
            let src = match std::fs::read_to_string(&path) {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("error: read {}: {e}", path.display());
                    return ExitCode::from(2);
                }
            };
            let brief = match AdBrief::from_markdown(&src) {
                Ok(b) => b,
                Err(e) => {
                    eprintln!("error: {e}");
                    return ExitCode::from(1);
                }
            };
            let warnings = brief.warnings();
            if warnings.is_empty() {
                println!("OK");
            } else {
                println!("OK with {} warning(s):", warnings.len());
                for w in &warnings {
                    println!("  {}: {}", w.slot, w.message);
                }
            }
            if json {
                let v = brief.to_json();
                let s = if pretty {
                    serde_json::to_string_pretty(&v).unwrap()
                } else {
                    serde_json::to_string(&v).unwrap()
                };
                println!("{s}");
            }
            ExitCode::SUCCESS
        }
    }
}
