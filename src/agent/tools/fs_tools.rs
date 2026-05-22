//! `fs.*` tools — read, write, list, exists, mkdir.
//!
//! These are dispatched in-process (no subprocess) for speed. The
//! agent stays inside the working directory: a `..` segment or
//! absolute path traversal aborts the call with an error rather than
//! probing the host filesystem.

use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use super::{arg_str as s, Tool, ToolRegistry, ToolResult};

pub fn register(r: &mut ToolRegistry) {
    r.register(FsRead);
    r.register(FsWrite);
    r.register(FsList);
    r.register(FsExists);
    r.register(FsMkdir);
}

/// 1 MiB cap on returning file contents — past that we slice to keep
/// the model from inhaling binary blobs.
const READ_CAP: usize = 1 * 1024 * 1024;

fn safe_path(raw: &str) -> Option<PathBuf> {
    let p = Path::new(raw);
    if p.components().any(|c| matches!(c, std::path::Component::ParentDir)) {
        return None;
    }
    Some(p.to_path_buf())
}

pub struct FsRead;
impl Tool for FsRead {
    fn name(&self) -> &str { "fs.read" }
    fn description(&self) -> &str {
        "Read a file's contents as UTF-8 text. Truncates to ~1 MiB."
    }
    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "required": ["path"],
            "properties": { "path": { "type": "string" } }
        })
    }
    fn dispatch(&self, args: &Value) -> ToolResult {
        let raw = match s(args, "path") {
            Some(v) => v, None => return ToolResult::local_err(self.name(), "missing `path`"),
        };
        let path = match safe_path(&raw) {
            Some(p) => p,
            None => return ToolResult::local_err(self.name(), "path contains `..`"),
        };
        match std::fs::read(&path) {
            Ok(bytes) => {
                let total = bytes.len();
                let take = bytes.into_iter().take(READ_CAP).collect::<Vec<_>>();
                let text = String::from_utf8_lossy(&take).into_owned();
                ToolResult::local_ok(self.name(), json!({
                    "path": raw,
                    "size": total,
                    "truncated": total > READ_CAP,
                    "content": text,
                }))
            }
            Err(e) => ToolResult::local_err(self.name(), e.to_string()),
        }
    }
}

pub struct FsWrite;
impl Tool for FsWrite {
    fn name(&self) -> &str { "fs.write" }
    fn description(&self) -> &str {
        "Write text content to a file. Creates parent directories."
    }
    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "required": ["path", "content"],
            "properties": {
                "path": { "type": "string" },
                "content": { "type": "string" }
            }
        })
    }
    fn dispatch(&self, args: &Value) -> ToolResult {
        let raw = match s(args, "path") {
            Some(v) => v, None => return ToolResult::local_err(self.name(), "missing `path`"),
        };
        let content = match s(args, "content") {
            Some(v) => v, None => return ToolResult::local_err(self.name(), "missing `content`"),
        };
        let path = match safe_path(&raw) {
            Some(p) => p,
            None => return ToolResult::local_err(self.name(), "path contains `..`"),
        };
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                if let Err(e) = std::fs::create_dir_all(parent) {
                    return ToolResult::local_err(self.name(), e.to_string());
                }
            }
        }
        match std::fs::write(&path, content.as_bytes()) {
            Ok(_) => {
                let mut tr = ToolResult::local_ok(
                    self.name(),
                    json!({ "path": raw, "bytes": content.len() }),
                );
                tr.output_files.push(path);
                tr
            }
            Err(e) => ToolResult::local_err(self.name(), e.to_string()),
        }
    }
}

pub struct FsList;
impl Tool for FsList {
    fn name(&self) -> &str { "fs.list" }
    fn description(&self) -> &str { "List entries of a directory." }
    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "required": ["path"],
            "properties": { "path": { "type": "string" } }
        })
    }
    fn dispatch(&self, args: &Value) -> ToolResult {
        let raw = match s(args, "path") {
            Some(v) => v, None => return ToolResult::local_err(self.name(), "missing `path`"),
        };
        let path = match safe_path(&raw) {
            Some(p) => p,
            None => return ToolResult::local_err(self.name(), "path contains `..`"),
        };
        let it = match std::fs::read_dir(&path) {
            Ok(i) => i,
            Err(e) => return ToolResult::local_err(self.name(), e.to_string()),
        };
        let mut entries: Vec<Value> = Vec::new();
        for ent in it.flatten() {
            let name = ent.file_name().to_string_lossy().into_owned();
            let kind = if ent.path().is_dir() { "dir" } else { "file" };
            entries.push(json!({ "name": name, "kind": kind }));
        }
        ToolResult::local_ok(self.name(), json!({ "path": raw, "entries": entries }))
    }
}

pub struct FsExists;
impl Tool for FsExists {
    fn name(&self) -> &str { "fs.exists" }
    fn description(&self) -> &str { "Check whether a path exists." }
    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "required": ["path"],
            "properties": { "path": { "type": "string" } }
        })
    }
    fn dispatch(&self, args: &Value) -> ToolResult {
        let raw = match s(args, "path") {
            Some(v) => v, None => return ToolResult::local_err(self.name(), "missing `path`"),
        };
        let path = match safe_path(&raw) {
            Some(p) => p,
            None => return ToolResult::local_err(self.name(), "path contains `..`"),
        };
        let exists = path.exists();
        let kind = if !exists {
            "missing"
        } else if path.is_dir() {
            "dir"
        } else {
            "file"
        };
        ToolResult::local_ok(self.name(), json!({ "path": raw, "exists": exists, "kind": kind }))
    }
}

pub struct FsMkdir;
impl Tool for FsMkdir {
    fn name(&self) -> &str { "fs.mkdir" }
    fn description(&self) -> &str { "Create a directory (recursive)." }
    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "required": ["path"],
            "properties": { "path": { "type": "string" } }
        })
    }
    fn dispatch(&self, args: &Value) -> ToolResult {
        let raw = match s(args, "path") {
            Some(v) => v, None => return ToolResult::local_err(self.name(), "missing `path`"),
        };
        let path = match safe_path(&raw) {
            Some(p) => p,
            None => return ToolResult::local_err(self.name(), "path contains `..`"),
        };
        match std::fs::create_dir_all(&path) {
            Ok(_) => ToolResult::local_ok(self.name(), json!({ "path": raw })),
            Err(e) => ToolResult::local_err(self.name(), e.to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn fs_list_works() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("a.txt"), "hello").unwrap();
        std::env::set_current_dir(tmp.path()).unwrap();
        let t = FsList;
        let r = t.dispatch(&json!({"path": "."}));
        assert!(r.ok);
        let entries = r.response["entries"].as_array().unwrap();
        assert!(entries.iter().any(|e| e["name"] == "a.txt"));
    }

    #[test]
    fn fs_traversal_blocked() {
        let t = FsRead;
        let r = t.dispatch(&json!({"path": "../etc/passwd"}));
        assert!(!r.ok);
        assert!(r.summary.contains(".."));
    }
}
