//! `wavelet.image.*` tools — composite, isolate.

use serde_json::{json, Value};

use super::{
    arg_str as s, attach_out_file, push_flag as push, spawn_gamut, Tool, ToolRegistry, ToolResult,
};

pub fn register(r: &mut ToolRegistry) {
    r.register(Composite);
    r.register(Isolate);
}

pub struct Composite;
impl Tool for Composite {
    fn name(&self) -> &str { "wavelet.image.composite" }
    fn description(&self) -> &str {
        "Composite multiple input images into one (background + foreground layers)."
    }
    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "required": ["base", "out"],
            "properties": {
                "base": { "type": "string" },
                "overlay": { "type": "string" },
                "out": { "type": "string" },
                "mode": { "type": "string", "description": "normal | multiply | screen | overlay" },
                "opacity": { "type": "number" }
            }
        })
    }
    fn dispatch(&self, args: &Value) -> ToolResult {
        let base = match s(args, "base") {
            Some(v) => v, None => return ToolResult::local_err(self.name(), "missing `base`"),
        };
        let out = match s(args, "out") {
            Some(v) => v, None => return ToolResult::local_err(self.name(), "missing `out`"),
        };
        let mut cmd = vec![
            "image".into(), "composite".into(),
            "--base".into(), base,
            "--out".into(), out.clone(),
        ];
        push(&mut cmd, args, "overlay", "--overlay");
        push(&mut cmd, args, "mode", "--mode");
        push(&mut cmd, args, "opacity", "--opacity");
        match spawn_gamut(&cmd) {
            Ok(child) => {
                let mut r = ToolResult::from_subprocess(self.name(), child);
                attach_out_file(&mut r, &out);
                r
            }
            Err(e) => ToolResult::local_err(self.name(), e.to_string()),
        }
    }
}

pub struct Isolate;
impl Tool for Isolate {
    fn name(&self) -> &str { "wavelet.image.isolate" }
    fn description(&self) -> &str {
        "Text-prompted segmentation — isolate the named subject from the image. Required: `input` (image path), `prompt` (what to keep), `out`."
    }
    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "required": ["input", "prompt", "out"],
            "properties": {
                "input": { "type": "string", "description": "Source image path or URL." },
                "prompt": { "type": "string", "description": "What to keep, e.g. 'the car'." },
                "out": { "type": "string" },
                "backend": { "type": "string" },
                "dry_run": { "type": "boolean" }
            }
        })
    }
    fn dispatch(&self, args: &Value) -> ToolResult {
        let input = match s(args, "input") {
            Some(v) => v, None => return ToolResult::local_err(self.name(), "missing `input`"),
        };
        let prompt = match s(args, "prompt") {
            Some(v) => v, None => return ToolResult::local_err(self.name(), "missing `prompt`"),
        };
        let out = match s(args, "out") {
            Some(v) => v, None => return ToolResult::local_err(self.name(), "missing `out`"),
        };
        let mut cmd = vec![
            "image".into(), "isolate".into(),
            "--prompt".into(), prompt,
            "--out".into(), out.clone(),
        ];
        push(&mut cmd, args, "backend", "--backend");
        push(&mut cmd, args, "dry_run", "--dry-run");
        cmd.push(input);
        match spawn_gamut(&cmd) {
            Ok(child) => {
                let mut r = ToolResult::from_subprocess(self.name(), child);
                attach_out_file(&mut r, &out);
                r
            }
            Err(e) => ToolResult::local_err(self.name(), e.to_string()),
        }
    }
}
