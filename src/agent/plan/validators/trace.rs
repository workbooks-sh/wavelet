//! Trace-file validators — gate stages on what the agent actually
//! invoked, not just what files appeared on disk.
//!
//! Both validators read newline-delimited JSON traces written by the
//! eval-time PATH shims (`packages/wavelet/evals/bin/{wavelet,brandwork}-traced`).
//! Each record carries the original `argv` of one invocation plus its
//! exit code. The shims log to:
//!
//!   - `<workdir>/.wavelet-trace.jsonl`   — every `wavelet …` call
//!   - `<workdir>/.brandwork-trace.jsonl` — every `brandwork …` call
//!     (`.adalign-trace.jsonl` also accepted during transition window)
//!
//! Outside the eval harness these files are absent; the validators
//! fail with a clear `trace_missing` reason rather than panicking.
//!
//! ## `brandwork_research_done` (was `adalign_research_done`)
//! Pass iff the agent invoked all three required brandwork verbs at
//! least once during the run: `brief`, `brand`, `ads`. Failure detail
//! lists the missing verbs so the agent can self-correct on retry.
//!
//! ## `wavelet_lint_passes`
//! Pass iff the agent ran `wavelet lint …` against `commercial.html`
//! (or its scenes/ peers) and the call exited 0. Catches the common
//! failure where an agent ships layouts with safe-zone violations
//! that lint would have flagged pre-publish.

use std::path::Path;
use std::time::Instant;

use serde_json::{json, Value};

use super::super::validator::{Validator, ValidatorCtx, ValidatorOutcome};

/// Brandwork verbs that must each be invoked at least once during the
/// research stage. Hardcoded for v1; if the verb set ever grows or
/// shrinks we can lift this into the criterion's `params`.
const REQUIRED_BRANDWORK_VERBS: &[&str] = &["brief", "brand", "ads"];

/// Default trace file paths (relative to `ctx.workdir`).
const BRANDWORK_TRACE_REL: &str = ".brandwork-trace.jsonl";
/// Transition-window alias — the old adalign shim wrote here; accepted
/// as fallback when `.brandwork-trace.jsonl` is absent.
const ADALIGN_TRACE_REL: &str = ".adalign-trace.jsonl";
const WAVELET_TRACE_REL: &str = ".wavelet-trace.jsonl";

/// Lint target file the agent is expected to run lint against.
const LINT_TARGET_DEFAULT: &str = "commercial.html";

/// `brandwork_research_done` — every required brandwork verb was invoked.
pub struct BrandworkResearchDone;

/// Deprecation type alias — keep external code that names `AdalignResearchDone`
/// compiling through the transition window.
pub type AdalignResearchDone = BrandworkResearchDone;

/// Criterion-kind alias for pipeline YAML files that still say
/// `kind: adalign_research_done`. Delegates all logic to
/// `BrandworkResearchDone`; only the kind string differs.
pub struct AdalignResearchDoneAlias;

impl Validator for AdalignResearchDoneAlias {
    fn kind(&self) -> &'static str {
        "adalign_research_done"
    }

    fn check(&self, params: &serde_yaml::Value, ctx: &ValidatorCtx) -> ValidatorOutcome {
        BrandworkResearchDone.check(params, ctx)
    }
}

impl Validator for BrandworkResearchDone {
    fn kind(&self) -> &'static str {
        "brandwork_research_done"
    }

    fn check(&self, params: &serde_yaml::Value, ctx: &ValidatorCtx) -> ValidatorOutcome {
        let start = Instant::now();
        // Accept an explicit `trace` param; otherwise try the new
        // `.brandwork-trace.jsonl` path, falling back to the legacy
        // `.adalign-trace.jsonl` during the transition window.
        let explicit_trace = params.get("trace").and_then(|v| v.as_str());
        let (trace_path, trace_rel) = if let Some(t) = explicit_trace {
            (ctx.workdir.join(t), t.to_string())
        } else {
            let new_path = ctx.workdir.join(BRANDWORK_TRACE_REL);
            if new_path.exists() {
                (new_path, BRANDWORK_TRACE_REL.to_string())
            } else {
                // Transition fallback: accept the old adalign trace file
                (ctx.workdir.join(ADALIGN_TRACE_REL), ADALIGN_TRACE_REL.to_string())
            }
        };

        let records = match read_trace(&trace_path) {
            Ok(r) => r,
            Err(detail) => {
                return ValidatorOutcome {
                    ok: false,
                    detail,
                    cost_usd: 0.0,
                    wall_ms: start.elapsed().as_millis(),
                };
            }
        };

        let mut seen: Vec<&str> = Vec::new();
        for rec in &records {
            // Real-call gate: only count an invocation that
            //   - matches a required verb at argv[1]
            //   - did NOT pass `--help` anywhere (an exploration probe)
            //   - exited 0 (the API actually accepted it)
            //   - produced non-trivial stdout (>= 256 bytes — anything
            //     smaller is almost certainly an error envelope or
            //     empty payload)
            //
            // Without these, the agent satisfies the gate by running
            // `brandwork brief --help` three times, never pulling real
            // brand data.
            let Some(verb) = first_arg(rec) else { continue };
            if !REQUIRED_BRANDWORK_VERBS.contains(&verb) || seen.contains(&verb) {
                continue;
            }
            if argv_mentions(rec, "--help") {
                continue;
            }
            let exit_ok = rec.get("exit").and_then(|v| v.as_i64()) == Some(0);
            let stdout_bytes = rec.get("stdout_bytes")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            if exit_ok && stdout_bytes >= 256 {
                seen.push(verb);
            }
        }

        let missing: Vec<&&str> = REQUIRED_BRANDWORK_VERBS
            .iter()
            .filter(|v| !seen.contains(v))
            .collect();

        if missing.is_empty() {
            ValidatorOutcome {
                ok: true,
                detail: json!({
                    "trace": trace_rel,
                    "required": REQUIRED_BRANDWORK_VERBS,
                    "invoked": seen,
                    "calls_logged": records.len(),
                }),
                cost_usd: 0.0,
                wall_ms: start.elapsed().as_millis(),
            }
        } else {
            let missing_strs: Vec<String> = missing.iter().map(|s| (**s).to_string()).collect();
            ValidatorOutcome {
                ok: false,
                detail: json!({
                    "trace": trace_rel,
                    "required": REQUIRED_BRANDWORK_VERBS,
                    "invoked": seen,
                    "missing_verbs": missing_strs,
                    "calls_logged": records.len(),
                    "reason": "missing_brandwork_verbs",
                    "hint": "Phase 1 brand research is required. Run `brandwork brief <domain>`, `brandwork brand <domain>`, and `brandwork ads <domain>` against the brand's actual domain — not `--help` probes — and capture the JSON output. The criterion only counts invocations that exit 0 with non-trivial stdout.",
                }),
                cost_usd: 0.0,
                wall_ms: start.elapsed().as_millis(),
            }
        }
    }
}

/// `wavelet_lint_passes` — agent ran `wavelet lint <target>` and exit was 0.
pub struct WaveletLintPasses;

impl Validator for WaveletLintPasses {
    fn kind(&self) -> &'static str {
        "wavelet_lint_passes"
    }

    fn check(&self, params: &serde_yaml::Value, ctx: &ValidatorCtx) -> ValidatorOutcome {
        let start = Instant::now();
        let trace_rel = params
            .get("trace")
            .and_then(|v| v.as_str())
            .unwrap_or(WAVELET_TRACE_REL);
        let target = params
            .get("target")
            .and_then(|v| v.as_str())
            .unwrap_or(LINT_TARGET_DEFAULT);
        let trace_path = ctx.workdir.join(trace_rel);

        let records = match read_trace(&trace_path) {
            Ok(r) => r,
            Err(detail) => {
                return ValidatorOutcome {
                    ok: false,
                    detail,
                    cost_usd: 0.0,
                    wall_ms: start.elapsed().as_millis(),
                };
            }
        };

        let mut lint_calls: Vec<&Value> = Vec::new();
        for rec in &records {
            if first_arg(rec) == Some("lint") {
                lint_calls.push(rec);
            }
        }

        if lint_calls.is_empty() {
            return ValidatorOutcome {
                ok: false,
                detail: json!({
                    "trace": trace_rel,
                    "reason": "no_lint_invocation",
                    "expected_target": target,
                    "hint": "Run `wavelet lint commercial.html --platform <p> --mp4 commercial.mp4` and fix every reported finding before declaring compose complete.",
                }),
                cost_usd: 0.0,
                wall_ms: start.elapsed().as_millis(),
            };
        }

        // Look for a lint call against the expected target (substring
        // match; the agent may pass `commercial.html` or `./commercial.html`
        // or `scenes/` — accept any argv that mentions the target or a
        // scenes/ peer path) AND that passed `--mp4`. The 008 eval surfaced
        // the failure mode where an agent satisfies the lint gate with an
        // HTML-only scan and never runs the post-render contrast pass.
        // Requiring `--mp4` in the argv forces the agent to lint the final
        // composited MP4, which is the only stage that sees the same
        // pixels the viewer will.
        let mut matched: Option<&Value> = None;
        let mut had_html_match = false;
        for rec in &lint_calls {
            if !(argv_mentions(rec, target) || argv_mentions(rec, "scenes")) {
                continue;
            }
            had_html_match = true;
            if !argv_mentions(rec, "--mp4") {
                continue;
            }
            if let Some(exit) = rec.get("exit").and_then(|v| v.as_i64()) {
                if exit == 0 {
                    matched = Some(rec);
                    break;
                } else if matched.is_none() {
                    matched = Some(rec);
                }
            }
        }
        // Early-exit hint when the agent linted HTML but never passed
        // `--mp4` — distinct from "no lint call ever" so the agent's
        // retry prompt knows exactly what to add.
        if matched.is_none() && had_html_match {
            return ValidatorOutcome {
                ok: false,
                detail: json!({
                    "trace": trace_rel,
                    "target": target,
                    "reason": "missing_mp4_postrender_lint",
                    "lint_calls_logged": lint_calls.len(),
                    "hint": format!(
                        "lint was invoked against {target} but without `--mp4`. The post-render contrast pass is the only stage that sees actual composited pixels (HTML overlay + Veo video). Run `wavelet lint {target} --platform <p> --mp4 commercial.mp4` and clear every finding."
                    ),
                }),
                cost_usd: 0.0,
                wall_ms: start.elapsed().as_millis(),
            };
        }

        match matched {
            Some(rec) if rec.get("exit").and_then(|v| v.as_i64()) == Some(0) => ValidatorOutcome {
                ok: true,
                detail: json!({
                    "trace": trace_rel,
                    "target": target,
                    "lint_calls_logged": lint_calls.len(),
                    "matched_argv": rec.get("argv").cloned().unwrap_or(Value::Null),
                }),
                cost_usd: 0.0,
                wall_ms: start.elapsed().as_millis(),
            },
            Some(rec) => ValidatorOutcome {
                ok: false,
                detail: json!({
                    "trace": trace_rel,
                    "target": target,
                    "reason": "lint_nonzero_exit",
                    "lint_calls_logged": lint_calls.len(),
                    "last_matching_call": rec,
                    "hint": "wavelet lint returned a non-zero exit. Fix every reported finding and re-run.",
                }),
                cost_usd: 0.0,
                wall_ms: start.elapsed().as_millis(),
            },
            None => ValidatorOutcome {
                ok: false,
                detail: json!({
                    "trace": trace_rel,
                    "target": target,
                    "reason": "lint_target_mismatch",
                    "lint_calls_logged": lint_calls.len(),
                    "hint": format!(
                        "Lint was invoked but not against `{target}` or `scenes/`. Run `wavelet lint {target} --platform <p>`."
                    ),
                }),
                cost_usd: 0.0,
                wall_ms: start.elapsed().as_millis(),
            },
        }
    }
}

/// `screenplay_duration_fits` — agent ran `wavelet screenplay validate
/// <fountain> --duration <secs>` and the call exited 0. The validate
/// subcommand exits 3 on `over_budget`, so a 0 exit means the script's
/// estimated read time fits the declared spot length within tolerance.
pub struct ScreenplayDurationFits;

impl Validator for ScreenplayDurationFits {
    fn kind(&self) -> &'static str {
        "screenplay_duration_fits"
    }

    fn check(&self, params: &serde_yaml::Value, ctx: &ValidatorCtx) -> ValidatorOutcome {
        let start = Instant::now();
        let trace_rel = params
            .get("trace")
            .and_then(|v| v.as_str())
            .unwrap_or(WAVELET_TRACE_REL);
        let trace_path = ctx.workdir.join(trace_rel);

        let records = match read_trace(&trace_path) {
            Ok(r) => r,
            Err(detail) => {
                return ValidatorOutcome {
                    ok: false,
                    detail,
                    cost_usd: 0.0,
                    wall_ms: start.elapsed().as_millis(),
                };
            }
        };

        // Look for any `screenplay validate` call. argv shape is
        // ["wavelet", "screenplay", "validate", ...]. The handler exits
        // 0 on fits/under_budget and 3 on over_budget — we only count
        // the most recent (last) matching call so a re-run after a
        // rewrite supersedes the initial overshoot.
        let mut last_call: Option<&Value> = None;
        for rec in &records {
            if first_arg(rec) == Some("screenplay")
                && argv_at(rec, 2) == Some("validate")
            {
                last_call = Some(rec);
            }
        }

        match last_call {
            Some(rec) if rec.get("exit").and_then(|v| v.as_i64()) == Some(0) => {
                ValidatorOutcome {
                    ok: true,
                    detail: json!({
                        "trace": trace_rel,
                        "matched_argv": rec.get("argv").cloned().unwrap_or(Value::Null),
                    }),
                    cost_usd: 0.0,
                    wall_ms: start.elapsed().as_millis(),
                }
            }
            Some(rec) => ValidatorOutcome {
                ok: false,
                detail: json!({
                    "trace": trace_rel,
                    "reason": "screenplay_over_budget",
                    "last_matching_call": rec,
                    "hint": "wavelet screenplay validate returned non-zero — the script's estimated read time exceeds the declared spot length. Cut copy (or extend the spot) and re-run validate before moving to storyboard.",
                }),
                cost_usd: 0.0,
                wall_ms: start.elapsed().as_millis(),
            },
            None => ValidatorOutcome {
                ok: false,
                detail: json!({
                    "trace": trace_rel,
                    "reason": "no_screenplay_validate_call",
                    "hint": "Run `wavelet screenplay validate <fountain> --duration <secs>` after writing the screenplay and before storyboard. The gate refuses to advance until validate exits 0.",
                }),
                cost_usd: 0.0,
                wall_ms: start.elapsed().as_millis(),
            },
        }
    }
}

/// Read a `.jsonl` trace and parse each non-empty line as JSON. Returns
/// a structured failure detail on read error or empty trace.
fn read_trace(path: &Path) -> Result<Vec<Value>, Value> {
    let raw = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) => {
            return Err(json!({
                "trace": path.display().to_string(),
                "reason": "trace_missing",
                "error": e.to_string(),
                "hint": "The PATH shim writes this file. If you're running outside the eval harness, the criterion can't grade.",
            }));
        }
    };
    let mut out = Vec::new();
    for line in raw.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Ok(v) = serde_json::from_str::<Value>(trimmed) {
            out.push(v);
        }
    }
    Ok(out)
}

/// First positional argument after argv[0]. For wavelet/brandwork argv
/// the shape is `["wavelet", "<verb>", …]` / `["brandwork", "<verb>", …]`.
fn first_arg(rec: &Value) -> Option<&str> {
    argv_at(rec, 1)
}

/// argv entry at a specific index, when present.
fn argv_at(rec: &Value, idx: usize) -> Option<&str> {
    rec.get("argv")
        .and_then(|v| v.as_array())
        .and_then(|a| a.get(idx))
        .and_then(|v| v.as_str())
}

/// True iff any argv entry contains `needle` as a substring.
fn argv_mentions(rec: &Value, needle: &str) -> bool {
    rec.get("argv")
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str())
                .any(|s| s.contains(needle))
        })
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    fn ctx<'a>(workdir: &'a Path, bin: &'a Path) -> ValidatorCtx<'a> {
        ValidatorCtx {
            workdir,
            gamut_bin: bin,
            session_cost_usd: 0.0,
        }
    }

    fn yaml_null() -> serde_yaml::Value {
        serde_yaml::Value::Null
    }

    #[test]
    fn brandwork_research_done_passes_when_all_three_verbs_invoked() {
        let dir = tempfile::tempdir().unwrap();
        let workdir = dir.path();
        let trace = workdir.join(".brandwork-trace.jsonl");
        // stdout_bytes >= 256 to clear the real-call threshold
        let lines = [
            r#"{"ts":"2026-05-22T00:00:00Z","argv":["brandwork","brief","--brand","kitchenaid"],"duration_ms":12,"exit":0,"stdout_bytes":4096,"stderr_bytes":0}"#,
            r#"{"ts":"2026-05-22T00:00:01Z","argv":["brandwork","brand","kitchenaid"],"duration_ms":12,"exit":0,"stdout_bytes":1024,"stderr_bytes":0}"#,
            r#"{"ts":"2026-05-22T00:00:02Z","argv":["brandwork","ads","kitchenaid"],"duration_ms":12,"exit":0,"stdout_bytes":2048,"stderr_bytes":0}"#,
        ];
        fs::write(&trace, lines.join("\n")).unwrap();

        let bin = PathBuf::from("wavelet");
        let outcome = BrandworkResearchDone.check(&yaml_null(), &ctx(workdir, &bin));
        assert!(outcome.ok, "expected pass, detail={:?}", outcome.detail);
        assert_eq!(outcome.detail["calls_logged"], serde_json::Value::from(3));
    }

    #[test]
    fn brandwork_research_done_rejects_help_probes_and_empty_stdout() {
        let dir = tempfile::tempdir().unwrap();
        let workdir = dir.path();
        let trace = workdir.join(".brandwork-trace.jsonl");
        // All three verbs called but each with --help or trivial output.
        // None should count as a real research call.
        let lines = [
            r#"{"ts":"2026-05-22T00:00:00Z","argv":["brandwork","brief","--help"],"duration_ms":12,"exit":0,"stdout_bytes":4096,"stderr_bytes":0}"#,
            r#"{"ts":"2026-05-22T00:00:01Z","argv":["brandwork","brand"],"duration_ms":12,"exit":0,"stdout_bytes":40,"stderr_bytes":0}"#,
            r#"{"ts":"2026-05-22T00:00:02Z","argv":["brandwork","ads","kitchenaid"],"duration_ms":12,"exit":1,"stdout_bytes":4096,"stderr_bytes":80}"#,
        ];
        fs::write(&trace, lines.join("\n")).unwrap();

        let bin = PathBuf::from("wavelet");
        let outcome = BrandworkResearchDone.check(&yaml_null(), &ctx(workdir, &bin));
        assert!(!outcome.ok, "expected fail (help probes + small stdout + nonzero exit)");
        let missing = outcome.detail["missing_verbs"].as_array().unwrap();
        assert_eq!(missing.len(), 3, "all three should be missing");
    }

    #[test]
    fn brandwork_research_done_fails_when_trace_missing() {
        let dir = tempfile::tempdir().unwrap();
        let bin = PathBuf::from("wavelet");
        let outcome = BrandworkResearchDone.check(&yaml_null(), &ctx(dir.path(), &bin));
        assert!(!outcome.ok);
        assert_eq!(
            outcome.detail["reason"],
            serde_json::Value::String("trace_missing".into())
        );
    }

    #[test]
    fn brandwork_research_done_lists_missing_verbs() {
        let dir = tempfile::tempdir().unwrap();
        let workdir = dir.path();
        let trace = workdir.join(".brandwork-trace.jsonl");
        // Only `brief` invoked as a real call; `brand` and `ads` missing.
        fs::write(
            &trace,
            r#"{"ts":"2026-05-22T00:00:00Z","argv":["brandwork","brief","--brand","kitchenaid"],"duration_ms":12,"exit":0,"stdout_bytes":4096,"stderr_bytes":0}"#,
        )
        .unwrap();

        let bin = PathBuf::from("wavelet");
        let outcome = BrandworkResearchDone.check(&yaml_null(), &ctx(workdir, &bin));
        assert!(!outcome.ok);
        let missing = outcome.detail["missing_verbs"]
            .as_array()
            .expect("missing_verbs array");
        let names: Vec<&str> = missing.iter().filter_map(|v| v.as_str()).collect();
        assert!(names.contains(&"brand"));
        assert!(names.contains(&"ads"));
        assert!(!names.contains(&"brief"));
    }

    #[test]
    fn wavelet_lint_passes_passes_on_zero_exit_against_commercial_html() {
        let dir = tempfile::tempdir().unwrap();
        let workdir = dir.path();
        let trace = workdir.join(".wavelet-trace.jsonl");
        fs::write(
            &trace,
            r#"{"ts":"2026-05-22T00:00:00Z","argv":["wavelet","lint","commercial.html","--platform","instagram","--mp4","commercial.mp4"],"duration_ms":40,"exit":0,"stdout_bytes":10,"stderr_bytes":0}"#,
        )
        .unwrap();

        let bin = PathBuf::from("wavelet");
        let outcome = WaveletLintPasses.check(&yaml_null(), &ctx(workdir, &bin));
        assert!(outcome.ok, "expected pass, detail={:?}", outcome.detail);
    }

    #[test]
    fn wavelet_lint_passes_fails_on_nonzero_exit() {
        let dir = tempfile::tempdir().unwrap();
        let workdir = dir.path();
        let trace = workdir.join(".wavelet-trace.jsonl");
        fs::write(
            &trace,
            r#"{"ts":"2026-05-22T00:00:00Z","argv":["wavelet","lint","commercial.html","--mp4","commercial.mp4"],"duration_ms":40,"exit":1,"stdout_bytes":10,"stderr_bytes":40}"#,
        )
        .unwrap();

        let bin = PathBuf::from("wavelet");
        let outcome = WaveletLintPasses.check(&yaml_null(), &ctx(workdir, &bin));
        assert!(!outcome.ok);
        assert_eq!(
            outcome.detail["reason"],
            serde_json::Value::String("lint_nonzero_exit".into())
        );
    }

    #[test]
    fn wavelet_lint_passes_fails_when_html_lint_skipped_mp4_flag() {
        // The 008-failure case: agent lints HTML but never with --mp4.
        // The new gate flags this distinct from a complete no-lint case.
        let dir = tempfile::tempdir().unwrap();
        let workdir = dir.path();
        let trace = workdir.join(".wavelet-trace.jsonl");
        fs::write(
            &trace,
            r#"{"ts":"2026-05-22T00:00:00Z","argv":["wavelet","lint","commercial.html","--platform","instagram"],"duration_ms":40,"exit":0,"stdout_bytes":10,"stderr_bytes":0}"#,
        )
        .unwrap();
        let bin = PathBuf::from("wavelet");
        let outcome = WaveletLintPasses.check(&yaml_null(), &ctx(workdir, &bin));
        assert!(!outcome.ok);
        assert_eq!(
            outcome.detail["reason"],
            serde_json::Value::String("missing_mp4_postrender_lint".into())
        );
    }

    #[test]
    fn wavelet_lint_passes_fails_when_no_lint_invoked() {
        let dir = tempfile::tempdir().unwrap();
        let workdir = dir.path();
        let trace = workdir.join(".wavelet-trace.jsonl");
        fs::write(
            &trace,
            r#"{"ts":"2026-05-22T00:00:00Z","argv":["wavelet","render","commercial.html"],"duration_ms":40,"exit":0,"stdout_bytes":10,"stderr_bytes":0}"#,
        )
        .unwrap();

        let bin = PathBuf::from("wavelet");
        let outcome = WaveletLintPasses.check(&yaml_null(), &ctx(workdir, &bin));
        assert!(!outcome.ok);
        assert_eq!(
            outcome.detail["reason"],
            serde_json::Value::String("no_lint_invocation".into())
        );
    }

    #[test]
    fn wavelet_lint_passes_accepts_scenes_dir_target() {
        let dir = tempfile::tempdir().unwrap();
        let workdir = dir.path();
        let trace = workdir.join(".wavelet-trace.jsonl");
        fs::write(
            &trace,
            r#"{"ts":"2026-05-22T00:00:00Z","argv":["wavelet","lint","scenes/","--platform","instagram","--mp4","commercial.mp4"],"duration_ms":40,"exit":0,"stdout_bytes":10,"stderr_bytes":0}"#,
        )
        .unwrap();

        let bin = PathBuf::from("wavelet");
        let outcome = WaveletLintPasses.check(&yaml_null(), &ctx(workdir, &bin));
        assert!(outcome.ok, "expected pass, detail={:?}", outcome.detail);
    }

    #[test]
    fn screenplay_duration_fits_passes_on_zero_exit() {
        let dir = tempfile::tempdir().unwrap();
        let workdir = dir.path();
        let trace = workdir.join(".wavelet-trace.jsonl");
        fs::write(
            &trace,
            r#"{"ts":"2026-05-22T00:00:00Z","argv":["wavelet","screenplay","validate","script.fountain","--duration","12"],"duration_ms":15,"exit":0,"stdout_bytes":480,"stderr_bytes":0}"#,
        )
        .unwrap();

        let bin = PathBuf::from("wavelet");
        let outcome = ScreenplayDurationFits.check(&yaml_null(), &ctx(workdir, &bin));
        assert!(outcome.ok, "expected pass, detail={:?}", outcome.detail);
    }

    #[test]
    fn screenplay_duration_fits_fails_on_over_budget_exit() {
        let dir = tempfile::tempdir().unwrap();
        let workdir = dir.path();
        let trace = workdir.join(".wavelet-trace.jsonl");
        fs::write(
            &trace,
            r#"{"ts":"2026-05-22T00:00:00Z","argv":["wavelet","screenplay","validate","script.fountain","--duration","12"],"duration_ms":15,"exit":3,"stdout_bytes":480,"stderr_bytes":0}"#,
        )
        .unwrap();

        let bin = PathBuf::from("wavelet");
        let outcome = ScreenplayDurationFits.check(&yaml_null(), &ctx(workdir, &bin));
        assert!(!outcome.ok);
        assert_eq!(
            outcome.detail["reason"],
            serde_json::Value::String("screenplay_over_budget".into())
        );
    }

    #[test]
    fn screenplay_duration_fits_fails_when_validate_never_called() {
        let dir = tempfile::tempdir().unwrap();
        let workdir = dir.path();
        let trace = workdir.join(".wavelet-trace.jsonl");
        fs::write(
            &trace,
            r#"{"ts":"2026-05-22T00:00:00Z","argv":["wavelet","screenplay","parse","script.fountain"],"duration_ms":15,"exit":0,"stdout_bytes":480,"stderr_bytes":0}"#,
        )
        .unwrap();

        let bin = PathBuf::from("wavelet");
        let outcome = ScreenplayDurationFits.check(&yaml_null(), &ctx(workdir, &bin));
        assert!(!outcome.ok);
        assert_eq!(
            outcome.detail["reason"],
            serde_json::Value::String("no_screenplay_validate_call".into())
        );
    }

    #[test]
    fn screenplay_duration_fits_uses_last_call_when_retried() {
        // First call fails (over budget); agent rewrites + re-runs and
        // gets a zero exit. Validator should reflect the latest state.
        let dir = tempfile::tempdir().unwrap();
        let workdir = dir.path();
        let trace = workdir.join(".wavelet-trace.jsonl");
        let lines = [
            r#"{"ts":"2026-05-22T00:00:00Z","argv":["wavelet","screenplay","validate","script.fountain","--duration","12"],"duration_ms":15,"exit":3,"stdout_bytes":480,"stderr_bytes":0}"#,
            r#"{"ts":"2026-05-22T00:01:00Z","argv":["wavelet","screenplay","validate","script.fountain","--duration","12"],"duration_ms":15,"exit":0,"stdout_bytes":480,"stderr_bytes":0}"#,
        ];
        fs::write(&trace, lines.join("\n")).unwrap();

        let bin = PathBuf::from("wavelet");
        let outcome = ScreenplayDurationFits.check(&yaml_null(), &ctx(workdir, &bin));
        assert!(outcome.ok, "expected pass after rewrite, detail={:?}", outcome.detail);
    }
}
