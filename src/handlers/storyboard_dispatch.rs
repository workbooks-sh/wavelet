use std::process::ExitCode;
use crate::cli_args::StoryboardOp;
use crate::handlers::util::parse_resolution;

/// (auto-generated placeholder)
pub fn run_storyboard(op: StoryboardOp) -> ExitCode {
    use crate::storyboard::{
        plan_from_screenplay_full, verify_storyboard, StoryboardLevel,
    };
    match op {
        StoryboardOp::Plan {
            screenplay,
            velocity,
            fps,
            resolution,
            aspect,
            pretty,
            out,
            onsets,
            no_snap,
            match_runtime,
            workdir,
            no_characters,
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
            let v: crate::velocity::VelocityProfile = match std::fs::read_to_string(&velocity)
                .map_err(|e| format!("read {}: {e}", velocity.display()))
                .and_then(|s| serde_json::from_str(&s).map_err(|e| format!("parse velocity: {e}")))
            {
                Ok(v) => v,
                Err(e) => {
                    eprintln!("{e}");
                    return ExitCode::from(2);
                }
            };
            let res = match aspect.as_deref() {
                Some(a) => match crate::aspect::AspectRatio::parse(a) {
                    Some(ar) => {
                        let (w, h) = ar.dimensions(720);
                        [w, h]
                    }
                    None => {
                        eprintln!(
                            "invalid --aspect '{a}', want one of 16:9|9:16|1:1|4:5|21:9"
                        );
                        return ExitCode::from(3);
                    }
                },
                None => match parse_resolution(&resolution) {
                    Some(r) => r,
                    None => {
                        eprintln!("invalid --resolution '{resolution}', want WxH");
                        return ExitCode::from(3);
                    }
                },
            };
            let onset_times: Option<Vec<f32>> = match (&onsets, no_snap) {
                (Some(path), false) => {
                    match crate::audio::DecodedAudio::decode(path) {
                        Ok(audio) => {
                            let onset_ms = crate::query::beat::detect_onsets_interleaved(
                                &audio.samples,
                                audio.sample_rate,
                            );
                            Some(onset_ms.iter().map(|&m| m as f32 / 1000.0).collect())
                        }
                        Err(e) => {
                            eprintln!("decode {}: {e}", path.display());
                            return ExitCode::from(2);
                        }
                    }
                }
                _ => None,
            };
            // Auto-load character refs from <workdir>/refs/character/.
            // Workdir defaults to the screenplay's parent directory so
            // legacy callers don't need to pass `--workdir` explicitly.
            let resolved_workdir = workdir.clone().or_else(|| {
                screenplay.parent().map(|p| p.to_path_buf())
            });
            let characters = if no_characters {
                None
            } else {
                resolved_workdir.as_ref().and_then(|wd| {
                    match crate::clipref::character::load_characters(wd) {
                        Ok(map) if !map.is_empty() => {
                            eprintln!(
                                "loaded {} character ref(s) from {}",
                                map.len(),
                                wd.join("refs/character").display(),
                            );
                            Some(map)
                        }
                        Ok(_) => None,
                        Err(e) => {
                            eprintln!(
                                "warn: failed to load character refs from {}: {e}",
                                wd.display(),
                            );
                            None
                        }
                    }
                })
            };
            let mut sb = plan_from_screenplay_full(
                &s,
                &v,
                screenplay.display().to_string(),
                velocity.display().to_string(),
                fps,
                res,
                onset_times.as_deref(),
                characters.as_ref(),
            );
            if let Some(target) = match_runtime {
                crate::storyboard::plan::match_runtime(&mut sb, target);
            }
            let json = if pretty {
                serde_json::to_string_pretty(&sb)
            } else {
                serde_json::to_string(&sb)
            };
            match (json, out) {
                (Ok(j), Some(p)) => {
                    if let Err(e) = std::fs::write(&p, j) {
                        eprintln!("write {}: {e}", p.display());
                        return ExitCode::from(2);
                    }
                    eprintln!(
                        "wrote {} â {} scenes, {} shots, {:.1}s",
                        p.display(),
                        sb.scenes.len(),
                        sb.shots.len(),
                        sb.duration_secs,
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
        StoryboardOp::Verify { storyboard, json } => {
            let src = match std::fs::read_to_string(&storyboard) {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("read {}: {e}", storyboard.display());
                    return ExitCode::from(2);
                }
            };
            let sb: crate::storyboard::Storyboard = match serde_json::from_str(&src) {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("parse {}: {e}", storyboard.display());
                    return ExitCode::from(2);
                }
            };
            let findings = verify_storyboard(&sb);
            let mut errors = 0usize;
            let mut warnings = 0usize;
            for f in &findings {
                match f.level {
                    StoryboardLevel::Error => errors += 1,
                    StoryboardLevel::Warning => warnings += 1,
                }
            }
            if json {
                let report = serde_json::json!({
                    "ok": errors == 0,
                    "errors": errors,
                    "warnings": warnings,
                    "findings": findings,
                });
                println!("{}", serde_json::to_string_pretty(&report).unwrap_or_default());
            } else {
                for f in &findings {
                    let prefix = match f.level {
                        StoryboardLevel::Error => "ERROR",
                        StoryboardLevel::Warning => "WARN ",
                    };
                    println!("{prefix}  [{}] {}", f.origin, f.message);
                }
                println!();
                if findings.is_empty() {
                    println!("â clean: 0 errors, 0 warnings");
                } else if errors == 0 {
                    println!("â {} warning(s), 0 errors", warnings);
                } else {
                    println!("â {} error(s), {} warning(s)", errors, warnings);
                }
            }
            if errors > 0 {
                ExitCode::from(1)
            } else {
                ExitCode::SUCCESS
            }
        }
    }
}
