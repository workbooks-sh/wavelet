//! Render-pipeline validators: `comp_verify_passes` and
//! `c2pa_verify_passes`. Both delegate to the existing `wavelet verify`
//! and `wavelet c2pa verify` subcommands.
//!
//! `wavelet verify` is not yet `--json`-aware (it prints findings as
//! plain text and exits non-zero when any are ERROR-level). We grade by
//! exit code and surface the stderr/stdout tail in the failure detail.

use std::time::Instant;

use serde_json::{json, Value};

use super::super::validator::{Validator, ValidatorCtx, ValidatorOutcome};
use super::util::{argv_with_bin, run_json, ShellJson};

/// `comp_verify_passes` — params: `{comp, deep?}`. Passes iff
/// `wavelet verify <comp> [--deep]` exits 0.
pub struct CompVerifyPasses;

impl Validator for CompVerifyPasses {
    fn kind(&self) -> &'static str { "comp_verify_passes" }

    fn check(&self, params: &serde_yaml::Value, ctx: &ValidatorCtx) -> ValidatorOutcome {
        let start = Instant::now();
        let Some(comp) = params.get("comp").and_then(|v| v.as_str()) else {
            return ValidatorOutcome {
                ok: false,
                detail: json!({"error": "missing_param", "param": "comp"}),
                cost_usd: 0.0,
                wall_ms: start.elapsed().as_millis(),
            };
        };
        let deep = params.get("deep").and_then(|v| v.as_bool()).unwrap_or(false);

        let mut argv = vec!["verify".to_string(), comp.to_string()];
        if deep { argv.push("--deep".into()); }

        let out = std::process::Command::new(ctx.gamut_bin)
            .args(&argv)
            .current_dir(ctx.workdir)
            .output();
        match out {
            Ok(o) => {
                let stdout = String::from_utf8_lossy(&o.stdout).to_string();
                let stderr = String::from_utf8_lossy(&o.stderr).to_string();
                let ok = o.status.success();
                let detail = if ok {
                    json!({
                        "argv": argv_with_bin(ctx.gamut_bin, &argv),
                        "exit_code": o.status.code(),
                        "stdout_tail": tail(&stdout, 512),
                    })
                } else {
                    json!({
                        "argv": argv_with_bin(ctx.gamut_bin, &argv),
                        "error": "verify_failed",
                        "exit_code": o.status.code(),
                        "stdout_tail": tail(&stdout, 1024),
                        "stderr_tail": tail(&stderr, 512),
                    })
                };
                ValidatorOutcome { ok, detail, cost_usd: 0.0, wall_ms: start.elapsed().as_millis() }
            }
            Err(e) => ValidatorOutcome {
                ok: false,
                detail: json!({
                    "argv": argv_with_bin(ctx.gamut_bin, &argv),
                    "error": "spawn_failed",
                    "reason": e.to_string(),
                }),
                cost_usd: 0.0,
                wall_ms: start.elapsed().as_millis(),
            },
        }
    }
}

/// `c2pa_verify_passes` — params: `{path}`. Shells `wavelet c2pa verify
/// <path> --json`; passes iff exit 0 AND the parsed manifest reports
/// no validation errors.
pub struct C2paVerifyPasses;

impl Validator for C2paVerifyPasses {
    fn kind(&self) -> &'static str { "c2pa_verify_passes" }

    fn check(&self, params: &serde_yaml::Value, ctx: &ValidatorCtx) -> ValidatorOutcome {
        let start = Instant::now();
        let Some(path) = params.get("path").and_then(|v| v.as_str()) else {
            return ValidatorOutcome {
                ok: false,
                detail: json!({"error": "missing_param", "param": "path"}),
                cost_usd: 0.0,
                wall_ms: start.elapsed().as_millis(),
            };
        };

        let argv = vec![
            "c2pa".to_string(),
            "verify".to_string(),
            path.to_string(),
            "--json".to_string(),
        ];

        match run_json(ctx.gamut_bin, &argv, ctx.workdir) {
            ShellJson::Ok(v) => grade_c2pa(v, &argv, ctx.gamut_bin, start),
            other => ValidatorOutcome {
                ok: false,
                detail: other.into_failure_detail(&argv_with_bin(ctx.gamut_bin, &argv)),
                cost_usd: 0.0,
                wall_ms: start.elapsed().as_millis(),
            },
        }
    }
}

fn grade_c2pa(
    manifest: Value,
    argv: &[String],
    bin: &std::path::Path,
    start: Instant,
) -> ValidatorOutcome {
    // Treat a non-empty `validation_status` array (or any nested
    // `validation_errors`) as failure. Otherwise pass.
    let validation_status = manifest
        .pointer("/validation_status")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let errors: Vec<&Value> = validation_status
        .iter()
        .filter(|s| {
            s.get("code")
                .and_then(|c| c.as_str())
                .map(|c| !c.starts_with("ok") && !c.contains("success"))
                .unwrap_or(true)
        })
        .collect();
    let ok = errors.is_empty();
    let detail = if ok {
        json!({
            "argv": argv_with_bin(bin, argv),
            "validation_status": validation_status,
        })
    } else {
        json!({
            "argv": argv_with_bin(bin, argv),
            "failed_clause": "validation_status",
            "errors": errors,
            "manifest": manifest,
        })
    };
    ValidatorOutcome { ok, detail, cost_usd: 0.0, wall_ms: start.elapsed().as_millis() }
}

fn tail(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        s.chars().rev().take(n).collect::<String>().chars().rev().collect()
    }
}
