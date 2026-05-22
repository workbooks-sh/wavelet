//! `query.*` tools — scene-graph, pixels, snapshot, beat alignment.

use serde_json::{json, Value};

use super::{arg_str as s, push_flag as push, spawn_gamut, Tool, ToolRegistry, ToolResult};

pub fn register(r: &mut ToolRegistry) {
    r.register(QuerySceneGraph);
    r.register(QueryPixels);
    r.register(QuerySnapshot);
    r.register(QueryBeat);
}

fn run_query(args: &Value, tool_name: &str, extra_flags: &[(&str, &str)]) -> ToolResult {
    let comp = match s(args, "comp") {
        Some(v) => v, None => return ToolResult::local_err(tool_name, "missing `comp`"),
    };
    let mut cmd = vec!["query".into(), comp];
    push(&mut cmd, args, "at", "--at");
    for (k, flag) in extra_flags {
        push(&mut cmd, args, k, flag);
    }
    cmd.push("--json".into());
    match spawn_gamut(&cmd) {
        Ok(out) => ToolResult::from_subprocess(tool_name, out),
        Err(e) => ToolResult::local_err(tool_name, e.to_string()),
    }
}

pub struct QuerySceneGraph;
impl Tool for QuerySceneGraph {
    fn name(&self) -> &str { "query.scene_graph" }
    fn description(&self) -> &str {
        "Query a composition's scene-graph at a given time without touching pixels."
    }
    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "required": ["comp", "at"],
            "properties": {
                "comp": { "type": "string" },
                "at": { "type": "string", "description": "Time spec, e.g. 0s, 2.5s." },
                "bbox": { "type": "string", "description": "CSS selector to bbox." },
                "visible": { "type": "string", "description": "Selector for visibility." }
            }
        })
    }
    fn dispatch(&self, args: &Value) -> ToolResult {
        run_query(args, self.name(), &[("bbox", "--bbox"), ("visible", "--visible")])
    }
}

pub struct QueryPixels;
impl Tool for QueryPixels {
    fn name(&self) -> &str { "query.pixels" }
    fn description(&self) -> &str {
        "Query pixel-level properties (color, contrast, banding) at a time."
    }
    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "required": ["comp", "at"],
            "properties": {
                "comp": { "type": "string" },
                "at": { "type": "string" },
                "color_at": { "type": "string" },
                "color_in": { "type": "string" },
                "contrast": { "type": "string" },
                "banding": { "type": "string" }
            }
        })
    }
    fn dispatch(&self, args: &Value) -> ToolResult {
        run_query(args, self.name(), &[
            ("color_at", "--color-at"),
            ("color_in", "--color-in"),
            ("contrast", "--contrast"),
            ("banding", "--banding"),
        ])
    }
}

pub struct QuerySnapshot;
impl Tool for QuerySnapshot {
    fn name(&self) -> &str { "query.snapshot" }
    fn description(&self) -> &str {
        "Render a snapshot PNG of the composition at a given time."
    }
    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "required": ["comp", "at", "snapshot"],
            "properties": {
                "comp": { "type": "string" },
                "at": { "type": "string" },
                "snapshot": { "type": "string", "description": "Output PNG path." }
            }
        })
    }
    fn dispatch(&self, args: &Value) -> ToolResult {
        run_query(args, self.name(), &[("snapshot", "--snapshot")])
    }
}

pub struct QueryBeat;
impl Tool for QueryBeat {
    fn name(&self) -> &str { "query.beat" }
    fn description(&self) -> &str {
        "Score on-beat alignment of an event vs detected onsets."
    }
    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "required": ["comp", "at", "on_beat"],
            "properties": {
                "comp": { "type": "string" },
                "at": { "type": "string" },
                "on_beat": { "type": "string", "description": "Selector / event name." },
                "tolerance_ms": { "type": "integer" }
            }
        })
    }
    fn dispatch(&self, args: &Value) -> ToolResult {
        run_query(args, self.name(), &[
            ("on_beat", "--on-beat"),
            ("tolerance_ms", "--tolerance-ms"),
        ])
    }
}
