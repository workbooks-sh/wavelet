//! `c2pa.sign` + `c2pa.verify`.

use serde_json::{json, Value};

use super::{arg_str as s, push_flag as push, spawn_gamut, Tool, ToolRegistry, ToolResult};

pub fn register(r: &mut ToolRegistry) {
    r.register(C2paSign);
    r.register(C2paVerify);
}

pub struct C2paSign;
impl Tool for C2paSign {
    fn name(&self) -> &str { "c2pa.sign" }
    fn description(&self) -> &str {
        "Sign an MP4 with a C2PA content-credentials manifest."
    }
    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "required": ["input"],
            "properties": {
                "input": { "type": "string", "description": "MP4 to sign in-place." },
                "title": { "type": "string" },
                "author": { "type": "string" },
                "signing_cert": { "type": "string" },
                "signing_key": { "type": "string" },
                "cache_root": { "type": "string" }
            }
        })
    }
    fn dispatch(&self, args: &Value) -> ToolResult {
        let input = match s(args, "input") {
            Some(v) => v, None => return ToolResult::local_err(self.name(), "missing `input`"),
        };
        let mut cmd = vec!["c2pa".into(), "sign".into(), input];
        push(&mut cmd, args, "title", "--title");
        push(&mut cmd, args, "author", "--author");
        push(&mut cmd, args, "signing_cert", "--signing-cert");
        push(&mut cmd, args, "signing_key", "--signing-key");
        push(&mut cmd, args, "cache_root", "--cache-root");
        match spawn_gamut(&cmd) {
            Ok(out) => ToolResult::from_subprocess(self.name(), out),
            Err(e) => ToolResult::local_err(self.name(), e.to_string()),
        }
    }
}

pub struct C2paVerify;
impl Tool for C2paVerify {
    fn name(&self) -> &str { "c2pa.verify" }
    fn description(&self) -> &str {
        "Verify a C2PA-signed MP4 and print the manifest tree."
    }
    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "required": ["input"],
            "properties": {
                "input": { "type": "string" }
            }
        })
    }
    fn dispatch(&self, args: &Value) -> ToolResult {
        let input = match s(args, "input") {
            Some(v) => v, None => return ToolResult::local_err(self.name(), "missing `input`"),
        };
        let cmd = vec!["c2pa".into(), "verify".into(), input];
        match spawn_gamut(&cmd) {
            Ok(out) => ToolResult::from_subprocess(self.name(), out),
            Err(e) => ToolResult::local_err(self.name(), e.to_string()),
        }
    }
}
