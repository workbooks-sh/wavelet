//! LLM-as-creative-director orchestration (wb-epk3).
//!
//! Given a brief + per-shot skeleton (`subject`, `action`, and the
//! `id` it'll be merged back onto), one LLM call fills the seven
//! L-Storyboard slots for every shot in the spot. Today's deterministic
//! `Generation`-payload + shot-type-label prompt assembly is replaced
//! by an LLM that writes specific, consistent, director-grade prose
//! across the whole spot — the way 9 of 15 commercial ComfyUI
//! workflows do it.
//!
//! ## Flow
//!
//! 1. Caller builds [`DirectorRequest`] from a brief + the
//!    `Vec<ShotSkeleton>` (one per shot in the storyboard).
//! 2. [`synthesize_shot_attributes`] serializes the skeleton + brief
//!    into the user prompt (see [`super::prompts`]), sends the system
//!    + user prompt through any [`LlmBackend`] impl, and parses the
//!    returned JSON.
//! 3. Every returned shot is validated via `ShotAttributes::validate()`.
//!    If any slot is empty / missing the call retries ONCE with a
//!    follow-up listing the empty slots. After 2 attempts, errors.
//! 4. Returns `Vec<(shot_id, ShotAttributes)>` — caller merges into
//!    the storyboard.
//!
//! The module ships its own backend trait so the orchestration is
//! decoupled from the wire format. The default `FalAnyLlmBackend` impl
//! lives in [`super::backend`] and routes through `fal-ai/any-llm`.

use serde::{Deserialize, Serialize};

use crate::backends::BackendError;
use crate::storyboard::attributes::ShotAttributes;

use super::prompts::{build_retry_prompt, build_user_prompt, SYSTEM_PROMPT};

/// Brief + shot skeletons fed into the creative director.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DirectorRequest {
    /// Full creative brief — pasted verbatim into the user prompt.
    pub brief: String,
    /// One skeleton per shot. Order is preserved through the call;
    /// returned attributes are keyed by `id` so they can be merged
    /// back onto the storyboard even if the LLM reorders them.
    pub shots: Vec<ShotSkeleton>,
    /// Optional style override applied to every shot (e.g.
    /// `"A24-flavored, 35mm grain, dusk palette"`). Surfaces as a
    /// `STYLE ANCHOR` section in the user prompt.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub style_anchor: Option<String>,
}

/// Minimal per-shot input — id + the two skeleton fields the director
/// always needs (`subject`, `action`). Scene context comes from the
/// brief.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShotSkeleton {
    /// Stable shot id (matches `Shot::id` on the storyboard).
    pub id: String,
    /// What the shot is OF (character name, product, environment).
    pub subject: String,
    /// What's happening within the shot (verb phrase).
    pub action: String,
}

/// Backend abstraction the director calls. One method: take a system
/// prompt + a user message + an optional follow-up retry message, and
/// return the raw model output. The orchestrator handles JSON parsing
/// and validation.
pub trait LlmBackend {
    /// Run a one-shot or two-turn completion. `retry_followup` is
    /// `None` on the first call and `Some(_)` when re-asking after a
    /// validation failure — adapters that prefer multi-turn chat can
    /// thread it through; one-shot adapters concatenate.
    fn complete(
        &self,
        system_prompt: &str,
        user_prompt: &str,
        retry_followup: Option<&str>,
    ) -> Result<String, BackendError>;
}

/// Wire-format JSON returned by the LLM. Tolerates extra top-level
/// fields (reasoning, usage, etc.) but requires `shots`.
#[derive(Debug, Deserialize)]
struct DirectorResponse {
    shots: Vec<DirectorShotAttrs>,
}

/// Per-shot wire object. Required fields parsed strictly; missing
/// fields fail decode with a clear message.
#[derive(Debug, Deserialize)]
struct DirectorShotAttrs {
    id: String,
    subject: String,
    action: String,
    scene: String,
    camera: String,
    lens: String,
    lighting: String,
    style: String,
}

/// Strip common LLM JSON-wrapper noise: leading prose, markdown
/// `\`\`\`json` fences, trailing prose. Finds the first `{` and the
/// matching closing `}`. Returns the candidate JSON slice.
pub(crate) fn extract_json_object(raw: &str) -> &str {
    let trimmed = raw.trim();
    let stripped = trimmed
        .trim_start_matches("```json")
        .trim_start_matches("```JSON")
        .trim_start_matches("```")
        .trim_end_matches("```")
        .trim();
    let Some(start) = stripped.find('{') else {
        return stripped;
    };
    let bytes = stripped.as_bytes();
    let mut depth = 0i32;
    let mut in_str = false;
    let mut escape = false;
    for (i, &b) in bytes.iter().enumerate().skip(start) {
        if escape {
            escape = false;
            continue;
        }
        match b {
            b'\\' if in_str => escape = true,
            b'"' => in_str = !in_str,
            b'{' if !in_str => depth += 1,
            b'}' if !in_str => {
                depth -= 1;
                if depth == 0 {
                    return &stripped[start..=i];
                }
            }
            _ => {}
        }
    }
    &stripped[start..]
}

/// Parse a model response into one `(id, ShotAttributes)` pair per
/// shot. Errors on malformed JSON, missing required slots, or extra
/// shots not present in the request.
fn parse_response(
    raw: &str,
    requested_ids: &[String],
) -> Result<Vec<(String, ShotAttributes)>, BackendError> {
    let json = extract_json_object(raw);
    let parsed: DirectorResponse = serde_json::from_str(json).map_err(|e| {
        BackendError::Decode(format!(
            "director response was not the expected JSON shape: {e}; raw: {snippet}",
            snippet = truncate(raw, 256)
        ))
    })?;

    let mut out = Vec::with_capacity(parsed.shots.len());
    for s in parsed.shots {
        if !requested_ids.iter().any(|r| r == &s.id) {
            return Err(BackendError::Decode(format!(
                "director returned an unknown shot id `{}`",
                s.id
            )));
        }
        out.push((
            s.id,
            ShotAttributes {
                subject: s.subject,
                action: s.action,
                scene: s.scene,
                camera: s.camera,
                lens: s.lens,
                lighting: s.lighting,
                style: s.style,
            },
        ));
    }
    Ok(out)
}

/// Collect `(shot_id, slot_name)` pairs where validation failed —
/// drives the retry follow-up prompt.
fn empty_slots(
    attrs: &[(String, ShotAttributes)],
) -> Vec<(String, &'static str)> {
    let mut out = Vec::new();
    for (id, a) in attrs {
        for (slot, value) in [
            ("subject", &a.subject),
            ("action", &a.action),
            ("scene", &a.scene),
            ("camera", &a.camera),
            ("lens", &a.lens),
            ("lighting", &a.lighting),
            ("style", &a.style),
        ] {
            if value.trim().is_empty() {
                out.push((id.clone(), slot));
            }
        }
    }
    out
}

fn missing_ids(
    requested: &[String],
    returned: &[(String, ShotAttributes)],
) -> Vec<String> {
    requested
        .iter()
        .filter(|id| !returned.iter().any(|(rid, _)| rid == *id))
        .cloned()
        .collect()
}

fn truncate(s: &str, n: usize) -> String {
    if s.len() <= n {
        s.to_string()
    } else {
        format!("{}…", &s[..n])
    }
}

/// Run the creative-director synthesis: one or two LLM calls, validate
/// every returned shot, return `(id, ShotAttributes)` pairs that pass.
/// At most one retry; after two attempts with any unfilled slot, errors.
pub fn synthesize_shot_attributes(
    req: DirectorRequest,
    llm: &dyn LlmBackend,
) -> Result<Vec<(String, ShotAttributes)>, BackendError> {
    if req.shots.is_empty() {
        return Err(BackendError::InvalidRequest(
            "director: shots is empty".into(),
        ));
    }
    let requested_ids: Vec<String> =
        req.shots.iter().map(|s| s.id.clone()).collect();
    let shots_json = serde_json::to_string(&req.shots)
        .map_err(|e| BackendError::Decode(format!("serialize shots: {e}")))?;
    let user_prompt =
        build_user_prompt(&req.brief, &shots_json, req.style_anchor.as_deref());

    let raw = llm.complete(SYSTEM_PROMPT, &user_prompt, None)?;
    let mut attrs = parse_response(&raw, &requested_ids)?;

    let mut missing_slots = empty_slots(&attrs);
    let mut missing = missing_ids(&requested_ids, &attrs);
    if missing_slots.is_empty() && missing.is_empty() {
        return Ok(attrs);
    }

    // One retry: list every empty slot + any missing shot.
    for id in &missing {
        for slot in [
            "subject", "action", "scene", "camera", "lens", "lighting", "style",
        ] {
            missing_slots.push((id.clone(), slot));
        }
    }
    let retry = build_retry_prompt(&missing_slots);
    let raw2 = llm.complete(SYSTEM_PROMPT, &user_prompt, Some(&retry))?;
    attrs = parse_response(&raw2, &requested_ids)?;

    missing_slots = empty_slots(&attrs);
    missing = missing_ids(&requested_ids, &attrs);
    if !missing_slots.is_empty() || !missing.is_empty() {
        return Err(BackendError::Decode(format!(
            "director: validation still failed after retry; empty slots: {missing_slots:?}, missing shots: {missing:?}"
        )));
    }
    Ok(attrs)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    fn make_req(ids: &[&str]) -> DirectorRequest {
        DirectorRequest {
            brief: "Sell a coffee machine.".into(),
            shots: ids
                .iter()
                .map(|id| ShotSkeleton {
                    id: (*id).into(),
                    subject: "espresso machine".into(),
                    action: "extracts a shot".into(),
                })
                .collect(),
            style_anchor: None,
        }
    }

    fn good_response_for(ids: &[&str]) -> String {
        let mut s = String::from("{\n  \"shots\": [\n");
        for (i, id) in ids.iter().enumerate() {
            if i > 0 {
                s.push_str(",\n");
            }
            s.push_str(&format!(
                "    {{\"id\": \"{id}\", \"subject\": \"a chrome espresso machine\", \"action\": \"extracts a double shot\", \"scene\": \"on a marble counter at dawn\", \"camera\": \"MS 85mm, 3/4 front, eye level\", \"lens\": \"shallow DoF, controlled vignette\", \"lighting\": \"key from camera-left, soft fill\", \"style\": \"editorial, warm grade\"}}"
            ));
        }
        s.push_str("\n  ]\n}");
        s
    }

    /// LLM stub that emits a canned response, optionally one for the
    /// first call and a different one on retry.
    struct StubLlm {
        first: String,
        retry: Option<String>,
        calls: RefCell<usize>,
    }

    impl LlmBackend for StubLlm {
        fn complete(
            &self,
            _system_prompt: &str,
            _user_prompt: &str,
            retry_followup: Option<&str>,
        ) -> Result<String, BackendError> {
            *self.calls.borrow_mut() += 1;
            if retry_followup.is_some() {
                Ok(self.retry.clone().unwrap_or_else(|| self.first.clone()))
            } else {
                Ok(self.first.clone())
            }
        }
    }

    #[test]
    fn happy_path_one_call() {
        let req = make_req(&["s-0", "s-1"]);
        let llm = StubLlm {
            first: good_response_for(&["s-0", "s-1"]),
            retry: None,
            calls: RefCell::new(0),
        };
        let out = synthesize_shot_attributes(req, &llm).unwrap();
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].0, "s-0");
        assert_eq!(out[0].1.camera, "MS 85mm, 3/4 front, eye level");
        assert_eq!(*llm.calls.borrow(), 1);
    }

    #[test]
    fn tolerates_markdown_fences() {
        let req = make_req(&["s-0"]);
        let raw = format!(
            "```json\n{}\n```",
            good_response_for(&["s-0"])
        );
        let llm = StubLlm {
            first: raw,
            retry: None,
            calls: RefCell::new(0),
        };
        let out = synthesize_shot_attributes(req, &llm).unwrap();
        assert_eq!(out.len(), 1);
    }

    #[test]
    fn tolerates_leading_prose() {
        let req = make_req(&["s-0"]);
        let raw = format!(
            "Sure, here you go:\n\n{}\n\nLet me know if you want changes.",
            good_response_for(&["s-0"])
        );
        let llm = StubLlm {
            first: raw,
            retry: None,
            calls: RefCell::new(0),
        };
        let out = synthesize_shot_attributes(req, &llm).unwrap();
        assert_eq!(out.len(), 1);
    }

    #[test]
    fn errors_on_missing_required_slot() {
        let req = make_req(&["s-0"]);
        // `lens` missing on first call; retry returns the same thing.
        let bad = "{\"shots\":[{\"id\":\"s-0\",\"subject\":\"x\",\"action\":\"x\",\"scene\":\"x\",\"camera\":\"x\",\"lighting\":\"x\",\"style\":\"x\"}]}".to_string();
        let llm = StubLlm {
            first: bad.clone(),
            retry: Some(bad),
            calls: RefCell::new(0),
        };
        let err = synthesize_shot_attributes(req, &llm).unwrap_err();
        match err {
            BackendError::Decode(msg) => assert!(msg.contains("director response")),
            other => panic!("expected Decode, got {other:?}"),
        }
    }

    #[test]
    fn retries_once_when_slot_is_empty_then_succeeds() {
        let req = make_req(&["s-0"]);
        let empty_lens = "{\"shots\":[{\"id\":\"s-0\",\"subject\":\"x\",\"action\":\"x\",\"scene\":\"x\",\"camera\":\"x\",\"lens\":\"   \",\"lighting\":\"x\",\"style\":\"x\"}]}".to_string();
        let llm = StubLlm {
            first: empty_lens,
            retry: Some(good_response_for(&["s-0"])),
            calls: RefCell::new(0),
        };
        let out = synthesize_shot_attributes(req, &llm).unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(*llm.calls.borrow(), 2);
    }

    #[test]
    fn errors_after_two_attempts_with_empty_slot() {
        let req = make_req(&["s-0"]);
        let empty_lens = "{\"shots\":[{\"id\":\"s-0\",\"subject\":\"x\",\"action\":\"x\",\"scene\":\"x\",\"camera\":\"x\",\"lens\":\"\",\"lighting\":\"x\",\"style\":\"x\"}]}".to_string();
        let llm = StubLlm {
            first: empty_lens.clone(),
            retry: Some(empty_lens),
            calls: RefCell::new(0),
        };
        let err = synthesize_shot_attributes(req, &llm).unwrap_err();
        assert!(matches!(err, BackendError::Decode(_)));
        assert_eq!(*llm.calls.borrow(), 2);
    }

    #[test]
    fn rejects_unknown_shot_id() {
        let req = make_req(&["s-0"]);
        // The LLM hallucinated `s-99`.
        let raw = good_response_for(&["s-99"]);
        let llm = StubLlm {
            first: raw,
            retry: None,
            calls: RefCell::new(0),
        };
        let err = synthesize_shot_attributes(req, &llm).unwrap_err();
        match err {
            BackendError::Decode(m) => assert!(m.contains("unknown shot id")),
            other => panic!("expected Decode, got {other:?}"),
        }
    }

    #[test]
    fn retries_when_shot_is_missing_entirely() {
        let req = make_req(&["s-0", "s-1"]);
        let llm = StubLlm {
            first: good_response_for(&["s-0"]),
            retry: Some(good_response_for(&["s-0", "s-1"])),
            calls: RefCell::new(0),
        };
        let out = synthesize_shot_attributes(req, &llm).unwrap();
        assert_eq!(out.len(), 2);
        assert_eq!(*llm.calls.borrow(), 2);
    }

    #[test]
    fn empty_request_rejected() {
        let req = DirectorRequest {
            brief: "x".into(),
            shots: vec![],
            style_anchor: None,
        };
        let llm = StubLlm {
            first: "{}".into(),
            retry: None,
            calls: RefCell::new(0),
        };
        let err = synthesize_shot_attributes(req, &llm).unwrap_err();
        assert!(matches!(err, BackendError::InvalidRequest(_)));
        assert_eq!(*llm.calls.borrow(), 0);
    }

    #[test]
    fn extract_json_handles_nested_braces() {
        let raw = "prose\n```json\n{\"shots\":[{\"id\":\"a\",\"meta\":{\"x\":1}}]}\n```\ntrailing";
        let j = extract_json_object(raw);
        assert!(j.starts_with('{'));
        assert!(j.ends_with('}'));
        assert!(j.contains("\"x\":1"));
    }

    #[test]
    fn extract_json_ignores_braces_inside_strings() {
        let raw = "{\"shots\":[{\"id\":\"a}b\",\"subject\":\"{not-json}\"}]}";
        let j = extract_json_object(raw);
        assert_eq!(j, raw);
    }

    #[test]
    fn parse_response_round_trips() {
        let raw = good_response_for(&["s-0"]);
        let parsed =
            parse_response(&raw, &["s-0".to_string()]).unwrap();
        assert_eq!(parsed.len(), 1);
        let (id, attrs) = &parsed[0];
        assert_eq!(id, "s-0");
        assert!(attrs.validate().is_ok());
    }

    #[test]
    fn tolerates_extra_top_level_fields() {
        let raw = "{\"reasoning\":\"long thought\",\"usage\":{\"tokens\":42},\"shots\":[{\"id\":\"s-0\",\"subject\":\"x\",\"action\":\"x\",\"scene\":\"x\",\"camera\":\"x\",\"lens\":\"x\",\"lighting\":\"x\",\"style\":\"x\"}]}";
        let parsed = parse_response(raw, &["s-0".into()]).unwrap();
        assert_eq!(parsed.len(), 1);
    }
}
