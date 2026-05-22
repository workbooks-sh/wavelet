//! `wavelet transitions classify` handler.

use std::process::ExitCode;

use wavelet::grammar::classify_transitions;

use super::super::TransitionsOp;

/// Dispatch entrypoint.
pub fn run(op: TransitionsOp) -> ExitCode {
    match op {
        TransitionsOp::Classify {
            screenplay,
            velocity,
            pretty,
            out,
        } => {
            let src = match std::fs::read_to_string(&screenplay) {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("read {}: {e}", screenplay.display());
                    return ExitCode::from(2);
                }
            };
            let s = match fountain::parse(&src) {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("parse {}: {e}", screenplay.display());
                    return ExitCode::from(2);
                }
            };
            let v: wavelet::velocity::VelocityProfile = match std::fs::read_to_string(&velocity)
                .map_err(|e| format!("read {}: {e}", velocity.display()))
                .and_then(|s| serde_json::from_str(&s).map_err(|e| format!("parse velocity: {e}")))
            {
                Ok(v) => v,
                Err(e) => {
                    eprintln!("{e}");
                    return ExitCode::from(2);
                }
            };
            let classification = classify_transitions(
                &s,
                &v,
                screenplay.display().to_string(),
                velocity.display().to_string(),
            );
            let json = if pretty {
                serde_json::to_string_pretty(&classification)
            } else {
                serde_json::to_string(&classification)
            };
            match (json, out) {
                (Ok(j), Some(p)) => {
                    if let Err(e) = std::fs::write(&p, j) {
                        eprintln!("write {}: {e}", p.display());
                        return ExitCode::from(2);
                    }
                    eprintln!(
                        "wrote {} — {} transition(s) classified",
                        p.display(),
                        classification.transitions.len()
                    );
                    ExitCode::SUCCESS
                }
                (Ok(j), None) => {
                    println!("{j}");
                    ExitCode::SUCCESS
                }
                (Err(e), _) => {
                    eprintln!("serialize: {e}");
                    ExitCode::from(2)
                }
            }
        }
    }
}
