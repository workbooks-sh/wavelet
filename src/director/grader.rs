//! Rubric-graded outcomes loop with prompt-mutation (wb-8ync).
//!
//! Pattern (from Anthropic's "Outcomes" guidance and the May-2026
//! AI-commercial workflow research): generate → grade → **mutate
//! prompt on fail** → regen. Today's substrate retries with the *same*
//! prompt + a new seed, which is the wrong knob — the §4 cross-cutting
//! principle from the research is "prompt-rewriting is the iteration
//! substrate." Same model, same gen budget, but each retry probes a
//! semantically-different region of the model's output space.
//!
//! The grader takes a failed shot's prompt + the vision-verify
//! [`Finding`]s + the brief and asks an LLM to rewrite the prompt so it
//! addresses each FAIL/WARN finding specifically — not paraphrase, not
//! "fancier words," but a targeted edit that names what the previous
//! gen got wrong.
//!
//! Wire format:
//! - System prompt: lockedrubric-grader role + JSON output schema.
//! - User prompt: brief + original prompt + structured findings list +
//!   previous mutation history (so the model doesn't oscillate between
//!   the same two edits).
//! - Response: JSON `{mutated_prompt, reasoning, addressed_findings}`.
//!
//! The orchestrator that calls this caps `previous_mutations.len()` at
//! `max_mutations_per_shot` (default 3) and escalates to surgical edit
//! (`wavelet shot fix`) or a human checkpoint when the cap is hit. That
//! gating logic lives one layer up, in the storyboard executor.

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::backends::image::{Finding, FindingStatus};

use super::creative_director::LlmBackend;

/// One grader call.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraderRequest {
    /// The prompt that produced the failed gen.
    pub original_prompt: String,
    /// Vision-verify findings for the failed gen. The grader uses the
    /// FAIL + WARN entries; PASS entries are passed through unchanged
    /// (so the new prompt doesn't accidentally drop a passing aspect).
    pub findings: Vec<Finding>,
    /// Brief text — used as context so the grader knows what the spot
    /// is *supposed* to be.
    pub brief: String,
    /// Earlier mutations the grader has already produced for this
    /// shot. Bounded by the orchestrator (e.g. 3 max). Surfaced so the
    /// model can avoid proposing the same edit twice.
    #[serde(default)]
    pub previous_mutations: Vec<String>,
}

/// One grader response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraderResult {
    /// The rewritten prompt to feed back into the generator.
    pub mutated_prompt: String,
    /// One-paragraph rationale — what was wrong and what the new
    /// prompt changes. Stored alongside the mutation for audit and to
    /// help the next iteration if this one also fails.
    pub reasoning: String,
    /// Criterion strings (matched against `findings[i].criterion`) the
    /// grader believes the mutation addresses. Used by the orchestrator
    /// to decide whether to keep iterating on the same finding set or
    /// stop.
    pub addressed_findings: Vec<String>,
}

/// Grader-side failure modes.
#[derive(Debug, Error)]
pub enum GraderError {
    /// The backend rejected the call.
    #[error("backend: {0}")]
    Backend(#[from] crate::backends::BackendError),
    /// The LLM response wasn't valid JSON.
    #[error("response was not JSON: {0}")]
    NotJson(String),
    /// The JSON decoded but didn't match `GraderResult`.
    #[error("response failed schema: {0}")]
    SchemaMismatch(String),
    /// The grader returned an empty mutated_prompt — refuses to mutate.
    /// Surfaced separately because it's actionable: the caller should
    /// escalate to surgical edit instead of looping again.
    #[error("grader produced no mutation")]
    Empty,
    /// Nothing to grade — no FAIL or WARN findings.
    #[error("no FAIL or WARN findings to address")]
    Nothing,
}

/// Run the grader. Same `LlmBackend` trait used by the creative-
/// director synthesizer — typically `FalAnyLlmBackend` over
/// `fal-ai/any-llm`.
pub fn mutate_prompt(
    req: GraderRequest,
    llm: &dyn LlmBackend,
) -> Result<GraderResult, GraderError> {
    let to_address: Vec<&Finding> = req
        .findings
        .iter()
        .filter(|f| matches!(f.status, FindingStatus::Fail | FindingStatus::Warn))
        .collect();
    if to_address.is_empty() {
        return Err(GraderError::Nothing);
    }

    let user = build_user_prompt(
        &req.brief,
        &req.original_prompt,
        &to_address,
        &req.previous_mutations,
    );

    let raw = llm.complete(SYSTEM_PROMPT, &user, None)?;
    let parsed: GraderResult = serde_json::from_str(&raw).map_err(|e| {
        if raw.trim().is_empty() {
            GraderError::NotJson("response was empty".into())
        } else {
            GraderError::SchemaMismatch(format!("{e}: <<<{raw}>>>"))
        }
    })?;
    if parsed.mutated_prompt.trim().is_empty() {
        return Err(GraderError::Empty);
    }
    Ok(parsed)
}

/// Locked system prompt — describes the grader's role + the JSON
/// output schema. Byte-stable; a test snapshot below pins the length.
pub const SYSTEM_PROMPT: &str = "You are a prompt-rewriting assistant for AI commercial generation. The previous prompt produced a generation that failed one or more verification criteria. Your job is to rewrite the prompt so the next generation addresses each failure specifically.

Rules:
- Do NOT paraphrase or add filler. The rewrite must change something concrete: a subject detail, a camera angle, a lighting cue, a negative-prompt addition, a composition note.
- One criterion may need several prompt edits (e.g. \"car is in the wrong color\" → name the color in the subject phrase AND add the wrong color to negative prompts).
- Preserve every passing aspect of the original prompt verbatim. Only change what the findings call out.
- Look at the previous mutation history. If an earlier mutation already tried a specific angle and still failed, try a different angle this time — don't oscillate.
- The rewritten prompt is consumed by the same image / image-to-video model that just failed; write to that idiom (concrete nouns, present tense, no \"shows that\" / \"depicts\" language).

Return ONLY the JSON object. Schema:
{
  \"mutated_prompt\": \"<the rewritten prompt>\",
  \"reasoning\": \"<one paragraph: what was wrong, what the new prompt changes>\",
  \"addressed_findings\": [\"<criterion string>\", \"<criterion string>\"]
}";

/// Assemble the user prompt from the brief, original prompt, the
/// failing findings, and any earlier mutations. Stable field order so
/// the same request hashes identically across runs (cache-friendly).
pub fn build_user_prompt(
    brief: &str,
    original_prompt: &str,
    findings: &[&Finding],
    previous_mutations: &[String],
) -> String {
    let mut s = String::with_capacity(512);
    s.push_str("BRIEF:\n");
    s.push_str(brief.trim());
    s.push_str("\n\nORIGINAL PROMPT:\n");
    s.push_str(original_prompt.trim());
    s.push_str("\n\nFAILED CRITERIA (each must be addressed):\n");
    for f in findings {
        let tag = match f.status {
            FindingStatus::Fail => "FAIL",
            FindingStatus::Warn => "WARN",
            FindingStatus::Pass => continue,
        };
        s.push_str(&format!("- {tag}: {} — {}\n", f.criterion, f.reason));
    }
    if !previous_mutations.is_empty() {
        s.push_str("\nPREVIOUS MUTATIONS (already tried, do not repeat):\n");
        for (i, m) in previous_mutations.iter().enumerate() {
            s.push_str(&format!("  [{}] {}\n", i + 1, m.trim()));
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backends::BackendError;
    use std::cell::RefCell;

    fn fail_finding(criterion: &str, reason: &str) -> Finding {
        Finding {
            criterion: criterion.into(),
            status: FindingStatus::Fail,
            reason: reason.into(),
        }
    }

    fn pass_finding(criterion: &str) -> Finding {
        Finding {
            criterion: criterion.into(),
            status: FindingStatus::Pass,
            reason: "clearly met".into(),
        }
    }

    fn warn_finding(criterion: &str, reason: &str) -> Finding {
        Finding {
            criterion: criterion.into(),
            status: FindingStatus::Warn,
            reason: reason.into(),
        }
    }

    struct StubLlm {
        response: String,
        last_system: RefCell<String>,
        last_user: RefCell<String>,
    }

    impl LlmBackend for StubLlm {
        fn complete(
            &self,
            system_prompt: &str,
            user_prompt: &str,
            _retry_followup: Option<&str>,
        ) -> Result<String, BackendError> {
            *self.last_system.borrow_mut() = system_prompt.into();
            *self.last_user.borrow_mut() = user_prompt.into();
            Ok(self.response.clone())
        }
    }

    fn stub_llm(json: &str) -> StubLlm {
        StubLlm {
            response: json.into(),
            last_system: RefCell::new(String::new()),
            last_user: RefCell::new(String::new()),
        }
    }

    #[test]
    fn system_prompt_byte_stable() {
        assert!(SYSTEM_PROMPT.starts_with("You are a prompt-rewriting assistant"));
        assert!(SYSTEM_PROMPT.contains("Do NOT paraphrase"));
        assert!(SYSTEM_PROMPT.contains("Preserve every passing aspect"));
        assert!(SYSTEM_PROMPT.contains("\"mutated_prompt\""));
        assert!(SYSTEM_PROMPT.contains("\"addressed_findings\""));
        assert!(SYSTEM_PROMPT.contains("Return ONLY the JSON object"));
        // Byte length pins minor edits — update when intentionally rewording.
        assert_eq!(SYSTEM_PROMPT.len(), 1293);
    }

    #[test]
    fn user_prompt_lists_only_fail_and_warn() {
        let findings = vec![
            fail_finding("car is the right color", "car is red, brief says blue"),
            pass_finding("subject is in focus"),
            warn_finding("no other vehicles", "second car barely visible far right"),
        ];
        let refs: Vec<&Finding> = findings.iter().collect();
        let p = build_user_prompt(
            "Brief: sell blue cars.",
            "the car at golden hour, wide low-angle",
            &refs,
            &[],
        );
        assert!(p.contains("FAIL: car is the right color"));
        assert!(p.contains("WARN: no other vehicles"));
        assert!(!p.contains("subject is in focus"));
    }

    #[test]
    fn user_prompt_appends_previous_mutations_when_present() {
        let findings = vec![fail_finding("c1", "r1")];
        let refs: Vec<&Finding> = findings.iter().collect();
        let history = vec![
            "earlier attempt 1".to_string(),
            "earlier attempt 2".to_string(),
        ];
        let p = build_user_prompt("brief", "orig", &refs, &history);
        assert!(p.contains("PREVIOUS MUTATIONS"));
        assert!(p.contains("[1] earlier attempt 1"));
        assert!(p.contains("[2] earlier attempt 2"));
    }

    #[test]
    fn user_prompt_omits_history_section_when_empty() {
        let findings = vec![fail_finding("c1", "r1")];
        let refs: Vec<&Finding> = findings.iter().collect();
        let p = build_user_prompt("brief", "orig", &refs, &[]);
        assert!(!p.contains("PREVIOUS MUTATIONS"));
    }

    #[test]
    fn mutate_returns_parsed_result_on_valid_json() {
        let json = r#"{
            "mutated_prompt": "the blue car at golden hour, wide low-angle, no red cars",
            "reasoning": "Original had no color cue; brief calls out blue. Added blue + negative red.",
            "addressed_findings": ["car is the right color"]
        }"#;
        let llm = stub_llm(json);
        let result = mutate_prompt(
            GraderRequest {
                original_prompt: "the car at golden hour, wide low-angle".into(),
                findings: vec![fail_finding(
                    "car is the right color",
                    "car is red, brief says blue",
                )],
                brief: "Sell blue cars.".into(),
                previous_mutations: vec![],
            },
            &llm,
        )
        .unwrap();
        assert!(result.mutated_prompt.contains("blue car"));
        assert_eq!(result.addressed_findings, vec!["car is the right color"]);
    }

    #[test]
    fn mutate_rejects_empty_mutation() {
        let json = r#"{"mutated_prompt":"","reasoning":"x","addressed_findings":[]}"#;
        let llm = stub_llm(json);
        let err = mutate_prompt(
            GraderRequest {
                original_prompt: "orig".into(),
                findings: vec![fail_finding("c1", "r1")],
                brief: "b".into(),
                previous_mutations: vec![],
            },
            &llm,
        )
        .unwrap_err();
        assert!(matches!(err, GraderError::Empty));
    }

    #[test]
    fn mutate_rejects_no_failing_findings() {
        let llm = stub_llm("{}");
        let err = mutate_prompt(
            GraderRequest {
                original_prompt: "orig".into(),
                findings: vec![pass_finding("c1")],
                brief: "b".into(),
                previous_mutations: vec![],
            },
            &llm,
        )
        .unwrap_err();
        assert!(matches!(err, GraderError::Nothing));
    }

    #[test]
    fn mutate_surfaces_non_json_response() {
        let llm = stub_llm("nope, not JSON");
        let err = mutate_prompt(
            GraderRequest {
                original_prompt: "orig".into(),
                findings: vec![fail_finding("c1", "r1")],
                brief: "b".into(),
                previous_mutations: vec![],
            },
            &llm,
        )
        .unwrap_err();
        assert!(matches!(err, GraderError::SchemaMismatch(_)));
    }

    #[test]
    fn mutate_uses_locked_system_prompt() {
        let json = r#"{"mutated_prompt":"x","reasoning":"y","addressed_findings":[]}"#;
        let llm = stub_llm(json);
        mutate_prompt(
            GraderRequest {
                original_prompt: "orig".into(),
                findings: vec![fail_finding("c1", "r1")],
                brief: "b".into(),
                previous_mutations: vec![],
            },
            &llm,
        )
        .unwrap();
        assert_eq!(*llm.last_system.borrow(), SYSTEM_PROMPT);
    }
}
