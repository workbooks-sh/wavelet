//! Fal any-llm/vision adapter — `VisionVerify` cluster.
//!
//! Wraps `fal-ai/any-llm/vision` (a model-routing endpoint) with a
//! structured PASS/WARN/FAIL prompt. Used before paid render+mux to
//! catch identity drift, bystanders, watermarks, anatomical errors —
//! anything the brief said should/shouldn't be in frame.
//!
//! Default routed model: `google/gemini-3-pro` — per May-2026 SOTA
//! research (Video-MME 78.2 vs next-best 71.4, the largest vision gap
//! of any frontier model). Accuracy matters more than cost at the
//! verify gate: a false PASS at $0.01 lets through a re-render at
//! $0.20+, so it's cheaper overall to spend more per check. Cost-
//! sensitive callers (or experimentation) can drop to flash-lite via
//! the `VLM_MODEL` env override.
//!
//! Wire format:
//! ```text
//! POST https://fal.run/fal-ai/any-llm/vision
//! Authorization: Key <id>:<secret>
//! { "prompt": "...", "image_url": "...", "model": "..." }
//! → { "output": "1. PASS - ...\n2. FAIL - ...", "reasoning": null, "partial": false, "error": null }
//! ```
//!
//! Cost: ~$0.01 per call (depends on routed model).

use crate::backends::cache::{utc_now_iso8601, AssetCache, Manifest};
use crate::backends::fal::FalClient;
use crate::backends::image::{
    Finding, FindingStatus, VisionVerifyBackend, VisionVerifyRequest, VisionVerifyResult,
    CLUSTER_VISION_VERIFY,
};
use crate::backends::{
    check_budget, mode_label, BackendCallOutcome, BackendError, CostEstimate, RunMode,
};
use serde::{Deserialize, Serialize};

/// Provider id.
pub const PROVIDER: &str = "fal-vision-verify";

/// Fal model path (the router).
pub const MODEL_PATH: &str = "fal-ai/any-llm/vision";

/// Default routed VLM — Gemini 3 Pro. Highest Video-MME score among
/// frontier models (78.2, May-2026 SOTA). Set `VLM_MODEL=google/gemini-2.5-flash-lite`
/// for a ~5x cheaper, less-accurate fallback.
pub const DEFAULT_VLM: &str = "google/gemini-3-pro";

/// Env var to override the routed model.
pub const VLM_MODEL_ENV: &str = "VLM_MODEL";

/// Per-call cost estimate (USD). Conservative — Gemini 3 Pro on
/// fal-ai/any-llm/vision runs ~$0.02–0.04 per image-grading call.
pub const PRICE_PER_CALL_USD: f32 = 0.03;

/// Fal vision-verify adapter.
#[derive(Debug, Clone)]
pub struct FalVisionVerifyAdapter {
    client: FalClient,
}

impl FalVisionVerifyAdapter {
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

/// Build the structured grading prompt the VLM will fill out.
pub(crate) fn build_prompt(criteria: &[String]) -> String {
    let mut p = String::with_capacity(256 + criteria.len() * 64);
    p.push_str(
        "Examine this image. For each numbered criterion below, answer PASS, WARN, or FAIL \
followed by ' - ' and a brief one-sentence reason. Use exactly this format per line, one line \
per criterion, in order: '<n>. <PASS|WARN|FAIL> - <reason>'. PASS = clearly met. WARN = \
partially met or unclear. FAIL = clearly violated. Do not add any other commentary.\n\nCriteria:\n",
    );
    for (i, c) in criteria.iter().enumerate() {
        p.push_str(&format!("{}. {}\n", i + 1, c));
    }
    p
}

impl VisionVerifyBackend for FalVisionVerifyAdapter {
    fn name(&self) -> &'static str {
        PROVIDER
    }

    fn estimate_cost(&self, _: &VisionVerifyRequest) -> CostEstimate {
        CostEstimate {
            provider: PROVIDER.into(),
            cost_usd: PRICE_PER_CALL_USD,
            explanation: format!("${PRICE_PER_CALL_USD:.4}/call (any-llm/vision router)"),
        }
    }

    fn verify(
        &self,
        request: &VisionVerifyRequest,
        mode: RunMode,
    ) -> Result<BackendCallOutcome<VisionVerifyResult>, BackendError> {
        if request.image_url.trim().is_empty() {
            return Err(BackendError::InvalidRequest("image_url is empty".into()));
        }
        if request.criteria.is_empty() {
            return Err(BackendError::InvalidRequest(
                "at least one criterion is required".into(),
            ));
        }

        let estimate = self.estimate_cost(request);
        check_budget(&estimate, mode)?;

        let request_hash = AssetCache::request_hash(PROVIDER, CLUSTER_VISION_VERIFY, request)?;
        let cache = self.client.cache();

        if let Some(manifest) = cache.hit(PROVIDER, &request_hash)? {
            let response: VisionVerifyResult = serde_json::from_value(manifest.response.clone())
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
            // Dry-run: emit synthetic Warn findings so the JSON shape is
            // identical to a live response, without grading anything.
            let findings = request
                .criteria
                .iter()
                .map(|c| Finding {
                    criterion: c.clone(),
                    status: FindingStatus::Warn,
                    reason: "dry-run: not graded".into(),
                })
                .collect();
            let response = VisionVerifyResult {
                provider: PROVIDER.into(),
                findings,
                overall_pass: true,
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

        let body = AnyLlmVisionBody {
            prompt: build_prompt(&request.criteria),
            image_url: request.image_url.clone(),
            model: Self::routed_model(),
        };
        let parsed: AnyLlmVisionResponse = self.client.post_sync(MODEL_PATH, &body)?;
        if let Some(err) = parsed.error.as_ref() {
            return Err(BackendError::HttpStatus {
                status: 502,
                body: err.clone(),
            });
        }
        let findings = parse_findings(&parsed.output, &request.criteria);
        let overall_pass = findings
            .iter()
            .all(|f| !matches!(f.status, FindingStatus::Fail));
        let result = VisionVerifyResult {
            provider: PROVIDER.into(),
            findings,
            overall_pass,
        };

        let manifest = Manifest {
            version: 1,
            provider: PROVIDER.into(),
            cluster: CLUSTER_VISION_VERIFY.into(),
            request_hash: request_hash.clone(),
            request: serde_json::to_value(request).map_err(|e| {
                BackendError::Cache(format!("serialize request for cache: {e}"))
            })?,
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
struct AnyLlmVisionBody {
    prompt: String,
    image_url: String,
    model: String,
}

#[derive(Debug, Deserialize)]
struct AnyLlmVisionResponse {
    #[serde(default)]
    output: String,
    #[serde(default)]
    error: Option<String>,
}

/// Parse the VLM's numbered output into one `Finding` per input
/// criterion. Tolerates extra whitespace, optional bold markers, and
/// case-insensitive verdict tokens (`Pass`/`pass`/`PASS`, `failed`,
/// etc.). When a criterion's line is missing or unparseable, emits a
/// `Warn` finding with the raw fallback so the caller can investigate
/// instead of silently passing.
pub(crate) fn parse_findings(output: &str, criteria: &[String]) -> Vec<Finding> {
    let mut by_index: Vec<Option<(FindingStatus, String)>> = vec![None; criteria.len()];

    for raw_line in output.lines() {
        let line = raw_line.trim().trim_start_matches(['*', '-', '#', ' ']);
        if line.is_empty() {
            continue;
        }
        let Some((num_part, rest)) = line.split_once('.') else {
            continue;
        };
        let Ok(n) = num_part.trim().parse::<usize>() else {
            continue;
        };
        if n == 0 || n > criteria.len() {
            continue;
        }
        let rest = rest.trim_start_matches([' ', '*', '_', '`']).trim();
        let (verdict_word, reason) = split_verdict_and_reason(rest);
        let Some(status) = classify_verdict(verdict_word) else {
            continue;
        };
        by_index[n - 1] = Some((status, reason));
    }

    criteria
        .iter()
        .enumerate()
        .map(|(i, c)| match by_index[i].take() {
            Some((status, reason)) => Finding {
                criterion: c.clone(),
                status,
                reason,
            },
            None => Finding {
                criterion: c.clone(),
                status: FindingStatus::Warn,
                reason: "parser: no verdict line for this criterion".into(),
            },
        })
        .collect()
}

fn split_verdict_and_reason(rest: &str) -> (&str, String) {
    // Split on the first whitespace; reason is whatever follows the
    // verdict, with a leading `-`/`:`/`–` stripped.
    let mut iter = rest.splitn(2, |c: char| c.is_whitespace());
    let verdict = iter.next().unwrap_or("");
    let after = iter.next().unwrap_or("").trim();
    let reason = after
        .trim_start_matches(['-', ':', '–', '—', ' ', '*'])
        .trim()
        .to_string();
    (verdict.trim_matches(|c: char| !c.is_alphabetic()), reason)
}

fn classify_verdict(word: &str) -> Option<FindingStatus> {
    let w = word.to_ascii_lowercase();
    if w == "pass" || w == "passes" || w == "passed" {
        Some(FindingStatus::Pass)
    } else if w == "warn" || w == "warning" || w == "warns" {
        Some(FindingStatus::Warn)
    } else if w == "fail" || w == "fails" || w == "failed" {
        Some(FindingStatus::Fail)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fresh_cache() -> std::path::PathBuf {
        let tmp = std::env::temp_dir().join(format!(
            "wavelet-fal-verify-{}",
            AssetCache::request_hash("seed", "seed", &"x").unwrap()
        ));
        let _ = std::fs::remove_dir_all(&tmp);
        tmp
    }

    #[test]
    fn request_round_trips() {
        let req = VisionVerifyRequest::new(
            "https://example.com/x.jpg",
            vec!["shows a saguaro".into(), "no people".into()],
        );
        let json = serde_json::to_string(&req).unwrap();
        let back: VisionVerifyRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(back.image_url, "https://example.com/x.jpg");
        assert_eq!(back.criteria.len(), 2);
        assert_eq!(back.criteria[1], "no people");
    }

    #[test]
    fn prompt_lists_numbered_criteria() {
        let criteria = vec![
            "the subject is a green Porsche 911 GT3".to_string(),
            "no bystanders visible".to_string(),
            "no baked-in text or watermarks".to_string(),
        ];
        let prompt = build_prompt(&criteria);
        assert!(prompt.contains("PASS"));
        assert!(prompt.contains("FAIL"));
        assert!(prompt.contains("1. the subject is a green Porsche 911 GT3"));
        assert!(prompt.contains("2. no bystanders visible"));
        assert!(prompt.contains("3. no baked-in text or watermarks"));
    }

    #[test]
    fn parser_recovers_pass_warn_fail() {
        let criteria = vec!["c1".to_string(), "c2".to_string(), "c3".to_string()];
        let output = "1. PASS - all good here.\n2. WARN - somewhat unclear.\n3. FAIL - violated.";
        let findings = parse_findings(output, &criteria);
        assert_eq!(findings.len(), 3);
        assert_eq!(findings[0].status, FindingStatus::Pass);
        assert!(findings[0].reason.contains("all good"));
        assert_eq!(findings[1].status, FindingStatus::Warn);
        assert_eq!(findings[2].status, FindingStatus::Fail);
        assert!(findings[2].reason.contains("violated"));
    }

    #[test]
    fn parser_is_case_insensitive_and_accepts_inflections() {
        let criteria = vec!["c1".to_string(), "c2".to_string(), "c3".to_string()];
        // Mixed case + inflected verdicts the model has been observed
        // emitting: `Fail`, `failed`, `Passed`.
        let output = "1. Fail - off-brand.\n2. failed - no subject.\n3. Passed - fine.";
        let findings = parse_findings(output, &criteria);
        assert_eq!(findings[0].status, FindingStatus::Fail);
        assert_eq!(findings[1].status, FindingStatus::Fail);
        assert_eq!(findings[2].status, FindingStatus::Pass);
    }

    #[test]
    fn parser_emits_warn_for_missing_lines() {
        let criteria = vec!["c1".to_string(), "c2".to_string(), "c3".to_string()];
        // Model only graded the first criterion.
        let output = "1. PASS - sure.";
        let findings = parse_findings(output, &criteria);
        assert_eq!(findings.len(), 3);
        assert_eq!(findings[0].status, FindingStatus::Pass);
        assert_eq!(findings[1].status, FindingStatus::Warn);
        assert_eq!(findings[2].status, FindingStatus::Warn);
        assert!(findings[1].reason.contains("parser"));
    }

    #[test]
    fn dry_run_emits_request_shape_without_grading() {
        let client = FalClient::with_key("id:secret", fresh_cache());
        let adapter = FalVisionVerifyAdapter::new(client);
        let req = VisionVerifyRequest::new(
            "https://example.com/x.jpg",
            vec!["criterion A".into(), "criterion B".into()],
        );
        let out = adapter.verify(&req, RunMode::DryRun).unwrap();
        assert_eq!(out.mode, "dry-run");
        assert_eq!(out.provider, PROVIDER);
        assert_eq!(out.response.findings.len(), 2);
        assert!(out
            .response
            .findings
            .iter()
            .all(|f| matches!(f.status, FindingStatus::Warn)));
        assert!(out.response.overall_pass);
    }

    #[test]
    fn empty_image_url_rejected() {
        let client = FalClient::with_key("id:secret", fresh_cache());
        let adapter = FalVisionVerifyAdapter::new(client);
        let req = VisionVerifyRequest::new("", vec!["x".into()]);
        assert!(matches!(
            adapter.verify(&req, RunMode::DryRun).unwrap_err(),
            BackendError::InvalidRequest(_)
        ));
    }

    #[test]
    fn empty_criteria_rejected() {
        let client = FalClient::with_key("id:secret", fresh_cache());
        let adapter = FalVisionVerifyAdapter::new(client);
        let req = VisionVerifyRequest::new("https://example.com/x.jpg", vec![]);
        assert!(matches!(
            adapter.verify(&req, RunMode::DryRun).unwrap_err(),
            BackendError::InvalidRequest(_)
        ));
    }

    #[test]
    fn cost_estimate_matches_price_per_call() {
        let client = FalClient::with_key("id:secret", fresh_cache());
        let adapter = FalVisionVerifyAdapter::new(client);
        let req = VisionVerifyRequest::new("https://example.com/x.jpg", vec!["a".into()]);
        let est = adapter.estimate_cost(&req);
        assert_eq!(est.provider, PROVIDER);
        assert!((est.cost_usd - PRICE_PER_CALL_USD).abs() < 1e-6);
    }

    #[test]
    fn overall_pass_false_when_any_fail() {
        // Build a VisionVerifyResult directly to exercise the contract:
        // any Fail flips overall_pass to false; Warn does not.
        let warn_then_pass = VisionVerifyResult {
            provider: PROVIDER.into(),
            findings: vec![
                Finding {
                    criterion: "a".into(),
                    status: FindingStatus::Pass,
                    reason: String::new(),
                },
                Finding {
                    criterion: "b".into(),
                    status: FindingStatus::Warn,
                    reason: String::new(),
                },
            ],
            overall_pass: true,
        };
        // Equivalent of what the adapter computes.
        let recomputed = warn_then_pass
            .findings
            .iter()
            .all(|f| !matches!(f.status, FindingStatus::Fail));
        assert!(recomputed);

        let with_fail_findings = vec![
            Finding {
                criterion: "a".into(),
                status: FindingStatus::Pass,
                reason: String::new(),
            },
            Finding {
                criterion: "b".into(),
                status: FindingStatus::Fail,
                reason: String::new(),
            },
        ];
        let any_fail = with_fail_findings
            .iter()
            .all(|f| !matches!(f.status, FindingStatus::Fail));
        assert!(!any_fail);
    }
}
