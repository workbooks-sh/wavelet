//! `wavelet dialogue { tts | captions }` handler.

use std::path::PathBuf;
use std::process::ExitCode;

use wavelet::backends::elevenlabs::{ElevenLabsClient, ElevenLabsTtsAdapter};
use wavelet::backends::tts::{TtsRequest, VoiceIdTtsBackend};
use wavelet::backends::{exit_for_backend_error, BackendError, RunMode};

use super::super::DialogueOp;

/// Dispatch entrypoint.
pub fn run(op: DialogueOp) -> ExitCode {
    match op {
        DialogueOp::Tts {
            text,
            voice,
            backend,
            model,
            stability,
            similarity,
            style,
            dry_run,
            max_cost,
            cache,
            out,
            pretty,
        } => {
            let backend = wavelet::config::resolve_backend(
                wavelet::config::BackendKind::Tts,
                backend.as_deref(),
            );
            run_tts(
                text, voice, backend, model, stability, similarity, style, dry_run, max_cost,
                cache, out, pretty,
            )
        }
        DialogueOp::Captions {
            audio,
            text,
            backend,
            duration_ms,
            style,
            dry_run,
            max_cost,
            cache,
            out,
            pretty,
        } => run_captions(
            audio,
            text,
            backend,
            duration_ms,
            style,
            dry_run,
            max_cost,
            cache,
            out,
            pretty,
        ),
    }
}

#[allow(clippy::too_many_arguments)]
fn run_tts(
    text: String,
    voice: String,
    backend: String,
    model: Option<String>,
    stability: Option<f32>,
    similarity: Option<f32>,
    style: Option<f32>,
    dry_run: bool,
    max_cost: f32,
    cache: PathBuf,
    out: Option<PathBuf>,
    pretty: bool,
) -> ExitCode {
    let req = TtsRequest {
        text,
        voice_id: voice,
        model,
        stability,
        similarity_boost: similarity,
        style,
    };
    let mode = if dry_run {
        RunMode::DryRun
    } else {
        RunMode::Live {
            max_cost_usd: max_cost,
        }
    };

    let outcome = match backend.as_str() {
        "elevenlabs" => {
            let client = if dry_run {
                ElevenLabsClient::with_key("dry-run", &cache)
            } else {
                match ElevenLabsClient::from_env(&cache) {
                    Ok(c) => c,
                    Err(e) => {
                        eprintln!("backend elevenlabs: {e}");
                        if let BackendError::MissingCredential(name) = &e {
                            eprintln!("set {name} or pass --dry-run to preview.");
                        }
                        return exit_for_backend_error(&e);
                    }
                }
            };
            let adapter = ElevenLabsTtsAdapter::new(client);
            adapter.synthesize(&req, mode)
        }
        "fal-kokoro" => {
            use wavelet::backends::fal::{FalClient, FalKokoroAdapter};
            let client = if dry_run {
                FalClient::with_key("dry-run", &cache)
            } else {
                match FalClient::from_env(&cache) {
                    Ok(c) => c,
                    Err(e) => {
                        eprintln!("backend fal-kokoro: {e}");
                        if let BackendError::MissingCredential(name) = &e {
                            eprintln!("set {name} or pass --dry-run to preview.");
                        }
                        return exit_for_backend_error(&e);
                    }
                }
            };
            let adapter = FalKokoroAdapter::new(client);
            adapter.synthesize(&req, mode)
        }
        "gemini-tts" | "google-gemini-tts" | "gemini-3.1-flash-tts-preview" => {
            use wavelet::backends::google::{GeminiTtsAdapter, GoogleAiClient};
            let client = if dry_run {
                GoogleAiClient::with_key("dry-run", &cache)
            } else {
                match GoogleAiClient::from_env(&cache) {
                    Ok(c) => c,
                    Err(e) => {
                        eprintln!("backend gemini-tts: {e}");
                        if let BackendError::MissingCredential(name) = &e {
                            eprintln!("set {name} or pass --dry-run to preview.");
                        }
                        return exit_for_backend_error(&e);
                    }
                }
            };
            let adapter = GeminiTtsAdapter::new(client);
            adapter.synthesize(&req, mode)
        }
        other => {
            eprintln!("unknown --backend '{other}', want elevenlabs|fal-kokoro|gemini-tts");
            return ExitCode::from(3);
        }
    };

    match outcome {
        Ok(outcome) => {
            if let Some(dest) = out.as_ref() {
                if outcome.response.audio_bytes > 0 {
                    if let Err(e) = std::fs::copy(&outcome.response.audio_path, dest) {
                        eprintln!(
                            "copy {} → {}: {e}",
                            outcome.response.audio_path.display(),
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
            eprintln!("dialogue tts: {e}");
            exit_for_backend_error(&e)
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn run_captions(
    audio: String,
    text: String,
    backend: String,
    duration_ms: u32,
    style: Option<String>,
    dry_run: bool,
    max_cost: f32,
    cache: PathBuf,
    out: Option<PathBuf>,
    pretty: bool,
) -> ExitCode {
    use wavelet::backends::captions::{CaptionsBackend, CaptionsRequest, SyntheticEqualPacingAdapter};
    use wavelet::backends::fal::{FalClient, FalWhisperWordsAdapter};

    let mut req = CaptionsRequest::new(audio, text);
    req.duration_ms = duration_ms;

    let mode = if dry_run {
        RunMode::DryRun
    } else {
        RunMode::Live {
            max_cost_usd: max_cost,
        }
    };

    let outcome = match backend.as_str() {
        "fal-whisper-words" => {
            let client = if dry_run {
                FalClient::with_key("dry-run", &cache)
            } else {
                match FalClient::from_env(&cache) {
                    Ok(c) => c,
                    Err(e) => {
                        eprintln!("backend fal-whisper-words: {e}");
                        if let BackendError::MissingCredential(name) = &e {
                            eprintln!("set {name} or pass --dry-run to preview.");
                        }
                        return exit_for_backend_error(&e);
                    }
                }
            };
            let adapter = FalWhisperWordsAdapter::new(client);
            adapter.captions(&req, mode)
        }
        "synthetic" => SyntheticEqualPacingAdapter::new().captions(&req, mode),
        other => {
            eprintln!("unknown --backend '{other}', want fal-whisper-words|synthetic");
            return ExitCode::from(3);
        }
    };

    match outcome {
        Ok(outcome) => {
            let payload = serde_json::json!({
                "mode": outcome.mode,
                "provider": outcome.provider,
                "request_hash": outcome.request_hash,
                "cached": outcome.cached,
                "cost_estimate_usd": outcome.cost_estimate_usd,
                "style": style,
                "result": outcome.response,
            });
            let formatted = if pretty {
                serde_json::to_string_pretty(&payload)
            } else {
                serde_json::to_string(&payload)
            };
            let text = formatted.unwrap_or_else(|e| format!(r#"{{"error":"{e}"}}"#));
            if let Some(path) = out.as_ref() {
                if let Err(e) = std::fs::write(path, &text) {
                    eprintln!("write {}: {e}", path.display());
                    // Generic runtime I/O failure → 1, not 2.
                    return ExitCode::from(1);
                }
            } else {
                println!("{text}");
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("dialogue captions: {e}");
            exit_for_backend_error(&e)
        }
    }
}
