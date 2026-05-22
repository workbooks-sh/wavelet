//! `rubric_passes` — the only validator that costs money.
//!
//! Sends `{rubric.prompt, must_satisfy[]}` and the artifact (image or
//! text) to Gemini under the `objective-thinking` discipline:
//! stage-based reasoning (frame → evidence → hypotheses → adversarial
//! check → synthesis → audit) with an explicit per-clause verdict
//! contract. The judge returns structured JSON `{clauses: [{clause,
//! verdict: "pass"|"fail", evidence}], overall: "pass"|"fail"}`.
//!
//! Objective validators (artifact_exists, query.*, comp_verify_passes,
//! c2pa_verify_passes, unit_test_passes) should be ordered BEFORE
//! `rubric_passes` on each task. The dispatcher in `validator::check_all`
//! runs them in declared order so a cheap upstream failure short-circuits
//! the expensive vision call and saves spend.
//!
//! Missing `GEMINI_API_KEY` / `GOOGLE_API_KEY` returns a structured
//! failure rather than panicking — lets CI skip without crashing.

use std::time::Instant;

use serde_json::{json, Value};

use super::super::validator::{Validator, ValidatorCtx, ValidatorOutcome};

/// Coarse per-call cost (USD) imputed to `gemini-3.5-flash` vision
/// requests of this rough token shape. Used only as a budget signal —
/// the upstream cost meter folds it into the running session total.
const GEMINI_FLASH_COST_USD: f32 = 0.005;

pub struct RubricPasses;

impl Validator for RubricPasses {
    fn kind(&self) -> &'static str { "rubric_passes" }

    fn check(&self, params: &serde_yaml::Value, ctx: &ValidatorCtx) -> ValidatorOutcome {
        let start = Instant::now();
        let Some(artifact) = params.get("artifact").and_then(|v| v.as_str()) else {
            return fail(json!({"error": "missing_param", "param": "artifact"}), start, 0.0);
        };
        let rubric = match params.get("rubric") {
            Some(r) => r,
            None => return fail(json!({"error": "missing_param", "param": "rubric"}), start, 0.0),
        };
        let prompt = rubric
            .get("prompt")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let must_satisfy: Vec<String> = rubric
            .get("must_satisfy")
            .and_then(|v| v.as_sequence())
            .map(|s| s.iter().filter_map(|x| x.as_str().map(String::from)).collect())
            .unwrap_or_default();
        if must_satisfy.is_empty() {
            return fail(
                json!({"error": "empty_must_satisfy", "rubric": serde_json::to_value(rubric).unwrap_or(Value::Null)}),
                start,
                0.0,
            );
        }
        let model = params
            .get("model")
            .and_then(|v| v.as_str())
            .unwrap_or("gemini-3.5-flash")
            .to_string();

        // Resolve the artifact path against workdir.
        let artifact_path = ctx.workdir.join(artifact);
        let artifact_bytes = match std::fs::read(&artifact_path) {
            Ok(b) => b,
            Err(e) => {
                return fail(
                    json!({
                        "error": "artifact_read_failed",
                        "artifact": artifact,
                        "reason": e.to_string(),
                    }),
                    start,
                    0.0,
                );
            }
        };

        let api_key = match std::env::var("GEMINI_API_KEY")
            .or_else(|_| std::env::var("GOOGLE_API_KEY"))
        {
            Ok(k) if !k.is_empty() => k,
            _ => {
                return fail(
                    json!({
                        "error": "no_api_key",
                        "reason": "GEMINI_API_KEY not set",
                        "artifact": artifact,
                        "model": model,
                    }),
                    start,
                    0.0,
                );
            }
        };

        let mime = guess_mime(artifact);
        let b64 = base64_encode(&artifact_bytes);

        let judge_prompt = build_judge_prompt(prompt, &must_satisfy);

        let body = json!({
            "contents": [{
                "role": "user",
                "parts": [
                    { "text": judge_prompt },
                    { "inline_data": { "mime_type": mime, "data": b64 } }
                ]
            }],
            "generationConfig": {
                "response_mime_type": "application/json",
                "thinkingConfig": { "thinkingLevel": "low" }
            }
        });

        let url = format!(
            "https://generativelanguage.googleapis.com/v1beta/models/{model}:generateContent?key={api_key}"
        );

        let resp = match ureq::post(&url)
            .set("Content-Type", "application/json")
            .send_string(&body.to_string())
        {
            Ok(r) => r,
            Err(e) => {
                return fail(
                    json!({
                        "error": "gemini_request_failed",
                        "reason": e.to_string(),
                        "model": model,
                    }),
                    start,
                    GEMINI_FLASH_COST_USD,
                );
            }
        };

        let v: Value = match resp.into_json() {
            Ok(j) => j,
            Err(e) => {
                return fail(
                    json!({
                        "error": "gemini_response_parse_failed",
                        "reason": e.to_string(),
                    }),
                    start,
                    GEMINI_FLASH_COST_USD,
                );
            }
        };

        let text = extract_text(&v).unwrap_or_default();
        let verdict: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
        let clauses = verdict.get("clauses").and_then(|c| c.as_array()).cloned().unwrap_or_default();
        let overall_pass = verdict
            .get("overall")
            .and_then(|o| o.as_str())
            .map(|s| s.eq_ignore_ascii_case("pass"))
            .unwrap_or(false);
        let all_clause_pass = !clauses.is_empty()
            && clauses.iter().all(|c| {
                c.get("verdict")
                    .and_then(|v| v.as_str())
                    .map(|s| s.eq_ignore_ascii_case("pass"))
                    .unwrap_or(false)
            });
        let ok = overall_pass && all_clause_pass;

        let detail = if ok {
            json!({
                "model": model,
                "artifact": artifact,
                "clauses": clauses,
                "overall": "pass",
            })
        } else {
            let failing: Vec<&Value> = clauses
                .iter()
                .filter(|c| {
                    c.get("verdict").and_then(|v| v.as_str()).map(|s| !s.eq_ignore_ascii_case("pass")).unwrap_or(true)
                })
                .collect();
            json!({
                "model": model,
                "artifact": artifact,
                "failed_clause": "rubric_pass",
                "clauses": clauses,
                "failing_clauses": failing,
                "overall": verdict.get("overall").cloned().unwrap_or(Value::Null),
                "raw_judge_text": text,
            })
        };

        ValidatorOutcome {
            ok,
            detail,
            cost_usd: GEMINI_FLASH_COST_USD,
            wall_ms: start.elapsed().as_millis(),
        }
    }
}

fn build_judge_prompt(rubric_prompt: &str, must_satisfy: &[String]) -> String {
    let clauses = must_satisfy
        .iter()
        .enumerate()
        .map(|(i, c)| format!("  {}. {}", i + 1, c))
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        r#"You are a disciplined judge agent operating under the objective-thinking protocol.

Rules:
1. First impressions are suspect. Do not commit to a verdict before listing evidence.
2. Keep evidence types separate: observed facts, inferences, assumptions.
3. Generate at least one competing hypothesis before committing.
4. Hunt for disconfirming evidence — what would falsify your leading view?
5. Prefer explicit criteria over stylistic fluency.

Stages (work through internally, then summarize in the JSON output):
  frame → evidence → hypotheses → adversarial check → synthesis → audit

Rubric prompt (judge against this):
{rubric_prompt}

Must-satisfy clauses (each must independently pass for an overall pass):
{clauses}

Output a single JSON object — no prose before or after — matching:
{{
  "clauses": [
    {{ "clause": "<verbatim clause text>", "verdict": "pass"|"fail", "evidence": "<short cite of what in the artifact supports the verdict>" }}
  ],
  "overall": "pass" | "fail",
  "audit_note": "<one sentence on what would change your verdict>"
}}

A clause's verdict is "fail" if you cannot find direct evidence in the artifact that satisfies it. Charitable interpretation is forbidden — if it is ambiguous, the verdict is "fail".
"#
    )
}

fn extract_text(v: &Value) -> Option<String> {
    let parts = v
        .get("candidates")?
        .as_array()?
        .first()?
        .get("content")?
        .get("parts")?
        .as_array()?;
    let mut out = String::new();
    for p in parts {
        if let Some(t) = p.get("text").and_then(|s| s.as_str()) {
            out.push_str(t);
        }
    }
    if out.is_empty() { None } else { Some(out) }
}

fn guess_mime(path: &str) -> &'static str {
    let lower = path.to_ascii_lowercase();
    if lower.ends_with(".png") { "image/png" }
    else if lower.ends_with(".jpg") || lower.ends_with(".jpeg") { "image/jpeg" }
    else if lower.ends_with(".webp") { "image/webp" }
    else if lower.ends_with(".gif") { "image/gif" }
    else if lower.ends_with(".mp4") { "video/mp4" }
    else if lower.ends_with(".mov") { "video/quicktime" }
    else if lower.ends_with(".txt") || lower.ends_with(".md") { "text/plain" }
    else { "application/octet-stream" }
}

fn base64_encode(bytes: &[u8]) -> String {
    const ALPHA: &[u8; 64] =
        b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity((bytes.len() + 2) / 3 * 4);
    let mut i = 0;
    while i + 3 <= bytes.len() {
        let n = ((bytes[i] as u32) << 16) | ((bytes[i + 1] as u32) << 8) | (bytes[i + 2] as u32);
        out.push(ALPHA[((n >> 18) & 0x3F) as usize] as char);
        out.push(ALPHA[((n >> 12) & 0x3F) as usize] as char);
        out.push(ALPHA[((n >> 6) & 0x3F) as usize] as char);
        out.push(ALPHA[(n & 0x3F) as usize] as char);
        i += 3;
    }
    let rem = bytes.len() - i;
    if rem == 1 {
        let n = (bytes[i] as u32) << 16;
        out.push(ALPHA[((n >> 18) & 0x3F) as usize] as char);
        out.push(ALPHA[((n >> 12) & 0x3F) as usize] as char);
        out.push('=');
        out.push('=');
    } else if rem == 2 {
        let n = ((bytes[i] as u32) << 16) | ((bytes[i + 1] as u32) << 8);
        out.push(ALPHA[((n >> 18) & 0x3F) as usize] as char);
        out.push(ALPHA[((n >> 12) & 0x3F) as usize] as char);
        out.push(ALPHA[((n >> 6) & 0x3F) as usize] as char);
        out.push('=');
    }
    out
}

fn fail(detail: Value, start: Instant, cost_usd: f32) -> ValidatorOutcome {
    ValidatorOutcome {
        ok: false,
        detail,
        cost_usd,
        wall_ms: start.elapsed().as_millis(),
    }
}
