//! Prompt templates for the LLM-as-creative-director synthesis step
//! (wb-epk3). The system prompt is byte-stable — a test snapshot locks
//! it. Don't paraphrase or "clean up" the wording; minor changes shift
//! model behavior in ways that only show up at smoke time.

/// System prompt sent to the LLM. The output schema is locked: one JSON
/// object with a top-level `shots` array of `{id, subject, action,
/// scene, camera, lens, lighting, style}` objects, every slot a string.
pub const SYSTEM_PROMPT: &str = "You are a creative director for AI-generated commercials. Given a brief and a shot skeleton, return a JSON object with seven slots filled for every shot:

- subject (what the shot is OF)
- action (what is happening)
- scene (where it is)
- camera (framing: ECU / CU / MS / MWS / WS / EWS, angle, focal length)
- lens (optical character: anamorphic, spherical, DoF, aberration notes)
- lighting (direction + quality, time of day)
- style (aesthetic register, references)

Rules:
- Every slot is required for every shot. Never leave a slot empty. If a slot is genuinely under-determined, write \"unspecified\" — the downstream verify gate will flag it.
- Stay consistent across shots in a spot: same color palette, same lens family, same lighting time-of-day unless the brief says otherwise.
- Be specific. \"A modern office\" is wrong. \"A glass-walled conference room at golden hour, brutalist concrete columns visible through the windows\" is right.
- The output is consumed by Seedream / Nano Banana Pro / Veo image-to-video — write to that idiom (concrete nouns, present tense, no \"shows that\" / \"depicts\" language).

Return ONLY the JSON object. Schema:
{
  \"shots\": [
    {\"id\": \"<shot-id>\", \"subject\": \"...\", \"action\": \"...\", \"scene\": \"...\", \"camera\": \"...\", \"lens\": \"...\", \"lighting\": \"...\", \"style\": \"...\"}
  ]
}";

/// Serialize a brief + shot skeleton list + optional style anchor into
/// the user-prompt JSON payload. Stable field order so cached requests
/// hash identically across runs.
pub fn build_user_prompt(
    brief: &str,
    shots_json: &str,
    style_anchor: Option<&str>,
) -> String {
    match style_anchor {
        Some(anchor) if !anchor.trim().is_empty() => format!(
            "BRIEF:\n{brief}\n\nSTYLE ANCHOR (applies to every shot):\n{anchor}\n\nSHOTS:\n{shots_json}"
        ),
        _ => format!("BRIEF:\n{brief}\n\nSHOTS:\n{shots_json}"),
    }
}

/// Build a retry follow-up prompt that names the slots a previous
/// response left empty / failed validation on. Keeps the model honest
/// about which slots to refill.
pub fn build_retry_prompt(missing: &[(String, &'static str)]) -> String {
    let mut p = String::from(
        "Your previous response left required slots empty or unfilled. Return the same JSON shape, but fill these slots (write \"unspecified\" only if the brief genuinely doesn't constrain them):\n",
    );
    for (shot_id, slot) in missing {
        p.push_str(&format!("- shot {shot_id}: {slot}\n"));
    }
    p
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_prompt_byte_stable() {
        // Lock the system prompt. Changing it without intent shifts
        // model behavior; this test surfaces the change in review.
        assert!(SYSTEM_PROMPT.starts_with(
            "You are a creative director for AI-generated commercials."
        ));
        assert!(SYSTEM_PROMPT.contains("- subject (what the shot is OF)"));
        assert!(SYSTEM_PROMPT.contains("- action (what is happening)"));
        assert!(SYSTEM_PROMPT.contains("- scene (where it is)"));
        assert!(SYSTEM_PROMPT.contains("- camera (framing:"));
        assert!(SYSTEM_PROMPT.contains("- lens (optical character:"));
        assert!(SYSTEM_PROMPT.contains("- lighting (direction + quality"));
        assert!(SYSTEM_PROMPT.contains("- style (aesthetic register"));
        assert!(SYSTEM_PROMPT.contains("Return ONLY the JSON object."));
        assert!(SYSTEM_PROMPT.contains("\"shots\""));
        // Byte length: minor non-semantic edits (a stray space) will
        // also fail this. Update when intentionally rewording.
        assert_eq!(SYSTEM_PROMPT.len(), 1318);
    }

    #[test]
    fn user_prompt_without_anchor_omits_section() {
        let p = build_user_prompt("Sell shoes.", "[]", None);
        assert!(p.contains("BRIEF:\nSell shoes."));
        assert!(p.contains("SHOTS:\n[]"));
        assert!(!p.contains("STYLE ANCHOR"));
    }

    #[test]
    fn user_prompt_with_anchor_includes_section() {
        let p = build_user_prompt("Sell shoes.", "[]", Some("A24-flavored"));
        assert!(p.contains("STYLE ANCHOR (applies to every shot):\nA24-flavored"));
    }

    #[test]
    fn user_prompt_whitespace_anchor_treated_as_none() {
        let p = build_user_prompt("Sell shoes.", "[]", Some("   "));
        assert!(!p.contains("STYLE ANCHOR"));
    }

    #[test]
    fn retry_prompt_names_each_missing_slot() {
        let missing = vec![
            ("shot-0-0".to_string(), "lens"),
            ("shot-0-1".to_string(), "lighting"),
        ];
        let p = build_retry_prompt(&missing);
        assert!(p.contains("- shot shot-0-0: lens"));
        assert!(p.contains("- shot shot-0-1: lighting"));
        assert!(p.contains("\"unspecified\""));
    }
}
