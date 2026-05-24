//! `wavelet.render` + `verify` tools.

use serde_json::{json, Value};

use super::{arg_str as s, push_flag as push, spawn_gamut, Tool, ToolRegistry, ToolResult};

pub fn register(r: &mut ToolRegistry) {
    r.register(Render);
    r.register(Verify);
}

pub struct Render;
impl Tool for Render {
    fn name(&self) -> &str { "wavelet.render" }
    fn description(&self) -> &str {
        "Render a `commercial.html` composition manifest to MP4. The \
         manifest is a single top-level `commercial.html` (or `index.html`) \
         listing scenes + audio. Each `<section data-scene-href=\"...\" \
         data-duration=\"5s\">` is one scene; each \
         `<audio src=\"...\" data-spans=\"all\">` is an audio cue. Per-scene \
         HTML files contain `<video src=\"shots/shot-N.mp4\">` (or `<img>` \
         for stills) plus any creative overlay markup. HTML is the only \
         accepted input — JSON inputs are rejected at exit 3 with no \
         fallback path. \
         Example manifest: \
         `<!doctype html><html><head><meta name=\"resolution\" content=\"1920x1080\">\
         <meta name=\"fps\" content=\"30\"><meta name=\"duration\" content=\"15s\">\
         </head><body><section data-scene-href=\"scenes/01.html\" data-duration=\"5s\">\
         </section><section data-scene-href=\"scenes/02.html\" data-duration=\"5s\">\
         </section><section data-scene-href=\"scenes/03.html\" data-duration=\"5s\">\
         </section><audio src=\"music/track.wav\" data-spans=\"all\"></audio></body>\
         </html>`. Optionally sign the output with C2PA."
    }
    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "required": ["comp"],
            "properties": {
                "comp": { "type": "string", "description": "Path to HTML composition manifest (e.g. `commercial.html`)." },
                "out": { "type": "string", "description": "Output MP4 path. Defaults to <comp-stem>.mp4." },
                "sign_c2pa": { "type": "boolean" },
                "title": { "type": "string" },
                "author": { "type": "string" },
                "aspects": { "type": "string", "description": "Comma-separated aspects, e.g. `16:9,9:16`." }
            }
        })
    }
    fn dispatch(&self, args: &Value) -> ToolResult {
        let comp = match s(args, "comp") {
            Some(v) => v, None => return ToolResult::local_err(self.name(), "missing `comp`"),
        };
        let mut cmd = vec!["render".into(), comp];
        push(&mut cmd, args, "out", "--out");
        push(&mut cmd, args, "sign_c2pa", "--sign-c2pa");
        push(&mut cmd, args, "title", "--title");
        push(&mut cmd, args, "author", "--author");
        push(&mut cmd, args, "aspects", "--aspects");
        match spawn_gamut(&cmd) {
            Ok(out) => ToolResult::from_subprocess(self.name(), out),
            Err(e) => ToolResult::local_err(self.name(), e.to_string()),
        }
    }
}

pub struct Verify;
impl Tool for Verify {
    fn name(&self) -> &str { "verify" }
    fn description(&self) -> &str {
        "Lint a composition. Structural checks; --deep also renders mid-frames."
    }
    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "required": ["comp"],
            "properties": {
                "comp": { "type": "string" },
                "deep": { "type": "boolean" }
            }
        })
    }
    fn dispatch(&self, args: &Value) -> ToolResult {
        let comp = match s(args, "comp") {
            Some(v) => v, None => return ToolResult::local_err(self.name(), "missing `comp`"),
        };
        let mut cmd = vec!["verify".into(), comp];
        push(&mut cmd, args, "deep", "--deep");
        match spawn_gamut(&cmd) {
            Ok(out) => ToolResult::from_subprocess(self.name(), out),
            Err(e) => ToolResult::local_err(self.name(), e.to_string()),
        }
    }
}
