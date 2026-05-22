//! `workflow.*`, `brief.*`, `screenplay.*`, `velocity.*`, `storyboard.*`,
//! `continuity.*`, `transitions.*`, `captions.*` tools.

use serde_json::{json, Value};

use super::{arg_str as s, push_flag as push, spawn_gamut, Tool, ToolRegistry, ToolResult};

pub fn register(r: &mut ToolRegistry) {
    r.register(WorkflowRun);
    r.register(BriefCheck);
    r.register(ScreenplayParse);
    r.register(VelocityPropose);
    r.register(VelocityValidate);
    r.register(StoryboardPlan);
    r.register(StoryboardVerify);
    r.register(ContinuityCheck);
    r.register(TransitionsClassify);
    r.register(CaptionsAlign);
}

macro_rules! string_tool {
    ($name:ident, $key:literal, $desc:literal, $req:expr, $verb:expr, [$( ($k:literal, $flag:literal) ),* $(,)?]) => {
        pub struct $name;
        impl Tool for $name {
            fn name(&self) -> &str { $key }
            fn description(&self) -> &str { $desc }
            fn parameters_schema(&self) -> Value {
                let mut props = serde_json::Map::new();
                let req: &[&str] = &$req;
                for k in req {
                    props.insert((*k).to_string(), json!({ "type": "string" }));
                }
                $(
                    props.insert($k.to_string(), json!({ "type": "string" }));
                )*
                json!({
                    "type": "object",
                    "required": $req,
                    "properties": props
                })
            }
            fn dispatch(&self, args: &Value) -> ToolResult {
                let mut cmd: Vec<String> = $verb.iter().map(|s: &&str| s.to_string()).collect();
                let req: &[&str] = &$req;
                for k in req {
                    match s(args, k) {
                        Some(v) => { cmd.push(v); }
                        None => return ToolResult::local_err(self.name(), format!("missing `{}`", k)),
                    }
                }
                $(
                    push(&mut cmd, args, $k, $flag);
                )*
                match spawn_gamut(&cmd) {
                    Ok(out) => ToolResult::from_subprocess(self.name(), out),
                    Err(e) => ToolResult::local_err(self.name(), e.to_string()),
                }
            }
        }
    };
}

pub struct WorkflowRun;
impl Tool for WorkflowRun {
    fn name(&self) -> &str { "workflow.run" }
    fn description(&self) -> &str {
        "Run a declarative pipeline (wavelet workflow run). Pass the pipeline name or YAML path."
    }
    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "required": ["pipeline"],
            "properties": {
                "pipeline": { "type": "string" },
                "workdir": { "type": "string" },
                "input": { "type": "string" },
                "dry_run": { "type": "boolean" },
                "max_cost": { "type": "number" }
            }
        })
    }
    fn dispatch(&self, args: &Value) -> ToolResult {
        let pipeline = match s(args, "pipeline") {
            Some(v) => v, None => return ToolResult::local_err(self.name(), "missing `pipeline`"),
        };
        let mut cmd = vec!["workflow".into(), "run".into(), pipeline];
        push(&mut cmd, args, "workdir", "--workdir");
        push(&mut cmd, args, "input", "--input");
        push(&mut cmd, args, "dry_run", "--dry-run");
        push(&mut cmd, args, "max_cost", "--max-cost");
        match spawn_gamut(&cmd) {
            Ok(out) => ToolResult::from_subprocess(self.name(), out),
            Err(e) => ToolResult::local_err(self.name(), e.to_string()),
        }
    }
}

string_tool!(BriefCheck, "brief.check",
    "Validate a 9-line ad brief (PRODUCT/AUDIENCE/INSIGHT/...).",
    ["brief"],
    ["brief", "check"],
    []
);

string_tool!(ScreenplayParse, "screenplay.parse",
    "Parse a Fountain (.fountain) screenplay into a structured AST.",
    ["path"],
    ["screenplay", "parse"],
    [("pretty", "--pretty"), ("out", "--out")]
);

string_tool!(VelocityPropose, "velocity.propose",
    "Propose a BPM velocity curve from a parsed screenplay.",
    ["screenplay"],
    ["velocity", "propose"],
    [("target_bpm", "--target-bpm"), ("out", "--out")]
);

string_tool!(VelocityValidate, "velocity.validate",
    "Validate a velocity profile against a music track's detected BPM. Required: `profile` (positional path), `music` (passed via --against).",
    ["profile"],
    ["velocity", "validate"],
    [("music", "--against"), ("tolerance", "--tolerance")]
);

string_tool!(StoryboardPlan, "storyboard.plan",
    "Plan a draft storyboard from a screenplay + velocity profile. Required: `screenplay` (positional), `velocity` (via --velocity).",
    ["screenplay"],
    ["storyboard", "plan"],
    [("velocity", "--velocity"), ("out", "--out")]
);

string_tool!(StoryboardVerify, "storyboard.verify",
    "Run structural verification gates on a storyboard.",
    ["storyboard"],
    ["storyboard", "verify"],
    [("json", "--json")]
);

string_tool!(ContinuityCheck, "continuity.check",
    "Per-cut continuity analysis: 180° rule, motion-vector, shot-type rhythm.",
    ["storyboard"],
    ["continuity", "check"],
    [("out", "--out")]
);

string_tool!(TransitionsClassify, "transitions.classify",
    "Classify Fountain transitions against a velocity profile. Required: `screenplay` (positional), `velocity` (via --velocity).",
    ["screenplay"],
    ["transitions", "classify"],
    [("velocity", "--velocity"), ("out", "--out")]
);

string_tool!(CaptionsAlign, "captions.align",
    "Align word-level captions to an audio track.",
    ["audio", "transcript"],
    ["captions", "align"],
    [("out", "--out"), ("style", "--style")]
);
