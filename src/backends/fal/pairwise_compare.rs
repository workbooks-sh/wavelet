//! Fal any-llm/vision adapter — `PairwiseCompare` cluster (VISTA).
//!
//! Implements the structured pairwise judging step from
//! [arXiv 2510.15831](https://arxiv.org/abs/2510.15831). The VLM looks
//! at two candidate frames side-by-side, picks a winner per dimension
//! across {subject fidelity, composition, lighting + color, production
//! polish}, and returns strict JSON the bracket runner consumes.
//!
//! Wire format mirrors `vision_verify`: `fal-ai/any-llm/vision` with a
//! routed model + JSON-only prompt. The prompt is load-bearing — every
//! word was tuned in the VISTA paper, so do not casually rewrite.
//!
//! Cost: ~$0.01 per pair at the default routed model (Gemini Flash
//! Lite). For a 4-candidate bracket = 3 pairs = ~$0.03.
//!
//! Returned response shape (verbatim, in JSON):
//! ```json
//! {
//!   "subject_fidelity": "A" | "B" | "tie",
//!   "composition":      "A" | "B" | "tie",
//!   "lighting_color":   "A" | "B" | "tie",
//!   "production":       "A" | "B" | "tie",
//!   "rationale": "<one sentence per dimension, max 4 sentences total>"
//! }
//! ```

use crate::backends::cache::{utc_now_iso8601, AssetCache, Manifest};
use crate::backends::fal::FalClient;
use crate::backends::image::{
    PairwiseCompareBackend, PairwiseCompareRequest, PairwiseCompareResult, CLUSTER_PAIRWISE_COMPARE,
};
use crate::backends::{
    check_budget, mode_label, BackendCallOutcome, BackendError, CostEstimate, RunMode,
};
use serde::{Deserialize, Serialize};

/// Provider id.
pub const PROVIDER: &str = "fal-pairwise-compare";

/// Fal model path (the router).
pub const MODEL_PATH: &str = "fal-ai/any-llm/vision";

/// Default routed VLM — same default as `vision_verify`. JSON-mode
/// works best on Gemini family; override with `VLM_MODEL` env var when
/// probing another routed model.
pub const DEFAULT_VLM: &str = "google/gemini-2.5-flash-lite";

/// Env var to override the routed model.
pub const VLM_MODEL_ENV: &str = "VLM_MODEL";

/// Per-call cost estimate (USD). Conservative.
pub const PRICE_PER_CALL_USD: f32 = 0.01;

/// Fal pairwise-compare adapter.
#[derive(Debug, Clone)]
pub struct FalPairwiseCompareAdapter {
    client: FalClient,
}

impl FalPairwiseCompareAdapter {
    /// Build from a pre-constructed client.
    pub fn new(client: FalClient) -> Self {
        Self { client }
    }

    fn routed_model() -> String {
        std::env::var(VLM_MODEL_ENV)
            .ok()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| DEFAULT_VLM.into())
    }
}

/// Build the VISTA-style judging prompt. The wording is load-bearing —
/// it asks the VLM for strict JSON across four dimensions plus a short
/// rationale. Per-dimension verdict is `"A"`, `"B"`, or `"tie"`.
pub(crate) fn build_prompt(brief_excerpt: &str, shot_prompt: &str) -> String {
    format!(
        "You are judging two candidate frames for an AI-generated commercial.\n\n\
Brief context: {brief}\n\
Shot intent: {shot}\n\n\
Score each candidate independently on these four dimensions, then declare a winner per dimension. Output JSON only:\n\n\
{{\n  \"subject_fidelity\": \"A\" | \"B\" | \"tie\",\n  \"composition\":      \"A\" | \"B\" | \"tie\",\n  \"lighting_color\":   \"A\" | \"B\" | \"tie\",\n  \"production\":       \"A\" | \"B\" | \"tie\",\n  \"rationale\": \"<one sentence per dimension, max 4 sentences total>\"\n}}\n\n\
A is the first image, B is the second. Tie only when truly indistinguishable. Be decisive.",
        brief = brief_excerpt,
        shot = shot_prompt,
    )
}

impl PairwiseCompareBackend for FalPairwiseCompareAdapter {
    fn name(&self) -> &'static str {
        PROVIDER
    }

    fn estimate_cost(&self, _: &PairwiseCompareRequest) -> CostEstimate {
        CostEstimate {
            provider: PROVIDER.into(),
            cost_usd: PRICE_PER_CALL_USD,
            explanation: format!("${PRICE_PER_CALL_USD:.4}/pair (any-llm/vision router, VISTA)"),
        }
    }

    fn compare(
        &self,
        request: &PairwiseCompareRequest,
        mode: RunMode,
    ) -> Result<BackendCallOutcome<PairwiseCompareResult>, BackendError> {
        if request.image_a_url.trim().is_empty() || request.image_b_url.trim().is_empty() {
            return Err(BackendError::InvalidRequest(
                "image_a_url and image_b_url are required".into(),
            ));
        }

        let estimate = self.estimate_cost(request);
        check_budget(&estimate, mode)?;

        let request_hash =
            AssetCache::request_hash(PROVIDER, CLUSTER_PAIRWISE_COMPARE, request)?;
        let cache = self.client.cache();

        if let Some(manifest) = cache.hit(PROVIDER, &request_hash)? {
            let response: PairwiseCompareResult =
                serde_json::from_value(manifest.response.clone())
                    .map_err(|e| BackendError::Cache(format!("decode cached response: {e}")))?;
            return Ok(BackendCallOutcome {
                response,
                provider: PROVIDER.into(),
                request_hash,
                cached: true,
                cost_estimate_usd: 0.0,
                mode: mode_label(mode),
            });
        }

        if !mode.is_live() {
            // Dry-run: declare a tie across every dimension so the
            // shape is identical to a live response. The bracket
            // runner's seed-tiebreak path will pick A for stable diffs.
            let response = PairwiseCompareResult {
                provider: PROVIDER.into(),
                subject_fidelity: "tie".into(),
                composition: "tie".into(),
                lighting_color: "tie".into(),
                production: "tie".into(),
                rationale: "dry-run: not judged".into(),
            };
            return Ok(BackendCallOutcome {
                response,
                provider: PROVIDER.into(),
                request_hash,
                cached: false,
                cost_estimate_usd: estimate.cost_usd,
                mode: mode_label(mode),
            });
        }

        let body = AnyLlmPairwiseBody {
            prompt: build_prompt(&request.brief_excerpt, &request.shot_prompt),
            image_url: request.image_a_url.clone(),
            image_url_2: request.image_b_url.clone(),
            model: Self::routed_model(),
        };
        let parsed: AnyLlmPairwiseResponse = self.client.post_sync(MODEL_PATH, &body)?;
        if let Some(err) = parsed.error.as_ref() {
            return Err(BackendError::HttpStatus {
                status: 502,
                body: err.clone(),
            });
        }
        let result = parse_pairwise(&parsed.output)?;

        let manifest = Manifest {
            version: 1,
            provider: PROVIDER.into(),
            cluster: CLUSTER_PAIRWISE_COMPARE.into(),
            request_hash: request_hash.clone(),
            request: serde_json::to_value(request)
                .map_err(|e| BackendError::Cache(format!("serialize request for cache: {e}")))?,
            response: serde_json::to_value(&result).map_err(|e| {
                BackendError::Cache(format!("serialize response for cache: {e}"))
            })?,
            cost_estimate_usd: estimate.cost_usd,
            asset_path: None,
            created_at: utc_now_iso8601(),
        };
        cache.store(&manifest)?;

        Ok(BackendCallOutcome {
            response: result,
            provider: PROVIDER.into(),
            request_hash,
            cached: false,
            cost_estimate_usd: estimate.cost_usd,
            mode: mode_label(mode),
        })
    }
}

#[derive(Debug, Serialize)]
struct AnyLlmPairwiseBody {
    prompt: String,
    image_url: String,
    image_url_2: String,
    model: String,
}

#[derive(Debug, Deserialize)]
struct AnyLlmPairwiseResponse {
    #[serde(default)]
    output: String,
    #[serde(default)]
    error: Option<String>,
}

/// Parse the VLM's JSON response into a `PairwiseCompareResult`. Tolerates
/// fenced code blocks (`json ...`) and extra prose by extracting the
/// first balanced `{...}` block. Each verdict field is normalized to
/// `"A"` / `"B"` / `"tie"`; anything else returns an error so the
/// caller knows the response was malformed (the bracket runner will
/// surface this and the pair-aggregator's tie path will keep the
/// tournament moving).
pub(crate) fn parse_pairwise(raw: &str) -> Result<PairwiseCompareResult, BackendError> {
    let blob = extract_json_blob(raw).ok_or_else(|| {
        BackendError::InvalidRequest(format!("pairwise: no JSON object in VLM output: {raw}"))
    })?;
    let parsed: ParsedPairwiseJson = serde_json::from_str(&blob).map_err(|e| {
        BackendError::InvalidRequest(format!(
            "pairwise: VLM output not parseable as JSON: {e} (raw: {blob})"
        ))
    })?;
    Ok(PairwiseCompareResult {
        provider: PROVIDER.into(),
        subject_fidelity: normalize_verdict(&parsed.subject_fidelity)?,
        composition: normalize_verdict(&parsed.composition)?,
        lighting_color: normalize_verdict(&parsed.lighting_color)?,
        production: normalize_verdict(&parsed.production)?,
        rationale: parsed.rationale.unwrap_or_default(),
    })
}

#[derive(Debug, Deserialize)]
struct ParsedPairwiseJson {
    #[serde(default)]
    subject_fidelity: String,
    #[serde(default)]
    composition: String,
    #[serde(default)]
    lighting_color: String,
    #[serde(default)]
    production: String,
    #[serde(default)]
    rationale: Option<String>,
}

fn normalize_verdict(s: &str) -> Result<String, BackendError> {
    let t = s.trim().to_ascii_lowercase();
    let normalized = match t.as_str() {
        "a" => "A",
        "b" => "B",
        "tie" | "draw" | "equal" => "tie",
        other => {
            return Err(BackendError::InvalidRequest(format!(
                "pairwise: unknown verdict '{other}', want A/B/tie"
            )));
        }
    };
    Ok(normalized.to_string())
}

fn extract_json_blob(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    let stripped = trimmed
        .trim_start_matches("```json")
        .trim_start_matches("```")
        .trim_end_matches("```")
        .trim();
    let bytes = stripped.as_bytes();
    let start = bytes.iter().position(|b| *b == b'{')?;
    // Walk to the matching brace, respecting string literals.
    let mut depth = 0i32;
    let mut in_string = false;
    let mut escape = false;
    for (i, b) in bytes.iter().enumerate().skip(start) {
        if in_string {
            if escape {
                escape = false;
            } else if *b == b'\\' {
                escape = true;
            } else if *b == b'"' {
                in_string = false;
            }
            continue;
        }
        match *b {
            b'"' => in_string = true,
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(stripped[start..=i].to_string());
                }
            }
            _ => {}
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fresh_cache() -> std::path::PathBuf {
        let tmp = std::env::temp_dir().join(format!(
            "wavelet-fal-pairwise-{}",
            AssetCache::request_hash("seed", "seed", &"x").unwrap()
        ));
        let _ = std::fs::remove_dir_all(&tmp);
        tmp
    }

    #[test]
    fn request_round_trips() {
        let req = PairwiseCompareRequest::new(
            "https://example.com/a.png",
            "https://example.com/b.png",
            "a green Porsche 911 GT3, dusk light, dramatic mood",
            "low-angle shot from below the front fender, motion blur on tires",
        );
        let json = serde_json::to_string(&req).unwrap();
        let back: PairwiseCompareRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(back.image_a_url, "https://example.com/a.png");
        assert_eq!(back.image_b_url, "https://example.com/b.png");
        assert!(back.brief_excerpt.contains("Porsche"));
    }

    #[test]
    fn prompt_contains_load_bearing_phrases() {
        let p = build_prompt("brand X", "low-angle shot");
        assert!(p.contains("Output JSON only"));
        assert!(p.contains("subject_fidelity"));
        assert!(p.contains("composition"));
        assert!(p.contains("lighting_color"));
        assert!(p.contains("production"));
        assert!(p.contains("A is the first image"));
        assert!(p.contains("brand X"));
        assert!(p.contains("low-angle shot"));
    }

    #[test]
    fn parse_clean_json_block() {
        let raw = r#"{"subject_fidelity":"A","composition":"B","lighting_color":"tie","production":"A","rationale":"A nails identity. B framing slightly better. Lighting equal. A sharper."}"#;
        let r = parse_pairwise(raw).unwrap();
        assert_eq!(r.subject_fidelity, "A");
        assert_eq!(r.composition, "B");
        assert_eq!(r.lighting_color, "tie");
        assert_eq!(r.production, "A");
        assert!(r.rationale.contains("identity"));
    }

    #[test]
    fn parse_strips_fenced_code_block() {
        let raw = "```json\n{\"subject_fidelity\":\"B\",\"composition\":\"B\",\"lighting_color\":\"B\",\"production\":\"B\",\"rationale\":\"B all-round.\"}\n```";
        let r = parse_pairwise(raw).unwrap();
        assert_eq!(r.subject_fidelity, "B");
        assert_eq!(r.production, "B");
    }

    #[test]
    fn parse_handles_prose_around_json() {
        let raw = "Sure, here is the verdict:\n{\"subject_fidelity\":\"tie\",\"composition\":\"A\",\"lighting_color\":\"A\",\"production\":\"tie\",\"rationale\":\"close call.\"}\nLet me know if you need more.";
        let r = parse_pairwise(raw).unwrap();
        assert_eq!(r.composition, "A");
        assert_eq!(r.subject_fidelity, "tie");
    }

    #[test]
    fn parse_normalizes_lowercase_verdicts() {
        let raw = r#"{"subject_fidelity":"a","composition":"b","lighting_color":"TIE","production":"A","rationale":""}"#;
        let r = parse_pairwise(raw).unwrap();
        assert_eq!(r.subject_fidelity, "A");
        assert_eq!(r.composition, "B");
        assert_eq!(r.lighting_color, "tie");
    }

    #[test]
    fn parse_rejects_unknown_verdict() {
        let raw = r#"{"subject_fidelity":"both","composition":"A","lighting_color":"A","production":"A","rationale":""}"#;
        assert!(parse_pairwise(raw).is_err());
    }

    #[test]
    fn parse_rejects_non_json() {
        assert!(parse_pairwise("the answer is A I think").is_err());
    }

    #[test]
    fn dry_run_emits_all_ties() {
        let client = FalClient::with_key("id:secret", fresh_cache());
        let adapter = FalPairwiseCompareAdapter::new(client);
        let req = PairwiseCompareRequest::new(
            "https://example.com/a.png",
            "https://example.com/b.png",
            "brief",
            "shot",
        );
        let out = adapter.compare(&req, RunMode::DryRun).unwrap();
        assert_eq!(out.mode, "dry-run");
        assert_eq!(out.response.subject_fidelity, "tie");
        assert_eq!(out.response.composition, "tie");
        assert_eq!(out.response.lighting_color, "tie");
        assert_eq!(out.response.production, "tie");
    }

    #[test]
    fn empty_url_rejected() {
        let client = FalClient::with_key("id:secret", fresh_cache());
        let adapter = FalPairwiseCompareAdapter::new(client);
        let req = PairwiseCompareRequest::new("", "https://example.com/b.png", "", "");
        assert!(matches!(
            adapter.compare(&req, RunMode::DryRun).unwrap_err(),
            BackendError::InvalidRequest(_)
        ));
    }

    #[test]
    fn cost_estimate_one_cent() {
        let client = FalClient::with_key("id:secret", fresh_cache());
        let adapter = FalPairwiseCompareAdapter::new(client);
        let req = PairwiseCompareRequest::new(
            "https://example.com/a.png",
            "https://example.com/b.png",
            "",
            "",
        );
        let est = adapter.estimate_cost(&req);
        assert!((est.cost_usd - PRICE_PER_CALL_USD).abs() < 1e-6);
    }
}
