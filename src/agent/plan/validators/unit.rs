//! `unit_test_passes` — shells `cargo test -p <pkg> <test> --no-fail-fast
//! --quiet`. Passes iff exit 0.

use std::time::Instant;

use serde_json::json;

use super::super::validator::{Validator, ValidatorCtx, ValidatorOutcome};

pub struct UnitTestPasses;

impl Validator for UnitTestPasses {
    fn kind(&self) -> &'static str { "unit_test_passes" }

    fn check(&self, params: &serde_yaml::Value, ctx: &ValidatorCtx) -> ValidatorOutcome {
        let start = Instant::now();
        let Some(pkg) = params.get("pkg").and_then(|v| v.as_str()) else {
            return ValidatorOutcome {
                ok: false,
                detail: json!({"error": "missing_param", "param": "pkg"}),
                cost_usd: 0.0,
                wall_ms: start.elapsed().as_millis(),
            };
        };
        let test = params.get("test").and_then(|v| v.as_str()).unwrap_or("");

        let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_string());
        let mut argv = vec![
            "test".to_string(),
            "-p".to_string(),
            pkg.to_string(),
            "--no-fail-fast".to_string(),
            "--quiet".to_string(),
        ];
        if !test.is_empty() {
            argv.push(test.to_string());
        }

        let out = std::process::Command::new(&cargo)
            .args(&argv)
            .current_dir(ctx.workdir)
            .output();
        match out {
            Ok(o) => {
                let ok = o.status.success();
                let stdout = String::from_utf8_lossy(&o.stdout).to_string();
                let stderr = String::from_utf8_lossy(&o.stderr).to_string();
                let detail = if ok {
                    json!({
                        "cargo": cargo,
                        "argv": argv,
                        "exit_code": o.status.code(),
                        "stdout_tail": tail(&stdout, 512),
                    })
                } else {
                    json!({
                        "cargo": cargo,
                        "argv": argv,
                        "error": "tests_failed",
                        "exit_code": o.status.code(),
                        "stdout_tail": tail(&stdout, 1024),
                        "stderr_tail": tail(&stderr, 1024),
                    })
                };
                ValidatorOutcome { ok, detail, cost_usd: 0.0, wall_ms: start.elapsed().as_millis() }
            }
            Err(e) => ValidatorOutcome {
                ok: false,
                detail: json!({
                    "cargo": cargo,
                    "argv": argv,
                    "error": "spawn_failed",
                    "reason": e.to_string(),
                }),
                cost_usd: 0.0,
                wall_ms: start.elapsed().as_millis(),
            },
        }
    }
}

fn tail(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        s.chars().rev().take(n).collect::<String>().chars().rev().collect()
    }
}
