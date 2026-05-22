//! `wavelet music gen` handler — reference-conditioned music generation.

use std::process::ExitCode;

use wavelet::backends::elevenlabs::{ElevenLabsClient, ElevenLabsMusicAdapter};
use wavelet::backends::google::{GoogleAiClient, GoogleLyriaAdapter, LyriaModel};
use wavelet::backends::music::{RefConditionedMusicGenBackend, RefConditionedMusicRequest};
use wavelet::backends::udio::UdioMusicAdapter;
use wavelet::backends::{BackendError, RunMode};
use wavelet::velocity::VelocityProfile;

use super::super::MusicOp;

/// Dispatch entrypoint.
pub fn run(op: MusicOp) -> ExitCode {
    match op {
        MusicOp::Gen {
            prompt,
            velocity,
            style,
            duration,
            bpm,
            backend,
            variant,
            seed,
            dry_run,
            max_cost,
            cache,
            out,
            pretty,
        } => {
            let backend = wavelet::config::resolve_backend(
                wavelet::config::BackendKind::Music,
                backend.as_deref(),
            );
            let mut req = match (velocity.as_ref(), prompt.as_ref(), duration) {
                (Some(v_path), _, _) => {
                    let v: VelocityProfile = match std::fs::read_to_string(v_path)
                        .map_err(|e| format!("read {}: {e}", v_path.display()))
                        .and_then(|s| {
                            serde_json::from_str(&s)
                                .map_err(|e| format!("parse velocity: {e}"))
                        }) {
                        Ok(v) => v,
                        Err(e) => {
                            eprintln!("{e}");
                            return ExitCode::from(2);
                        }
                    };
                    let mut r = RefConditionedMusicRequest::from_velocity(&v, &style);
                    if let Some(d) = duration {
                        r.duration_secs = d;
                    }
                    if let Some(p) = prompt.as_ref() {
                        r.prompt = p.clone();
                    }
                    r
                }
                (None, Some(p), Some(d)) => RefConditionedMusicRequest::new(p, d),
                _ => {
                    eprintln!("supply either --velocity <path>, or both --prompt and --duration.");
                    return ExitCode::from(3);
                }
            };
            if let Some(b) = bpm {
                req.bpm = Some(b);
            }
            if variant.is_some() {
                req.model_variant = variant;
            }
            if let Some(s) = seed {
                req.seed = Some(s);
            }

            let mode = if dry_run {
                RunMode::DryRun
            } else {
                RunMode::Live {
                    max_cost_usd: max_cost,
                }
            };

            let outcome = match backend.as_str() {
                "google-lyria" | "google-lyria-3-pro" | "lyria" | "lyria-pro"
                | "google-lyria-3-clip" | "lyria-clip" => {
                    let model = match LyriaModel::parse(&backend) {
                        Ok(m) => m,
                        Err(e) => {
                            eprintln!("{e}");
                            return ExitCode::from(3);
                        }
                    };
                    let client = if dry_run {
                        GoogleAiClient::with_key("dry-run", &cache)
                    } else {
                        match GoogleAiClient::from_env(&cache) {
                            Ok(c) => c,
                            Err(e) => {
                                eprintln!("backend {backend}: {e}");
                                if let BackendError::MissingCredential(name) = &e {
                                    eprintln!("set {name} or pass --dry-run to preview.");
                                }
                                return ExitCode::from(2);
                            }
                        }
                    };
                    let adapter = GoogleLyriaAdapter::new(client, model);
                    adapter.generate(&req, mode)
                }
                "elevenlabs" | "elevenlabs-music" => {
                    let client = if dry_run {
                        ElevenLabsClient::with_key("dry-run", &cache)
                    } else {
                        match ElevenLabsClient::from_env(&cache) {
                            Ok(c) => c,
                            Err(e) => {
                                eprintln!("backend elevenlabs: {e}");
                                if let BackendError::MissingCredential(name) = &e {
                                    eprintln!(
                                        "set {name} or pass --dry-run to preview. \
                                         Note: the key must carry the `music_generation` \
                                         permission — TTS-only keys 401."
                                    );
                                }
                                return ExitCode::from(2);
                            }
                        }
                    };
                    let adapter = ElevenLabsMusicAdapter::new(client);
                    adapter.generate(&req, mode)
                }
                "udio" => {
                    let adapter = UdioMusicAdapter::new(&cache);
                    adapter.generate(&req, mode)
                }
                other => {
                    eprintln!(
                        "unknown --backend '{other}', want \
                         google-lyria|google-lyria-3-pro|google-lyria-3-clip|\
                         elevenlabs|udio"
                    );
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
                                return ExitCode::from(2);
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
                    eprintln!("music gen: {e}");
                    ExitCode::from(2)
                }
            }
        }
    }
}
