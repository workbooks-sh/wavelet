//! `wavelet captions overlay` — word-level caption HTML generator.

use std::process::ExitCode;

use wavelet::backends::captions::{render_overlay_html, CaptionsResult, OverlayConfig, OverlayStyle};

use super::super::CaptionsOp;

/// Dispatch entrypoint.
pub fn run(op: CaptionsOp) -> ExitCode {
    match op {
        CaptionsOp::Overlay {
            r#in,
            style,
            duration,
            width,
            height,
            out,
        } => {
            let raw = match std::fs::read_to_string(&r#in) {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("read {}: {e}", r#in.display());
                    return ExitCode::from(2);
                }
            };
            let value: serde_json::Value = match serde_json::from_str(&raw) {
                Ok(v) => v,
                Err(e) => {
                    eprintln!("parse captions JSON: {e}");
                    return ExitCode::from(2);
                }
            };
            let (result_val, embedded_style) = if value.get("result").is_some() {
                (
                    value.get("result").cloned().unwrap_or(serde_json::Value::Null),
                    value
                        .get("style")
                        .and_then(|s| s.as_str())
                        .map(|s| s.to_string()),
                )
            } else {
                (value, None)
            };
            let result: CaptionsResult = match serde_json::from_value(result_val) {
                Ok(r) => r,
                Err(e) => {
                    eprintln!("decode CaptionsResult: {e}");
                    return ExitCode::from(2);
                }
            };
            let style_str = if style == "hormozi" {
                embedded_style.unwrap_or(style)
            } else {
                style
            };
            let parsed_style = match OverlayStyle::from_str(&style_str) {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("{e}");
                    return ExitCode::from(3);
                }
            };
            let cfg = OverlayConfig {
                style: parsed_style,
                duration_ms: duration,
                width_px: width,
                height_px: height,
            };
            let html = render_overlay_html(&result, &cfg);
            if let Err(e) = std::fs::write(&out, html) {
                eprintln!("write {}: {e}", out.display());
                return ExitCode::from(2);
            }
            println!(
                "{{\"out\":\"{}\",\"style\":\"{}\"}}",
                out.display(),
                parsed_style.as_str()
            );
            ExitCode::SUCCESS
        }
    }
}
