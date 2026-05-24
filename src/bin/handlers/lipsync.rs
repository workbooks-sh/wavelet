//! `wavelet lipsync` handler — graft audio onto video.

use std::path::PathBuf;
use std::process::ExitCode;

use wavelet::backends::replicate::{ReplicateClient, ReplicateSyncLipSyncAdapter};
use wavelet::backends::video::{LipSyncBackend, LipSyncRequest};
use wavelet::backends::{exit_for_backend_error, BackendError, RunMode};

/// Dispatch entrypoint.
#[allow(clippy::too_many_arguments)]
pub fn run(
    video: String,
    audio: String,
    backend: String,
    sync_mode: Option<String>,
    temperature: Option<f32>,
    active_speaker: Option<bool>,
    dry_run: bool,
    max_cost: f32,
    cache: PathBuf,
    out: Option<PathBuf>,
    pretty: bool,
) -> ExitCode {
    let req = LipSyncRequest {
        video,
        audio,
        sync_mode,
        temperature,
        active_speaker,
    };
    let mode = if dry_run {
        RunMode::DryRun
    } else {
        RunMode::Live {
            max_cost_usd: max_cost,
        }
    };

    let outcome = match backend.as_str() {
        "sync-lipsync-2-pro" | "sync-2-pro" | "sync" | "replicate-sync-lipsync-2-pro" => {
            let client = if dry_run {
                ReplicateClient::with_token("dry-run", &cache)
            } else {
                match ReplicateClient::from_env(&cache) {
                    Ok(c) => c,
                    Err(e) => {
                        eprintln!("backend replicate-sync-lipsync-2-pro: {e}");
                        if let BackendError::MissingCredential(name) = &e {
                            eprintln!("set {name} or pass --dry-run to preview.");
                        }
                        return exit_for_backend_error(&e);
                    }
                }
            };
            let adapter = ReplicateSyncLipSyncAdapter::new(client);
            adapter.sync(&req, mode)
        }
        other => {
            eprintln!("unknown --backend '{other}', want sync-lipsync-2-pro|sync-2-pro|sync");
            return ExitCode::from(3);
        }
    };

    match outcome {
        Ok(outcome) => {
            if let Some(dest) = out.as_ref() {
                if outcome.response.video_bytes > 0 {
                    if let Err(e) = std::fs::copy(&outcome.response.video_path, dest) {
                        eprintln!(
                            "copy {} → {}: {e}",
                            outcome.response.video_path.display(),
                            dest.display()
                        );
                        // Generic runtime I/O failure → 1, not 2.
                        return ExitCode::from(1);
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
            eprintln!("lipsync: {e}");
            exit_for_backend_error(&e)
        }
    }
}
