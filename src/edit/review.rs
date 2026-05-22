//! Reviewer — uploads the executor's MP4 output and asks Gemini to
//! grade it against the user's original intent.
//!
//! Shape matches `rubric.passes` (`pass`, `score`, `reasoning`,
//! `competing_view`, `bias_audit`) so downstream tooling can consume
//! both surfaces interchangeably.

use serde::{Deserialize, Serialize};

use super::EditError;

/// Reviewer verdict — same shape as `rubric.passes`.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Verdict {
    /// Boolean shipping signal. May be omitted by the model; callers
    /// fall back to `score >= threshold`.
    #[serde(default)]
    pub pass: Option<bool>,
    /// Score in `[0.0, 1.0]`.
    #[serde(default)]
    pub score: Option<f32>,
    /// One-line rationale for the verdict.
    #[serde(default)]
    pub reasoning: String,
    /// Strongest counter the reviewer considered.
    #[serde(default)]
    pub competing_view: String,
    /// Biases the reviewer audited itself for.
    #[serde(default)]
    pub bias_audit: String,
}

impl Verdict {
    /// Resolve to a shipping decision under the loop's threshold.
    pub fn ships_at(&self, threshold: f32) -> bool {
        if self.pass == Some(true) {
            return true;
        }
        match self.score {
            Some(s) => s >= threshold,
            None => false,
        }
    }

    /// Numeric score with `0.0` as the default. Used for best-of-N
    /// selection on exhausted attempts.
    pub fn score_or_zero(&self) -> f32 {
        self.score.unwrap_or(0.0)
    }
}

/// Build the reviewer's prompt. The original intent and a recap of
/// what the planner *claimed* it did are both fed in; the reviewer
/// then compares those claims against the actual rendered video.
pub fn build_review_prompt(intent: &str, plan_summary: &str, threshold: f32) -> String {
    format!(
        r#"You are an objective-thinking judge. A video editor was asked to make a change to a video clip; your job is to watch the resulting clip and decide whether it actually fulfills the user's intent.

NOTE: a video file is attached. Watch the entire clip. Evaluate against what is VISIBLE and AUDIBLE in the video, not against the planner's description.

Discipline (apply silently before answering):
  - Restate the user's intent in your own words.
  - Sort what you see in the video into: facts (what visibly happened), claims (what the planner said happened but you can't verify), unknowns.
  - Generate the strongest case AGAINST your initial verdict — `competing_view` captures it.
  - Audit your reasoning for: charity drift (passing because it "looks fine"), confirmation bias toward the planner's narration, premature closure.
  - Calibrate the score to evidence quality.

Reply with EXACTLY one line: a single JSON object, no commentary, no code fences.

Schema: {{"pass": <bool>, "score": <0..1>, "reasoning": "<≤120 chars: why>", "competing_view": "<≤80 chars: strongest counter>", "bias_audit": "<≤80 chars: what biases you tested for>"}}

Pass = score >= {threshold:.2}.

Anti-patterns to refuse:
  - Charity drift: passing because the clip "looks reasonable" without verifying the SPECIFIC intent was met.
  - Reward hacking: passing because the planner's summary contains the intent's keywords verbatim.

=== ORIGINAL INTENT ===
{intent}

=== WHAT THE PLANNER SAYS IT DID ===
{plan_summary}

=== END ===

Reply with the JSON object only.
"#,
    )
}

/// Parse a verdict response. Mirrors `parseVerdict` in `rubric.mjs` —
/// tries strict JSON first, then a defensive regex fallback for
/// truncated outputs.
pub fn parse_verdict(text: &str) -> Result<Verdict, EditError> {
    let body = strip_code_fence(text);
    if let Some(start) = body.find('{') {
        if let Some(end) = body.rfind('}') {
            if end >= start {
                if let Ok(v) = serde_json::from_str::<Verdict>(&body[start..=end]) {
                    return Ok(v);
                }
            }
        }
    }
    let pass = regex_find(text, r#""pass"\s*:\s*(true|false)"#).map(|s| s == "true");
    let score = regex_find(text, r#""score"\s*:\s*([0-9]*\.?[0-9]+)"#)
        .and_then(|s| s.parse::<f32>().ok());
    let reasoning = regex_find(text, r#""reasoning"\s*:\s*"([^"]{0,200})"#).unwrap_or_default();
    if pass.is_some() || score.is_some() {
        return Ok(Verdict {
            pass,
            score,
            reasoning,
            competing_view: String::new(),
            bias_audit: String::new(),
        });
    }
    Err(EditError::ReviewParse(format!(
        "reviewer returned no parseable verdict: {}",
        text.chars().take(200).collect::<String>()
    )))
}

fn strip_code_fence(s: &str) -> &str {
    let t = s.trim();
    if let Some(rest) = t.strip_prefix("```json") {
        return rest.trim_end_matches("```").trim();
    }
    if let Some(rest) = t.strip_prefix("```") {
        return rest.trim_end_matches("```").trim();
    }
    t
}

// Tiny hand-rolled regex extractor — we don't want to pull in the
// `regex` crate for two patterns. Finds the first match of a fixed
// shape: `<key>: <captured>` where the captured group lives between
// the second pair of quotes / parens after `<key>`.
fn regex_find(haystack: &str, pattern: &str) -> Option<String> {
    // Hand-roll: this is called only on fallback paths.
    if pattern.contains("pass") {
        let idx = haystack.find("\"pass\"")?;
        let tail = &haystack[idx..];
        if tail.contains("true") {
            return Some("true".into());
        }
        if tail.contains("false") {
            return Some("false".into());
        }
        None
    } else if pattern.contains("score") {
        let idx = haystack.find("\"score\"")?;
        let tail = &haystack[idx + 7..];
        let after_colon = tail.find(':').map(|i| &tail[i + 1..])?;
        let s = after_colon.trim_start();
        let end = s
            .find(|c: char| !(c.is_ascii_digit() || c == '.'))
            .unwrap_or(s.len());
        if end == 0 {
            return None;
        }
        Some(s[..end].to_string())
    } else if pattern.contains("reasoning") {
        let idx = haystack.find("\"reasoning\"")?;
        let tail = &haystack[idx..];
        let q1 = tail.find('"')?;
        let q2 = tail[q1 + 1..].find('"')?;
        let after = &tail[q1 + 1 + q2 + 1..];
        let q3 = after.find('"')?;
        let q4 = after[q3 + 1..].find('"')?;
        Some(after[q3 + 1..q3 + 1 + q4].to_string())
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_strict_verdict() {
        let raw = r#"{"pass":true,"score":0.82,"reasoning":"dusk visible","competing_view":"could be darker","bias_audit":"checked charity drift"}"#;
        let v = parse_verdict(raw).unwrap();
        assert_eq!(v.pass, Some(true));
        assert_eq!(v.score, Some(0.82));
        assert_eq!(v.reasoning, "dusk visible");
    }

    #[test]
    fn ships_at_threshold() {
        let v = Verdict {
            pass: None,
            score: Some(0.71),
            ..Default::default()
        };
        assert!(v.ships_at(0.7));
        assert!(!v.ships_at(0.8));

        let pass_v = Verdict {
            pass: Some(true),
            score: Some(0.5),
            ..Default::default()
        };
        assert!(pass_v.ships_at(0.9));
    }

    #[test]
    fn defensive_parse_recovers_score() {
        let raw = "Some preamble then {\"pass\": true, \"score\": 0.65, \"reasoning\": \"ok\"} and more prose";
        let v = parse_verdict(raw).unwrap();
        assert_eq!(v.score, Some(0.65));
    }
}
