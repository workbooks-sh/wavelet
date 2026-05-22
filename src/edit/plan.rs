//! Planner — asks Gemini to decompose an intent into a typed JSON plan.
//!
//! The planner is the only stage that talks to a large-context model
//! for *generation*. The executor is dumb dispatch; the reviewer is
//! grade-only. The plan schema below is the contract.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use super::EditError;
use super::intent::{EditRequest, InputKind};

/// Top-level plan returned by the planner LLM.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Plan {
    /// Planner's one-line restatement of the intent — useful for
    /// surface logging + so the reviewer can see what the planner
    /// thought it was asked to do.
    pub intent_summary: String,
    /// Coarse strategy. Drives executor routing.
    pub approach: Approach,
    /// USD cost the planner estimates this plan will incur.
    pub estimated_cost_usd: f32,
    /// Wall-clock seconds the planner estimates this plan will take.
    pub estimated_seconds: u32,
    /// Ordered list of tool invocations.
    pub steps: Vec<Step>,
    /// Planner's chain-of-reasoning — surfaced in the report so a
    /// human reviewer can audit the decomposition.
    pub reasoning: String,
}

/// High-level approach the planner picked.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Approach {
    /// CSS-only edits on the source scene HTML — cheap, fast, no
    /// pixel models invoked.
    CssOnly,
    /// In-place pixel editing via the (not-yet-shipped) Gemini Omni
    /// edit endpoint. Returns a clear "not yet available" error in
    /// v1.
    OmniEdit,
    /// Re-roll the entire shot via Veo with a new prompt.
    VeoRegen,
    /// Mix-and-match — multiple sub-renders composited with ffmpeg.
    Composite,
}

/// A single executable step. Each variant maps 1:1 to a function in
/// `edit::tools::*`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum Step {
    /// Add a `filter:` declaration (or any other static CSS rule) to
    /// the target selector.
    CssFilter {
        /// CSS selector the rule binds to (e.g. `body`, `.sky`,
        /// `#headline`).
        target_selector: String,
        /// Raw CSS declarations — without enclosing braces, e.g.
        /// `filter: brightness(0.6) hue-rotate(20deg);`.
        css: String,
    },
    /// Insert a `@keyframes` block + an `animation:` shorthand on the
    /// target.
    CssAnimation {
        /// CSS selector to animate.
        target_selector: String,
        /// Raw CSS — typically a `@keyframes` block plus an
        /// `animation:` shorthand assigned to the selector.
        css: String,
    },
    /// Set `animation-duration` / `animation-play-state` indirectly
    /// via a multiplier on every animation on the target.
    PlaybackRate {
        /// CSS selector whose animation timing gets scaled.
        target_selector: String,
        /// Multiplier. Values > 1.0 slow down (longer duration);
        /// values < 1.0 speed up. The executor implements this by
        /// rewriting `animation-duration` on the matched rule.
        value: f32,
    },
    /// Override the composition's duration (in seconds). Used when
    /// the user asks for "slower" / "longer" but doesn't want to
    /// touch keyframes.
    DurationOverride {
        /// Replacement duration in seconds.
        secs: f32,
    },
    /// Re-render the scene HTML (after CSS edits have been applied)
    /// and produce a new MP4.
    ReRender {
        /// Optional duration override applied to the comp before
        /// render. Useful in combination with `PlaybackRate`.
        duration_secs: Option<f32>,
    },
    /// Pixel-level Omni edit. Returns "not yet available" in v1 —
    /// the model slug isn't shipped.
    OmniEdit {
        /// Edit instruction routed through the Omni model.
        instruction: String,
        /// Optional sampled-frame timestamps to focus the edit on
        /// (seconds).
        frames_at: Option<Vec<f32>>,
    },
    /// Re-roll the entire shot via Veo 3.1 Fast.
    VeoRegen {
        /// Replacement Veo prompt.
        prompt: String,
        /// Clip length.
        duration_secs: f32,
        /// Aspect ratio (`16:9`, `9:16`, etc.).
        aspect: String,
        /// Step-level USD ceiling.
        max_cost_usd: f32,
    },
    /// Concat / overlay a fragment of another video onto the output.
    Splice {
        /// Source MP4.
        source: PathBuf,
        /// Start time in seconds (inclusive).
        start_secs: f32,
        /// End time in seconds (exclusive).
        end_secs: f32,
    },
}

/// Build a planner prompt for the LLM.
pub fn build_planner_prompt(req: &EditRequest, prior_critique: Option<&str>) -> String {
    let kind = match req.kind {
        InputKind::Mp4 => "rendered MP4 (no scene HTML available — CssOnly is unavailable unless a sibling scene HTML can be located)",
        InputKind::SceneHtml => "scene HTML (full CSS-only path is available)",
    };
    let critique_block = match prior_critique {
        Some(c) => format!(
            "\n=== PRIOR ATTEMPT FAILED ===\nThe last plan was reviewed and rejected. Reviewer feedback:\n{c}\nUse this to pick a different approach or refine the steps.\n",
        ),
        None => String::new(),
    };
    format!(
        r#"You are a video-edit planner. Decompose the user's intent into a typed JSON plan that gets dispatched to a fixed set of tools.

=== INPUT ===
Path: {input}
Kind: {kind}
Intent: {intent}{critique_block}

=== TOOLS AVAILABLE ===
- CssOnly approach: CssFilter, CssAnimation, PlaybackRate, DurationOverride, ReRender — cheap, fast, no pixel models. Use when the intent is a color/timing/animation change and the input is a scene HTML.
- VeoRegen approach: VeoRegen — re-roll the whole shot via Veo 3.1 Fast (~$0.25/sec). Use when the intent changes scene content (camera move, subject identity, dramatically different visuals).
- OmniEdit approach: OmniEdit — pixel-level in-place edits. NOT YET SHIPPED. Only pick this if the user explicitly asks for surgical in-place pixel edits; the executor returns an "unavailable" error for v1.
- Composite approach: Splice — combine multiple sources via ffmpeg.

=== OUTPUT SCHEMA ===
Reply with a single JSON object, no commentary, no code fences:

{{
  "intent_summary": "<one-line restatement of the user's intent>",
  "approach": "CssOnly" | "OmniEdit" | "VeoRegen" | "Composite",
  "estimated_cost_usd": <float>,
  "estimated_seconds": <int>,
  "reasoning": "<why this approach, what trade-offs>",
  "steps": [
    {{ "kind": "CssFilter", "target_selector": "...", "css": "..." }},
    {{ "kind": "CssAnimation", "target_selector": "...", "css": "..." }},
    {{ "kind": "PlaybackRate", "target_selector": "...", "value": 1.5 }},
    {{ "kind": "DurationOverride", "secs": 8.0 }},
    {{ "kind": "ReRender", "duration_secs": 8.0 }},
    {{ "kind": "OmniEdit", "instruction": "...", "frames_at": [0.5, 1.5] }},
    {{ "kind": "VeoRegen", "prompt": "...", "duration_secs": 8.0, "aspect": "16:9", "max_cost_usd": 2.0 }},
    {{ "kind": "Splice", "source": "...", "start_secs": 0.0, "end_secs": 1.0 }}
  ]
}}

The "kind" tag on each step is required and case-sensitive. Pick exactly one approach and only include steps consistent with it. CssOnly plans MUST end with a ReRender step. VeoRegen plans MUST contain exactly one VeoRegen step.

Reply with the JSON object only.
"#,
        input = req.input.display(),
        kind = kind,
        intent = req.intent,
    )
}

/// Parse a planner LLM response into a `Plan`.
pub fn parse_plan(raw: &str) -> Result<Plan, EditError> {
    // Strip code fences if the model wrapped its output.
    let body = strip_code_fence(raw);
    // Find the first JSON object — defensive against prose preamble.
    let start = body.find('{');
    let end = body.rfind('}');
    let json = match (start, end) {
        (Some(s), Some(e)) if e >= s => &body[s..=e],
        _ => {
            return Err(EditError::PlanParse(format!(
                "no JSON object in planner response: {}",
                raw.chars().take(400).collect::<String>()
            )))
        }
    };
    serde_json::from_str::<Plan>(json).map_err(|e| {
        EditError::PlanParse(format!(
            "decode plan: {e}\nraw: {}",
            raw.chars().take(400).collect::<String>()
        ))
    })
}

fn strip_code_fence(s: &str) -> &str {
    let trimmed = s.trim();
    if let Some(rest) = trimmed.strip_prefix("```json") {
        return rest.trim_end_matches("```").trim();
    }
    if let Some(rest) = trimmed.strip_prefix("```") {
        return rest.trim_end_matches("```").trim();
    }
    trimmed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_minimal_plan() {
        let raw = r#"{
            "intent_summary": "make it dusk",
            "approach": "CssOnly",
            "estimated_cost_usd": 0.01,
            "estimated_seconds": 8,
            "reasoning": "CSS filter is sufficient",
            "steps": [
                { "kind": "CssFilter", "target_selector": "body", "css": "filter: brightness(0.6) hue-rotate(20deg);" },
                { "kind": "ReRender", "duration_secs": null }
            ]
        }"#;
        let plan = parse_plan(raw).unwrap();
        assert_eq!(plan.approach, Approach::CssOnly);
        assert_eq!(plan.steps.len(), 2);
        match &plan.steps[0] {
            Step::CssFilter { target_selector, .. } => assert_eq!(target_selector, "body"),
            other => panic!("expected CssFilter, got {other:?}"),
        }
    }

    #[test]
    fn parses_plan_wrapped_in_code_fence() {
        let raw = "```json\n{\n  \"intent_summary\": \"x\",\n  \"approach\": \"VeoRegen\",\n  \"estimated_cost_usd\": 2.0,\n  \"estimated_seconds\": 60,\n  \"reasoning\": \"need re-roll\",\n  \"steps\": [ { \"kind\": \"VeoRegen\", \"prompt\": \"a cat\", \"duration_secs\": 8.0, \"aspect\": \"16:9\", \"max_cost_usd\": 2.0 } ]\n}\n```";
        let plan = parse_plan(raw).unwrap();
        assert_eq!(plan.approach, Approach::VeoRegen);
    }

    #[test]
    fn rejects_plan_with_no_json() {
        let err = parse_plan("Sorry I cannot help with that.").unwrap_err();
        match err {
            EditError::PlanParse(msg) => assert!(msg.contains("no JSON object")),
            other => panic!("expected PlanParse, got {other:?}"),
        }
    }

    #[test]
    fn rejects_malformed_json() {
        let err = parse_plan(r#"{ "approach": "CssOnly", broken }"#).unwrap_err();
        assert!(matches!(err, EditError::PlanParse(_)));
    }

    #[test]
    fn planner_prompt_mentions_intent_and_input() {
        use crate::edit::intent::{EditConfig, EditRequest, InputKind};
        let req = EditRequest {
            input: PathBuf::from("/tmp/shot.html"),
            kind: InputKind::SceneHtml,
            intent: "make it dusk and slower".into(),
            cfg: EditConfig {
                max_attempts: 3,
                max_cost_usd: 0.5,
                pass_threshold: 0.7,
                planner_model: "gemini-3.1-pro-preview".into(),
                reviewer_model: "gemini-3.5-flash".into(),
                out_path: PathBuf::from("/tmp/out.mp4"),
                report_path: PathBuf::from("/tmp/r.json"),
                dry_run: false,
            },
        };
        let p = build_planner_prompt(&req, None);
        assert!(p.contains("make it dusk and slower"));
        assert!(p.contains("/tmp/shot.html"));
        assert!(p.contains("CssOnly"));
        let with_critique = build_planner_prompt(&req, Some("too dark"));
        assert!(with_critique.contains("PRIOR ATTEMPT FAILED"));
        assert!(with_critique.contains("too dark"));
    }
}
