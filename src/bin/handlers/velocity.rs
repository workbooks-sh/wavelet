//! `wavelet velocity` handler — propose / validate / render / onsets-to-edl.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use wavelet::velocity::{
    detect_onsets_ms, onsets_to_edl, propose_from_screenplay, render_curve_svg,
    validate_against_bpm,
};

use super::super::VelocityOp;

/// Dispatch entrypoint.
pub fn run(op: VelocityOp) -> ExitCode {
    match op {
        VelocityOp::Propose { screenplay, out, pretty } => {
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
            let profile = propose_from_screenplay(&s);
            let json = if pretty {
                serde_json::to_string_pretty(&profile)
            } else {
                serde_json::to_string(&profile)
            };
            match (json, out) {
                (Ok(j), Some(p)) => {
                    if let Err(e) = std::fs::write(&p, j) {
                        eprintln!("write {}: {e}", p.display());
                        return ExitCode::from(2);
                    }
                    eprintln!("wrote {}", p.display());
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
        VelocityOp::Validate {
            profile,
            against,
            tolerance,
            window,
            pretty,
            fps,
            no_emit_edl,
        } => {
            let p = match read_profile(&profile) {
                Ok(p) => p,
                Err(e) => {
                    eprintln!("load profile: {e}");
                    return ExitCode::from(2);
                }
            };
            let report = match validate_against_bpm(&p, &against, tolerance, window) {
                Ok(r) => r,
                Err(e) => {
                    eprintln!("validate: {e}");
                    return ExitCode::from(2);
                }
            };
            let json = if pretty {
                serde_json::to_string_pretty(&report).unwrap_or_default()
            } else {
                serde_json::to_string(&report).unwrap_or_default()
            };
            println!("{json}");

            if !no_emit_edl && !report.detected_onsets_ms.is_empty() {
                let duration_ms = (p.duration_secs * 1000.0) as u32;
                let edl = onsets_to_edl(
                    &report.detected_onsets_ms,
                    fps,
                    "wavelet-cuts",
                    Some(duration_ms),
                );
                let edl_path = sibling_edl_path(&against);
                if let Err(e) = std::fs::write(&edl_path, edl) {
                    eprintln!("write {}: {e}", edl_path.display());
                } else {
                    eprintln!("wrote {}", edl_path.display());
                }
            }

            if report.ok {
                ExitCode::SUCCESS
            } else {
                ExitCode::from(1)
            }
        }
        VelocityOp::OnsetsToEdl { music, fps, format, out } => {
            if format != "edl" {
                eprintln!(
                    "format `{format}` not implemented yet; only `edl` is supported. \
                     File a follow-up for fcpxml / premiere-marker-csv."
                );
                return ExitCode::from(2);
            }
            let onsets = match detect_onsets_ms(&music) {
                Ok(o) => o,
                Err(e) => {
                    eprintln!("detect onsets: {e}");
                    return ExitCode::from(2);
                }
            };
            let edl = onsets_to_edl(&onsets, fps, "wavelet-cuts", None);
            match out {
                Some(p) => {
                    if let Err(e) = std::fs::write(&p, edl) {
                        eprintln!("write {}: {e}", p.display());
                        return ExitCode::from(2);
                    }
                    eprintln!("wrote {} ({} onsets)", p.display(), onsets.len());
                }
                None => println!("{edl}"),
            }
            ExitCode::SUCCESS
        }
        VelocityOp::RenderCurve { profile, overlay, out } => {
            let p = match read_profile(&profile) {
                Ok(p) => p,
                Err(e) => {
                    eprintln!("load profile: {e}");
                    return ExitCode::from(2);
                }
            };
            let overlay_report = match overlay.as_ref() {
                Some(path) => match std::fs::read_to_string(path).and_then(|s| {
                    serde_json::from_str(&s).map_err(|e| {
                        std::io::Error::new(std::io::ErrorKind::InvalidData, e)
                    })
                }) {
                    Ok(r) => Some(r),
                    Err(e) => {
                        eprintln!("load overlay {}: {e}", path.display());
                        return ExitCode::from(2);
                    }
                },
                None => None,
            };
            let svg = render_curve_svg(&p, overlay_report.as_ref());
            match out {
                Some(p) => {
                    if let Err(e) = std::fs::write(&p, svg) {
                        eprintln!("write {}: {e}", p.display());
                        return ExitCode::from(2);
                    }
                    eprintln!("wrote {}", p.display());
                }
                None => println!("{svg}"),
            }
            ExitCode::SUCCESS
        }
    }
}

fn read_profile(path: &PathBuf) -> Result<wavelet::velocity::VelocityProfile, String> {
    let src = std::fs::read_to_string(path).map_err(|e| format!("read {}: {e}", path.display()))?;
    serde_json::from_str(&src).map_err(|e| format!("parse {}: {e}", path.display()))
}

fn sibling_edl_path(audio: &Path) -> PathBuf {
    let mut p = audio.to_path_buf();
    let stem = p
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("velocity")
        .to_owned();
    p.set_file_name(format!("{stem}.cuts.edl"));
    p
}
