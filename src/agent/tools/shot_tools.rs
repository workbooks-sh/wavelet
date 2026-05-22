//! `wavelet.shot.*` tools — edit, txt2vid, img2vid, refine_face, upscale.
//!
//! All five spawn the wavelet binary with `shot <verb> ...`. Schemas
//! mirror the clap definitions in `bin/wavelet.rs` for the corresponding
//! ShotOp variant. Hand-rolled — we don't try to auto-generate from
//! clap (too brittle).

use serde_json::{json, Value};

use super::{
    arg_str as s, attach_out_file, push_flag as push, spawn_gamut, Tool, ToolRegistry, ToolResult,
};

pub fn register(r: &mut ToolRegistry) {
    r.register(ShotEdit);
    r.register(ShotTxt2Vid);
    r.register(ShotUpscale);
}

pub struct ShotEdit;
impl Tool for ShotEdit {
    fn name(&self) -> &str { "wavelet.shot.edit" }
    fn description(&self) -> &str {
        "Plan/execute/review edit of a rendered shot (.mp4) or scene (.html). \
         Pass a natural-language `intent` describing the change."
    }
    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "required": ["input", "intent"],
            "properties": {
                "input": { "type": "string", "description": "Path to the .mp4 or scene .html." },
                "intent": { "type": "string", "description": "Plain-English edit instruction." },
                "out": { "type": "string", "description": "Output .mp4 path (defaults beside input)." },
                "report": { "type": "string", "description": "Report JSON path." },
                "max_attempts": { "type": "integer", "minimum": 1, "maximum": 8 },
                "max_cost": { "type": "number", "description": "USD budget cap." },
                "pass_threshold": { "type": "number" },
                "dry_run": { "type": "boolean" }
            }
        })
    }
    fn dispatch(&self, args: &Value) -> ToolResult {
        let input = match s(args, "input") {
            Some(v) => v,
            None => return ToolResult::local_err(self.name(), "missing `input`"),
        };
        let intent = match s(args, "intent") {
            Some(v) => v,
            None => return ToolResult::local_err(self.name(), "missing `intent`"),
        };
        let out_arg = s(args, "out");
        let mut cmd = vec!["shot".into(), "edit".into(), input, "--intent".into(), intent];
        push(&mut cmd, args, "out", "--out");
        push(&mut cmd, args, "report", "--report");
        push(&mut cmd, args, "max_attempts", "--max-attempts");
        push(&mut cmd, args, "max_cost", "--max-cost");
        push(&mut cmd, args, "pass_threshold", "--pass-threshold");
        push(&mut cmd, args, "dry_run", "--dry-run");
        match spawn_gamut(&cmd) {
            Ok(out) => {
                let mut r = ToolResult::from_subprocess(self.name(), out);
                if let Some(p) = out_arg {
                    attach_out_file(&mut r, &p);
                }
                r
            }
            Err(e) => ToolResult::local_err(self.name(), e.to_string()),
        }
    }
}

pub struct ShotTxt2Vid;
impl Tool for ShotTxt2Vid {
    fn name(&self) -> &str { "wavelet.shot.txt2vid" }
    fn description(&self) -> &str {
        "Generate a short video clip from a text prompt. Returns the produced .mp4 path. \
         For best results, supply `shot_spec` instead of `prompt` — it emits Veo's 5-part \
         formula (cinematography + subject + action + context + style) with anti-stock \
         negatives by default. See the `shot_spec` schema below. \
         Omit `backend` to use the workdir wavelet.config.toml default. \
         Set `max_cost` (USD) to allow paid generation — default $0 rejects all real calls."
    }
    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "required": ["out"],
            "properties": {
                "prompt": { "type": "string", "description": "Free-form prompt. Mutually exclusive with `shot_spec`." },
                "shot_spec": {
                    "type": "object",
                    "description": "Structured prompt — preferred. Emits Veo 5-part formula + anti-stock negatives.",
                    "required": ["subject", "action"],
                    "properties": {
                        "subject": { "type": "string" },
                        "action": { "type": "string" },
                        "context": { "type": "string" },
                        "style": { "type": "string" },
                        "cinematography": {
                            "type": "object",
                            "properties": {
                                "shot_type": { "type": "string", "description": "e.g. close-up, wide shot" },
                                "lens": { "type": "string", "description": "e.g. 85mm, shallow depth of field" },
                                "movement": { "type": "string", "description": "e.g. dolly in, handheld" },
                                "lighting": { "type": "string", "description": "e.g. golden hour, hard side light" }
                            }
                        },
                        "anti_stock": { "type": "boolean", "description": "Default true. Disables the anti-stock negative-prompt block." },
                        "negative_prompt": { "type": "string", "description": "Appended to anti-stock unless anti_stock is false." }
                    }
                },
                "out": { "type": "string" },
                "backend": {
                    "type": "string",
                    "enum": ["fal-wan-t2v", "veo", "veo-3.1", "veo-fast", "veo-3.1-fast", "veo-lite", "veo-3.1-lite"],
                    "description": "Optional. Omit to use the workdir wavelet.config.toml default."
                },
                "model": { "type": "string" },
                "aspect": { "type": "string", "description": "16:9, 9:16, 1:1." },
                "duration": { "type": "number", "description": "Seconds." },
                "max_cost": { "type": "number", "description": "USD cap for this call. REQUIRED for paid backends; default $0 rejects." },
                "seed": { "type": "integer" },
                "dry_run": { "type": "boolean" }
            }
        })
    }
    fn dispatch(&self, args: &Value) -> ToolResult {
        let out = match s(args, "out") {
            Some(v) => v,
            None => return ToolResult::local_err(self.name(), "missing `out`"),
        };

        let has_prompt = args.get("prompt").and_then(|v| v.as_str()).is_some();
        let has_spec = args.get("shot_spec").map(|v| v.is_object()).unwrap_or(false);

        if has_prompt && has_spec {
            return ToolResult::local_err(self.name(), "supply either `prompt` or `shot_spec`, not both");
        }

        let (prompt, negative): (String, Option<String>) = if has_spec {
            match serde_json::from_value::<crate::agent::prompt_builder::ShotPrompt>(
                args.get("shot_spec").cloned().unwrap_or(Value::Null),
            ) {
                Ok(spec) => {
                    let (p, n) = crate::agent::prompt_builder::build_veo_prompt(&spec);
                    (p, if n.is_empty() { None } else { Some(n) })
                }
                Err(e) => return ToolResult::local_err(self.name(), format!("invalid shot_spec: {e}")),
            }
        } else {
            match s(args, "prompt") {
                Some(v) => (v, None),
                None => return ToolResult::local_err(self.name(), "missing `prompt` (or supply `shot_spec`)"),
            }
        };

        let mut cmd = vec![
            "shot".into(), "txt2vid".into(),
            "--out".into(), out.clone(),
        ];
        push(&mut cmd, args, "backend", "--backend");
        push(&mut cmd, args, "model", "--model");
        push(&mut cmd, args, "aspect", "--aspect");
        push(&mut cmd, args, "duration", "--duration");
        push(&mut cmd, args, "max_cost", "--max-cost");
        push(&mut cmd, args, "seed", "--seed");
        push(&mut cmd, args, "dry_run", "--dry-run");
        if let Some(neg) = negative {
            cmd.push("--negative".into());
            cmd.push(neg);
            cmd.push("--no-default-negatives".into());
        }
        cmd.push(prompt);
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

pub struct ShotUpscale;
impl Tool for ShotUpscale {
    fn name(&self) -> &str { "wavelet.shot.upscale" }
    fn description(&self) -> &str {
        "Upscale a video (e.g. 720→1080, 1080→4K) via a backend model."
    }
    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "required": ["input", "out"],
            "properties": {
                "input": { "type": "string" },
                "out": { "type": "string" },
                "target": { "type": "string", "description": "720p | 1080p | 4k" },
                "model": { "type": "string" },
                "backend": { "type": "string" },
                "dry_run": { "type": "boolean" }
            }
        })
    }
    fn dispatch(&self, args: &Value) -> ToolResult {
        let input = match s(args, "input") {
            Some(v) => v,
            None => return ToolResult::local_err(self.name(), "missing `input`"),
        };
        let out = match s(args, "out") {
            Some(v) => v,
            None => return ToolResult::local_err(self.name(), "missing `out`"),
        };
        let mut cmd = vec![
            "shot".into(), "upscale".into(),
            "--out".into(), out.clone(),
        ];
        push(&mut cmd, args, "target", "--target");
        push(&mut cmd, args, "model", "--model");
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shot_edit_schema_valid() {
        let t = ShotEdit;
        let schema = t.parameters_schema();
        assert_eq!(schema["type"], "object");
        let req = schema["required"].as_array().unwrap();
        assert!(req.iter().any(|v| v == "input"));
        assert!(req.iter().any(|v| v == "intent"));
    }

    #[test]
    fn shot_edit_missing_intent_errors() {
        let t = ShotEdit;
        let r = t.dispatch(&json!({"input": "foo.mp4"}));
        assert!(!r.ok);
        assert!(r.summary.contains("intent"));
    }

}
