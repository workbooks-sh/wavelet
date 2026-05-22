//! `query.shader` tool (wb-mxrk.8) — exposes the five starter shader
//! assertions to the agent. In-process: dispatches via
//! `validators::shader::run_named_shader` (the same path the Plan
//! validator uses), so the CLI / validator / tool all return the same
//! JSON envelope.
//!
//! Ordering discipline (per the methodology doc §7.1): `query.shader`
//! is cheap pixel/composition; place it after `query.scene_graph` and
//! before `rubric_passes` when ordering validator steps in a Plan.

use std::path::PathBuf;

use serde_json::{json, Value};

use crate::agent::plan::validators::shader::{run_named_shader, KNOWN_SHADERS};

use super::{Tool, ToolRegistry, ToolResult};

/// Register the `query.shader` tool on a registry.
pub fn register(r: &mut ToolRegistry) {
    r.register(QueryShaderTool);
}

/// Agent-facing wrapper around `run_named_shader`.
pub struct QueryShaderTool;

impl Tool for QueryShaderTool {
    fn name(&self) -> &str {
        "query.shader"
    }

    fn description(&self) -> &str {
        "Run a GPU shader assertion against a frame (contrast, motion, \
         golden RMSE, edge density, color band). Returns \
         {pass, score, metrics, shader, elapsed_us}."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "required": ["shader", "frame"],
            "properties": {
                "shader": {
                    "type": "string",
                    "enum": KNOWN_SHADERS,
                    "description": "Which assertion shader to run."
                },
                "frame": {
                    "type": "string",
                    "description": "Path to the PNG frame to assert against."
                },
                "params": {
                    "type": "object",
                    "description": "Shader-specific params (region, thresholds, sibling-frame paths)."
                },
                "on_frame": {
                    "type": "string",
                    "description": "Reserved for v2 multi-frame dispatch; ignored in v1."
                }
            }
        })
    }

    fn dispatch(&self, args: &Value) -> ToolResult {
        let Some(shader) = args.get("shader").and_then(|v| v.as_str()) else {
            return ToolResult::local_err(self.name(), "missing `shader`");
        };
        if !KNOWN_SHADERS.contains(&shader) {
            let detail = json!({
                "error": "unknown_shader",
                "shader": shader,
                "available": KNOWN_SHADERS,
            });
            return ToolResult {
                ok: false,
                response: detail,
                summary: format!("{}: unknown_shader '{shader}'", self.name()),
                output_files: Vec::new(),
                cost_usd: 0.0,
            };
        }
        let Some(frame_str) = args.get("frame").and_then(|v| v.as_str()) else {
            return ToolResult::local_err(self.name(), "missing `frame`");
        };
        let frame_path = PathBuf::from(frame_str);
        let params = args.get("params").cloned().unwrap_or(json!({}));

        match run_named_shader(shader, &frame_path, &params) {
            Ok(res) => {
                let summary = format!(
                    "{}: shader={shader} pass={} score={:.4}",
                    self.name(),
                    res.pass,
                    res.score
                );
                ToolResult {
                    ok: res.pass,
                    response: res.to_json(),
                    summary,
                    output_files: Vec::new(),
                    cost_usd: 0.0,
                }
            }
            Err(e) => {
                let detail = json!({
                    "error": "dispatch_failed",
                    "shader": shader,
                    "frame": frame_str,
                    "reason": e.to_string(),
                });
                ToolResult {
                    ok: false,
                    response: detail,
                    summary: format!("{}: dispatch_failed: {e}", self.name()),
                    output_files: Vec::new(),
                    cost_usd: 0.0,
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::Path;

    fn write_split_png(path: &Path, w: u32, h: u32, left: [u8; 4], right: [u8; 4]) {
        let mut pixels = Vec::with_capacity((w * h * 4) as usize);
        for _y in 0..h {
            for x in 0..w {
                let px = if x < w / 2 { left } else { right };
                pixels.extend_from_slice(&px);
            }
        }
        let file = fs::File::create(path).unwrap();
        let mut enc = png::Encoder::new(file, w, h);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        let mut writer = enc.write_header().unwrap();
        writer.write_image_data(&pixels).unwrap();
        writer.finish().unwrap();
    }

    #[test]
    fn schema_lists_all_known_shaders() {
        let t = QueryShaderTool;
        let s = t.parameters_schema();
        let enum_v = s["properties"]["shader"]["enum"].as_array().unwrap();
        assert_eq!(enum_v.len(), KNOWN_SHADERS.len());
        for name in KNOWN_SHADERS {
            assert!(
                enum_v.iter().any(|v| v.as_str() == Some(name)),
                "missing {name}"
            );
        }
    }

    #[test]
    fn dispatch_unknown_shader_returns_structured_error() {
        let t = QueryShaderTool;
        let r = t.dispatch(&json!({
            "shader": "not_a_real_shader",
            "frame": "x.png",
        }));
        assert!(!r.ok);
        assert_eq!(r.response["error"], json!("unknown_shader"));
        assert!(r.response["available"].is_array());
    }

    #[test]
    fn dispatch_missing_params_errors() {
        let t = QueryShaderTool;
        let r = t.dispatch(&json!({ "shader": "contrast_in_region" }));
        assert!(!r.ok);
        assert!(r.response["error"].as_str().unwrap().contains("frame"));
    }

    #[test]
    fn dispatch_contrast_passes_on_black_white_split() {
        let dir = tempfile::tempdir().unwrap();
        let png = dir.path().join("frame.png");
        write_split_png(&png, 16, 16, [0, 0, 0, 255], [255, 255, 255, 255]);

        let t = QueryShaderTool;
        let r = t.dispatch(&json!({
            "shader": "contrast_in_region",
            "frame": png.to_string_lossy(),
            "params": {
                "region": [0.0, 0.0, 1.0, 1.0],
                "min_contrast": 1.0
            }
        }));
        assert!(r.ok, "response={:?}", r.response);
        assert_eq!(r.response["pass"], json!(true));
        assert_eq!(r.response["shader"], json!("contrast_in_region"));
        assert!(r.response["metrics"].is_array());
        assert!(r.response["elapsed_us"].is_u64());
    }

    #[test]
    fn dispatch_contrast_fails_on_near_gray() {
        let dir = tempfile::tempdir().unwrap();
        let png = dir.path().join("frame.png");
        write_split_png(&png, 16, 16, [120, 120, 120, 255], [132, 132, 132, 255]);

        let t = QueryShaderTool;
        let r = t.dispatch(&json!({
            "shader": "contrast_in_region",
            "frame": png.to_string_lossy(),
            "params": {
                "region": [0.0, 0.0, 1.0, 1.0],
                "min_contrast": 4.5
            }
        }));
        assert!(!r.ok);
        assert_eq!(r.response["pass"], json!(false));
        assert!(r.response.get("reason").is_some());
    }
}
