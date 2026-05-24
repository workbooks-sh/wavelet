use std::path::PathBuf;
use std::process::ExitCode;

use crate::backends::exit_for_backend_error;

/// `wavelet shot txt2vid` — text-to-video generation via the
/// `Txt2VidGen` cluster, or reference-to-video via `MultiRefVideoGen`
/// when `--reference` flags are supplied with a `fal-veo3-ref` backend.
///
/// Defaults to Google Veo (via `veo-lite`). Pass `--backend fal-veo3`
/// or `--backend fal-veo3-fast` to route through Fal's queue API.
/// Pass `--backend fal-veo3-ref --reference <path>` (repeatable, up to
/// 4) for character-consistent reference-to-video generation.
#[allow(clippy::too_many_arguments)]
pub fn run_shot_txt2vid(
    prompt: String,
    backend: Option<String>,
    duration: f32,
    aspect: String,
    negative: Option<String>,
    no_default_negatives: bool,
    seed: Option<u64>,
    variants: u32,
    _select: String,
    _max_variants_cost: Option<f32>,
    dry_run: bool,
    max_cost: f32,
    cache: PathBuf,
    out: Option<PathBuf>,
    pretty: bool,
    no_trim_static: bool,
    references: Vec<PathBuf>,
) -> ExitCode {
    use crate::backends::video::{Txt2VidGenBackend, Txt2VidRequest};
    use crate::backends::{BackendError, RunMode};

    let backend = crate::config::resolve_backend(
        crate::config::BackendKind::Video,
        backend.as_deref(),
    );
    let req = Txt2VidRequest {
        prompt,
        negative_prompt: negative,
        apply_default_negatives: !no_default_negatives,
        duration_secs: duration,
        aspect_ratio: aspect,
        seed,
    };
    let mode = if dry_run {
        RunMode::DryRun
    } else {
        RunMode::Live {
            max_cost_usd: max_cost,
        }
    };
    if variants == 0 || variants > crate::variants::MAX_VARIANTS {
        eprintln!(
            "--variants must be 1..={} (got {})",
            crate::variants::MAX_VARIANTS,
            variants
        );
        return ExitCode::from(3);
    }
    if variants > 1 {
        eprintln!("shot txt2vid: --variants is not supported on the Veo-only cluster yet (got {variants})");
        return ExitCode::from(3);
    }

    // Route to Fal Veo 3.1 reference-to-video when backend is a ref variant.
    // Must check before the generic "fal-veo" prefix check below.
    if backend.contains("ref") && backend.starts_with("fal-veo") {
        return run_fal_veo_ref(
            &backend,
            &req,
            &references,
            mode,
            dry_run,
            &cache,
            out,
            pretty,
            no_trim_static,
        );
    }

    // Route to Fal Veo text-only when the backend name starts with "fal-veo".
    if backend.starts_with("fal-veo") {
        return run_fal_veo(&backend, &req, mode, dry_run, &cache, out, pretty, no_trim_static);
    }

    // Default: Google Veo via AI Studio.
    use crate::backends::google::{GoogleAiClient, GoogleVeoAdapter, VeoModel};
    let model = match VeoModel::parse(&backend) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("backend: {e}");
            return ExitCode::from(3);
        }
    };
    let client = if dry_run {
        GoogleAiClient::with_key("dry-run", &cache)
    } else {
        match GoogleAiClient::from_env(&cache) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("backend google-veo: {e}");
                if let BackendError::MissingCredential(name) = &e {
                    eprintln!("set {name} or pass --dry-run to preview.");
                }
                return exit_for_backend_error(&e);
            }
        }
    };
    let adapter = GoogleVeoAdapter::new(client, model);
    let outcome = <GoogleVeoAdapter as Txt2VidGenBackend>::generate(&adapter, &req, mode);
    emit_outcome(outcome, out, dry_run, no_trim_static, pretty)
}

/// Handle `--backend fal-veo3*` routes by constructing a `FalVeoAdapter`
/// and delegating to the same emit / trim logic the Google path uses.
#[allow(clippy::too_many_arguments)]
fn run_fal_veo(
    backend: &str,
    req: &crate::backends::video::Txt2VidRequest,
    mode: crate::backends::RunMode,
    dry_run: bool,
    cache: &PathBuf,
    out: Option<PathBuf>,
    pretty: bool,
    no_trim_static: bool,
) -> ExitCode {
    use crate::backends::fal::{FalVeoAdapter, FalVeoModel};
    use crate::backends::video::Txt2VidGenBackend;
    use crate::backends::BackendError;

    let model = match FalVeoModel::parse(backend) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("backend: {e}");
            return ExitCode::from(3);
        }
    };
    let adapter = if dry_run {
        FalVeoAdapter::new(
            crate::backends::fal::FalClient::with_key("dry-run", cache),
            model,
            "dry-run",
        )
    } else {
        match FalVeoAdapter::from_env(model, cache) {
            Ok(a) => a,
            Err(e) => {
                eprintln!("backend fal-veo: {e}");
                if let BackendError::MissingCredential(name) = &e {
                    eprintln!("set {name} or pass --dry-run to preview.");
                }
                return exit_for_backend_error(&e);
            }
        }
    };
    let outcome = <FalVeoAdapter as Txt2VidGenBackend>::generate(&adapter, req, mode);
    emit_outcome(outcome, out, dry_run, no_trim_static, pretty)
}

/// Handle `--backend fal-veo3-ref*` routes by constructing a
/// `FalVeoRefAdapter` and delegating to the same emit / trim logic.
///
/// Returns exit code 3 (bad args) when `references` is empty — the
/// ref-to-video model requires at least one `--reference` image.
#[allow(clippy::too_many_arguments)]
fn run_fal_veo_ref(
    backend: &str,
    txt2vid_req: &crate::backends::video::Txt2VidRequest,
    references: &[PathBuf],
    mode: crate::backends::RunMode,
    dry_run: bool,
    cache: &PathBuf,
    out: Option<PathBuf>,
    pretty: bool,
    no_trim_static: bool,
) -> ExitCode {
    use crate::backends::fal::{FalVeoRefAdapter, FalVeoRefModel};
    use crate::backends::video::{MultiRefVideoGenBackend, MultiRefVideoRequest};
    use crate::backends::BackendError;

    if references.is_empty() {
        eprintln!(
            "backend {backend}: ref-to-video requires at least one --reference <path>. \
             Use --backend fal-veo3 for text-only generation."
        );
        return ExitCode::from(3);
    }

    let model = match FalVeoRefModel::parse(backend) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("backend: {e}");
            return ExitCode::from(3);
        }
    };

    let adapter = if dry_run {
        FalVeoRefAdapter::new(
            crate::backends::fal::FalClient::with_key("dry-run", cache),
            model,
            "dry-run",
        )
    } else {
        match FalVeoRefAdapter::from_env(model, cache) {
            Ok(a) => a,
            Err(e) => {
                eprintln!("backend fal-veo-ref: {e}");
                if let BackendError::MissingCredential(name) = &e {
                    eprintln!("set {name} or pass --dry-run to preview.");
                }
                return exit_for_backend_error(&e);
            }
        }
    };

    let ref_strings: Vec<String> = references
        .iter()
        .map(|p| p.display().to_string())
        .collect();

    let req = MultiRefVideoRequest {
        prompt: txt2vid_req.prompt.clone(),
        reference_images: ref_strings,
        reference_videos: Vec::new(),
        negative_prompt: txt2vid_req.negative_prompt.clone(),
        duration_secs: txt2vid_req.duration_secs,
        aspect_ratio: txt2vid_req.aspect_ratio.clone(),
        seed: txt2vid_req.seed,
    };

    let outcome = <FalVeoRefAdapter as MultiRefVideoGenBackend>::generate(&adapter, &req, mode);
    emit_outcome(outcome, out, dry_run, no_trim_static, pretty)
}

/// Emit the JSON outcome of a txt2vid call — shared by both dispatch
/// arms so the output shape is identical regardless of backend.
fn emit_outcome(
    outcome: Result<
        crate::backends::BackendCallOutcome<crate::backends::video::VideoResult>,
        crate::backends::BackendError,
    >,
    out: Option<PathBuf>,
    dry_run: bool,
    no_trim_static: bool,
    pretty: bool,
) -> ExitCode {
    match outcome {
        Ok(outcome) => {
            let mut trim_summary: serde_json::Value = serde_json::Value::Null;
            if let Some(dest) = out.as_ref() {
                if outcome.response.video_bytes > 0 {
                    if let Err(e) = std::fs::copy(&outcome.response.video_path, dest) {
                        eprintln!(
                            "copy {} -> {}: {e}",
                            outcome.response.video_path.display(),
                            dest.display()
                        );
                        return ExitCode::from(1);
                    }
                    if !dry_run && !no_trim_static {
                        trim_summary = auto_trim_in_place(dest);
                    }
                }
            }
            let payload = serde_json::json!({
                "mode": outcome.mode,
                "provider": outcome.provider,
                "request_hash": outcome.request_hash,
                "cached": outcome.cached,
                "cost_estimate_usd": outcome.cost_estimate_usd,
                "result": outcome.response,
                "trim_static": trim_summary,
            });
            let formatted = if pretty {
                serde_json::to_string_pretty(&payload)
            } else {
                serde_json::to_string(&payload)
            };
            println!(
                "{}",
                formatted.unwrap_or_else(|e| format!(r#"{{"error":"{e}"}}"#))
            );
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("shot txt2vid: {e}");
            exit_for_backend_error(&e)
        }
    }
}

/// Analyze `dest` for freeze frames and trim in place when a non-
/// trivial amount of leading or trailing static is detected. Writes
/// the trimmed clip to a sibling temp path then renames over the
/// destination — atomic on POSIX, so a partial failure leaves the
/// original intact.
///
/// Returns a JSON summary that's embedded in the txt2vid payload so
/// the agent and the trace see exactly what got trimmed.
fn auto_trim_in_place(dest: &std::path::Path) -> serde_json::Value {
    use crate::clip::trim_static::{analyze, apply_trim, DetectParams, MIN_MOTION_SECS};
    let params = DetectParams::default();
    let report = match analyze(dest, params) {
        Ok(r) => r,
        Err(e) => {
            return serde_json::json!({
                "applied": false,
                "reason": "analyze_failed",
                "error": e,
            });
        }
    };
    if report.unusable {
        // Surface the failure but DON'T trim — the agent should see
        // the report and decide whether to re-roll the clip.
        return serde_json::json!({
            "applied": false,
            "reason": "motion_too_short",
            "report": report,
            "hint": format!(
                "motion span {:.2}s < {:.2}s minimum — re-roll the clip",
                report.motion_duration_s, MIN_MOTION_SECS
            ),
        });
    }
    let static_total = report.leading_freeze_s + report.trailing_freeze_s;
    if static_total < 0.1 {
        return serde_json::json!({
            "applied": false,
            "reason": "no_static_detected",
            "report": report,
        });
    }
    let tmp = dest.with_extension("trim.tmp.mp4");
    if let Err(e) = apply_trim(dest, &tmp, report.trim_start_s, report.trim_end_s) {
        let _ = std::fs::remove_file(&tmp);
        return serde_json::json!({
            "applied": false,
            "reason": "apply_trim_failed",
            "error": e,
            "report": report,
        });
    }
    if let Err(e) = std::fs::rename(&tmp, dest) {
        let _ = std::fs::remove_file(&tmp);
        return serde_json::json!({
            "applied": false,
            "reason": "rename_failed",
            "error": e.to_string(),
            "report": report,
        });
    }
    serde_json::json!({
        "applied": true,
        "trimmed_leading_s": report.leading_freeze_s,
        "trimmed_trailing_s": report.trailing_freeze_s,
        "kept_motion_s": report.motion_duration_s,
        "input_duration_s": report.input_duration_s,
    })
}

#[cfg(test)]
mod tests {
    //! Exit-code regression tests for `shot txt2vid` — locks in the
    //! convention that `2` is reserved for clap arg-parse errors and
    //! any post-parse failure routes to `1` (generic runtime) or
    //! `3` (post-parse hard fail like missing credentials / cost gate).
    //!
    //! Eval 010 surfaced the original bug: the `fal-veo3-ref-fast`
    //! codepath was returning exit `2` for both missing-credential
    //! and HTTP-status backend errors, which collides with clap's
    //! parse-error code and made the eval driver unable to tell
    //! "I called the tool wrong" from "the backend pushed back".
    use super::*;
    use crate::backends::BackendError;

    #[test]
    fn missing_credential_maps_to_post_parse_hard_fail() {
        let err = BackendError::MissingCredential("FAL_KEY".into());
        let code = exit_for_backend_error(&err);
        assert_eq!(format!("{:?}", code), format!("{:?}", ExitCode::from(3)));
    }

    #[test]
    fn over_budget_maps_to_post_parse_hard_fail() {
        let err = BackendError::OverBudget {
            estimate: 1.50,
            budget: 1.20,
        };
        let code = exit_for_backend_error(&err);
        assert_eq!(format!("{:?}", code), format!("{:?}", ExitCode::from(3)));
    }

    #[test]
    fn invalid_request_maps_to_post_parse_hard_fail() {
        let err = BackendError::InvalidRequest("prompt empty".into());
        let code = exit_for_backend_error(&err);
        assert_eq!(format!("{:?}", code), format!("{:?}", ExitCode::from(3)));
    }

    #[test]
    fn http_status_maps_to_generic_runtime_error() {
        let err = BackendError::HttpStatus {
            status: 422,
            body: "prompt rejected".into(),
        };
        let code = exit_for_backend_error(&err);
        assert_eq!(format!("{:?}", code), format!("{:?}", ExitCode::from(1)));
    }

    #[test]
    fn transport_maps_to_generic_runtime_error() {
        let err = BackendError::Transport("connection refused".into());
        let code = exit_for_backend_error(&err);
        assert_eq!(format!("{:?}", code), format!("{:?}", ExitCode::from(1)));
    }

    #[test]
    fn decode_maps_to_generic_runtime_error() {
        let err = BackendError::Decode("bad json".into());
        let code = exit_for_backend_error(&err);
        assert_eq!(format!("{:?}", code), format!("{:?}", ExitCode::from(1)));
    }

    #[test]
    fn no_branch_returns_clap_collision_code_two() {
        // Belt-and-braces: every BackendError variant must route to
        // either 1 or 3 — never 2. If a new variant is added and
        // someone forgets to update `exit_for_backend_error`, the
        // match becomes non-exhaustive and the compiler catches it.
        // This test additionally proves no current variant resolves
        // to ExitCode::from(2).
        let cases = [
            BackendError::Unimplemented("foo"),
            BackendError::MissingCredential("X".into()),
            BackendError::OverBudget { estimate: 1.0, budget: 0.5 },
            BackendError::Transport("net".into()),
            BackendError::HttpStatus { status: 500, body: "".into() },
            BackendError::Decode("d".into()),
            BackendError::Cache("c".into()),
            BackendError::InvalidRequest("i".into()),
        ];
        let two = format!("{:?}", ExitCode::from(2));
        for err in &cases {
            let code = exit_for_backend_error(err);
            assert_ne!(
                format!("{:?}", code),
                two,
                "BackendError {err:?} routed to exit 2 (reserved for clap parse errors)",
            );
        }
    }

    /// Regression for eval 010: dry-run path for `fal-veo3-ref-fast`
    /// must return `ExitCode::SUCCESS`. This exercises the full
    /// `run_shot_txt2vid` → `run_fal_veo_ref` → `FalVeoRefAdapter`
    /// dry-run flow without any network calls.
    #[test]
    fn dry_run_fal_veo_ref_fast_returns_success() {
        let tmp = std::env::temp_dir().join("wavelet-shot-txt2vid-dryrun-test");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        let ref_path = tmp.join("ref.png");
        // Minimal 1x1 PNG bytes — enough for the dry-run path which
        // never actually uploads, but the file must exist if the
        // upload step were reached.
        std::fs::write(
            &ref_path,
            [
                0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48,
                0x44, 0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00,
                0x00, 0x1F, 0x15, 0xC4, 0x89, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x44, 0x41, 0x54, 0x78,
                0x9C, 0x62, 0x00, 0x01, 0x00, 0x00, 0x05, 0x00, 0x01, 0x0D, 0x0A, 0x2D, 0xB4, 0x00,
                0x00, 0x00, 0x00, 0x49, 0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82,
            ],
        )
        .unwrap();

        let code = run_shot_txt2vid(
            "a woman holds a serum bottle".into(),
            Some("fal-veo3-ref-fast".into()),
            4.0,
            "9:16".into(),
            None,
            false,
            None,
            1,
            "best".into(),
            None,
            /* dry_run */ true,
            /* max_cost */ 5.0,
            tmp.clone(),
            /* out */ None,
            /* pretty */ true,
            /* no_trim_static */ true,
            vec![ref_path],
        );
        assert_eq!(
            format!("{:?}", code),
            format!("{:?}", ExitCode::SUCCESS),
            "dry-run fal-veo3-ref-fast must return ExitCode::SUCCESS (eval-010 regression)"
        );

        let _ = std::fs::remove_dir_all(&tmp);
    }

    /// Companion regression: dry-run of the text-only Fal Veo path
    /// must also exit success. Covers `run_fal_veo` (non-ref) — the
    /// same path that the eval driver fell back to and that actually
    /// produced the on-disk MP4s.
    #[test]
    fn dry_run_fal_veo_fast_returns_success() {
        let tmp = std::env::temp_dir().join("wavelet-shot-txt2vid-dryrun-text-test");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();

        let code = run_shot_txt2vid(
            "a woman holds a serum bottle".into(),
            Some("fal-veo3-fast".into()),
            4.0,
            "9:16".into(),
            None,
            false,
            None,
            1,
            "best".into(),
            None,
            /* dry_run */ true,
            /* max_cost */ 5.0,
            tmp.clone(),
            /* out */ None,
            /* pretty */ false,
            /* no_trim_static */ true,
            vec![],
        );
        assert_eq!(
            format!("{:?}", code),
            format!("{:?}", ExitCode::SUCCESS),
            "dry-run fal-veo3-fast must return ExitCode::SUCCESS"
        );

        let _ = std::fs::remove_dir_all(&tmp);
    }
}
