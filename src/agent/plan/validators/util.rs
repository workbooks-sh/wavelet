//! Shared shell-out helpers for query/render/unit validators.
//!
//! Validators that delegate to a subprocess all use the same pattern:
//! spawn, capture stdout/stderr, parse JSON, fold any error into a
//! structured failure detail. This module factors out the boilerplate.

use std::path::Path;
use std::process::{Command, Output};

use serde_json::{json, Value};

/// Outcome of a shell-out: either parsed stdout JSON, or a structured
/// failure detail explaining which step failed.
pub enum ShellJson {
    /// Subprocess exited 0 and stdout parsed cleanly as JSON.
    Ok(Value),
    /// Subprocess exited 0 but stdout was not valid JSON.
    MalformedJson {
        stdout_tail: String,
        parse_error: String,
    },
    /// Subprocess exited non-zero. `stdout_json` is set when stdout
    /// still parsed cleanly (some tools emit structured errors on
    /// nonzero exit).
    NonZero {
        exit_code: Option<i32>,
        stdout_json: Option<Value>,
        stderr_tail: String,
    },
    /// Process could not be spawned at all (binary missing, perm denied).
    Spawn { reason: String },
}

impl ShellJson {
    /// Format a failing variant into the JSON `detail` block. Caller
    /// supplies the command shape it ran so the detail is reproducible.
    pub fn into_failure_detail(self, argv: &[String]) -> Value {
        match self {
            ShellJson::Ok(_) => json!({
                "argv": argv,
                "error": "unreachable_ok_in_failure_path",
            }),
            ShellJson::MalformedJson { stdout_tail, parse_error } => json!({
                "argv": argv,
                "error": "malformed_json",
                "parse_error": parse_error,
                "stdout_tail": stdout_tail,
            }),
            ShellJson::NonZero {
                exit_code,
                stdout_json,
                stderr_tail,
            } => json!({
                "argv": argv,
                "error": "subprocess_nonzero",
                "exit_code": exit_code,
                "stdout_json": stdout_json,
                "stderr_tail": stderr_tail,
            }),
            ShellJson::Spawn { reason } => json!({
                "argv": argv,
                "error": "spawn_failed",
                "reason": reason,
            }),
        }
    }
}

/// Run a binary with the given argv in `workdir`, capture output, parse
/// stdout as JSON. Failure is always structured.
pub fn run_json(bin: &Path, argv_tail: &[String], workdir: &Path) -> ShellJson {
    let mut cmd = Command::new(bin);
    cmd.args(argv_tail).current_dir(workdir);
    let out: Output = match cmd.output() {
        Ok(o) => o,
        Err(e) => return ShellJson::Spawn { reason: e.to_string() },
    };
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    let stdout_json: Option<Value> = serde_json::from_str(&stdout).ok();

    if out.status.success() {
        match stdout_json {
            Some(v) => ShellJson::Ok(v),
            None => ShellJson::MalformedJson {
                stdout_tail: tail(&stdout, 512),
                parse_error: serde_json::from_str::<Value>(&stdout)
                    .err()
                    .map(|e| e.to_string())
                    .unwrap_or_default(),
            },
        }
    } else {
        ShellJson::NonZero {
            exit_code: out.status.code(),
            stdout_json,
            stderr_tail: tail(&stderr, 512),
        }
    }
}

/// Convenience: build the leading argv ([bin_name, subcmd, ...]) string
/// list used in failure details, when callers want to echo what they
/// ran.
pub fn argv_with_bin(bin: &Path, tail: &[String]) -> Vec<String> {
    let mut v = vec![bin.display().to_string()];
    v.extend(tail.iter().cloned());
    v
}

fn tail(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        s.chars().rev().take(n).collect::<String>().chars().rev().collect()
    }
}
