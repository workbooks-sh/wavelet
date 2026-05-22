//! `wavelet.music.gen` + `wavelet.dialogue.tts`.

use serde_json::{json, Value};

use super::{
    arg_str as s, attach_out_file, push_flag as push, spawn_gamut, Tool, ToolRegistry, ToolResult,
};

pub fn register(r: &mut ToolRegistry) {
    r.register(MusicGen);
    r.register(DialogueTts);
}

pub struct MusicGen;
impl Tool for MusicGen {
    fn name(&self) -> &str { "wavelet.music.gen" }
    fn description(&self) -> &str {
        "Generate a music track from a prompt + duration. \
         Omit `backend` to use the workdir wavelet.config.toml default. \
         Set `max_cost` (USD) to allow paid generation — default $0 rejects all real calls."
    }
    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "required": ["prompt", "out"],
            "properties": {
                "prompt": { "type": "string" },
                "out": { "type": "string" },
                "duration": { "type": "number", "description": "Seconds." },
                "backend": {
                    "type": "string",
                    "enum": [
                        "lyria-pro", "lyria-clip", "google-lyria-3-pro", "google-lyria-3-clip",
                        "elevenlabs", "udio"
                    ],
                    "description": "Optional. Omit for cascade default. Preferred: Lyria (Google) — works with GOOGLE_API_KEY, no separate key needed."
                },
                "model": { "type": "string" },
                "max_cost": { "type": "number", "description": "USD cap for this call. REQUIRED for paid backends; default $0 rejects." },
                "seed": { "type": "integer" },
                "dry_run": { "type": "boolean" }
            }
        })
    }
    fn dispatch(&self, args: &Value) -> ToolResult {
        let prompt = match s(args, "prompt") {
            Some(v) => v, None => return ToolResult::local_err(self.name(), "missing `prompt`"),
        };
        let out = match s(args, "out") {
            Some(v) => v, None => return ToolResult::local_err(self.name(), "missing `out`"),
        };
        let mut cmd = vec![
            "music".into(), "gen".into(),
            "--prompt".into(), prompt,
            "--out".into(), out.clone(),
        ];
        push(&mut cmd, args, "duration", "--duration");
        push(&mut cmd, args, "backend", "--backend");
        push(&mut cmd, args, "model", "--model");
        push(&mut cmd, args, "max_cost", "--max-cost");
        push(&mut cmd, args, "seed", "--seed");
        push(&mut cmd, args, "dry_run", "--dry-run");
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

pub struct DialogueTts;
impl Tool for DialogueTts {
    fn name(&self) -> &str { "wavelet.dialogue.tts" }
    fn description(&self) -> &str {
        "Synthesize speech for a line of dialogue. \
         Omit `backend` to use the workdir wavelet.config.toml default. \
         Set `max_cost` (USD) to allow paid generation — default $0 rejects all real calls."
    }
    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "required": ["text", "out"],
            "properties": {
                "text": { "type": "string" },
                "out": { "type": "string", "description": "Output WAV / MP3 path." },
                "voice": { "type": "string" },
                "backend": {
                    "type": "string",
                    "enum": ["elevenlabs", "gemini-tts", "google-gemini-tts", "fal-kokoro"],
                    "description": "Optional. Omit for cascade default. ElevenLabs requires ELEVENLABS_API_KEY; Gemini TTS works with GOOGLE_API_KEY (no extra key)."
                },
                "model": { "type": "string" },
                "rate": { "type": "number" },
                "stability": { "type": "number" },
                "max_cost": { "type": "number", "description": "USD cap for this call. REQUIRED for paid backends; default $0 rejects." },
                "dry_run": { "type": "boolean" }
            }
        })
    }
    fn dispatch(&self, args: &Value) -> ToolResult {
        let text = match s(args, "text") {
            Some(v) => v, None => return ToolResult::local_err(self.name(), "missing `text`"),
        };
        let out = match s(args, "out") {
            Some(v) => v, None => return ToolResult::local_err(self.name(), "missing `out`"),
        };
        let mut cmd = vec![
            "dialogue".into(), "tts".into(),
            "--out".into(), out.clone(),
        ];
        push(&mut cmd, args, "voice", "--voice");
        push(&mut cmd, args, "backend", "--backend");
        push(&mut cmd, args, "model", "--model");
        push(&mut cmd, args, "rate", "--rate");
        push(&mut cmd, args, "stability", "--stability");
        push(&mut cmd, args, "max_cost", "--max-cost");
        push(&mut cmd, args, "dry_run", "--dry-run");
        cmd.push(text);
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
