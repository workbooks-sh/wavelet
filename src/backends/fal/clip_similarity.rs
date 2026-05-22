//! Fal CLIP-similarity adapter — `IdentitySimilarity` cluster.
//!
//! Intended target: a Fal-hosted CLIP-similarity endpoint (or a
//! CLIP/DINO feature-extraction endpoint, with cosine-sim computed
//! locally via `cosine_similarity`). Probed paths during construction:
//!
//! - `fal-ai/clip-similarity` — 404 (sync + queue)
//! - `fal-ai/imageutils/clip-similarity` — sync 404; queue submit
//!   returns 200 IN_QUEUE but the response route reports "Path
//!   /clip-similarity not found", so no real model is wired
//! - `fal-ai/dino-similarity`, `fal-ai/dino-v2`, `fal-ai/clip-features`,
//!   `fal-ai/clip-embeddings`, `fal-ai/imageutils/embeddings`, plus
//!   variants under `imageutils/{clip,dino,feature-extractor,…}` — all
//!   404 on sync + queue
//!
//! That leaves three viable shipping paths, none of which fit inside
//! this issue's scope:
//!
//! 1. Wait for Fal to publish a CLIP/DINO embedding endpoint — recheck
//!    the catalog periodically and flip this adapter to live once a
//!    real endpoint exists. Expected body shape (CLIP-similarity
//!    style): `{image_url, ref_image_url}` → `{similarity: f32}`.
//!    Embedding style: `{image_url}` → `{embedding: Vec<f32>}` for both
//!    images, then `cosine_similarity` locally.
//! 2. Port CLIP ViT-L/14 to `tract` or `ort` (rust-onnx) and embed in
//!    the binary — out of scope per the issue spec (would balloon the
//!    cargo build and add a model-download step). File as a follow-up
//!    if Fal stays barren.
//! 3. Use a different provider (Replicate has `andreasjansson/clip-features`,
//!    HuggingFace inference endpoints expose CLIP). Wire as a sibling
//!    adapter under a different namespace (`replicate::clip_features`).
//!
//! For now: the trait + CLI verb + cost model + caching scaffold all
//! ship green. The live `check` call returns `BackendError::Unimplemented`
//! so callers see a clear "wire a real backend" signal rather than a
//! silent-pass.
//!
//! Expected per-call cost when wired: ~$0.001 (embedding extraction is
//! ~10x cheaper than a generation call).

use crate::backends::cache::AssetCache;
use crate::backends::fal::FalClient;
use crate::backends::image::{
    IdentityCheckRequest, IdentityCheckResult, IdentitySimilarityBackend,
    CLUSTER_IDENTITY_CHECK,
};
use crate::backends::{
    check_budget, mode_label, BackendCallOutcome, BackendError, CostEstimate, RunMode,
};
use serde::{Deserialize, Serialize};

/// Provider id.
pub const PROVIDER: &str = "fal-clip-similarity";

/// Fal model path — the *intended* endpoint. Submit currently routes
/// to a 404'ing response handler; flip the adapter to live once Fal
/// exposes a real CLIP-similarity model under this path (or rewrite
/// `check` to call a sibling embedding endpoint + cosine-sim locally).
pub const MODEL_PATH: &str = "fal-ai/imageutils/clip-similarity";

/// Per-call cost estimate (USD). Embedding extraction is cheap.
pub const PRICE_PER_CALL_USD: f32 = 0.001;

/// Fal CLIP-similarity adapter.
#[derive(Debug, Clone)]
pub struct FalClipSimilarityAdapter {
    client: FalClient,
}

impl FalClipSimilarityAdapter {
    /// Build from a pre-constructed client.
    pub fn new(client: FalClient) -> Self {
        Self { client }
    }
}

impl IdentitySimilarityBackend for FalClipSimilarityAdapter {
    fn name(&self) -> &'static str {
        PROVIDER
    }

    fn estimate_cost(&self, _: &IdentityCheckRequest) -> CostEstimate {
        CostEstimate {
            provider: PROVIDER.into(),
            cost_usd: PRICE_PER_CALL_USD,
            explanation: format!("${PRICE_PER_CALL_USD:.4}/call (CLIP-similarity, conservative)"),
        }
    }

    fn check(
        &self,
        request: &IdentityCheckRequest,
        threshold: f32,
        mode: RunMode,
    ) -> Result<BackendCallOutcome<IdentityCheckResult>, BackendError> {
        if request.reference_url.trim().is_empty() {
            return Err(BackendError::InvalidRequest("reference_url is empty".into()));
        }
        if request.candidate_url.trim().is_empty() {
            return Err(BackendError::InvalidRequest("candidate_url is empty".into()));
        }
        if !(0.0..=1.0).contains(&threshold) {
            return Err(BackendError::InvalidRequest(format!(
                "threshold {threshold} outside [0.0, 1.0]"
            )));
        }

        let estimate = self.estimate_cost(request);
        check_budget(&estimate, mode)?;

        let cache_key = (request, threshold_bucket(threshold));
        let request_hash =
            AssetCache::request_hash(PROVIDER, CLUSTER_IDENTITY_CHECK, &cache_key)?;
        let cache = self.client.cache();

        if let Some(manifest) = cache.hit(PROVIDER, &request_hash)? {
            let response: IdentityCheckResult =
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
            let response = IdentityCheckResult {
                provider: PROVIDER.into(),
                similarity: 0.0,
                passes_threshold: false,
                threshold,
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

        Err(BackendError::Unimplemented(
            "fal-clip-similarity: no live Fal CLIP/DINO endpoint at probe time \
             (see clip_similarity.rs header for the recheck list). \
             Run with --dry-run, or wire a sibling adapter once Fal publishes \
             an embedding model.",
        ))
    }
}

/// Body shape we'll send the day Fal exposes a real similarity endpoint
/// under `MODEL_PATH`. Kept in code so the live wiring is a single
/// `post_sync` call away.
#[derive(Debug, Serialize)]
#[allow(dead_code)]
struct ClipSimilarityBody {
    image_url: String,
    ref_image_url: String,
}

/// Response shape we expect from a CLIP-similarity endpoint. Kept here
/// so the response decoder is already tested.
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct ClipSimilarityResponse {
    similarity: f32,
}

/// Decide whether `similarity` clears `threshold` (with a tiny epsilon
/// for the equality case so 0.85 vs 0.85 reads as pass). Used by the
/// live-mode response decoder when the endpoint lands; meanwhile the
/// dry-run path skips it. Exposed so tests can pin the math.
#[allow(dead_code)]
pub(crate) fn passes(similarity: f32, threshold: f32) -> bool {
    similarity + 1e-6 >= threshold
}

/// Quantize the threshold into a coarse bucket for cache-key stability —
/// two `0.8501` vs `0.8502` requests share a cache slot but `0.85` vs
/// `0.90` do not (different policy = different decision row).
fn threshold_bucket(threshold: f32) -> u32 {
    (threshold.clamp(0.0, 1.0) * 1000.0).round() as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fresh_cache() -> std::path::PathBuf {
        let tmp = std::env::temp_dir().join(format!(
            "wavelet-fal-clip-sim-{}",
            AssetCache::request_hash("seed", "seed", &"x").unwrap()
        ));
        let _ = std::fs::remove_dir_all(&tmp);
        tmp
    }

    fn mk() -> FalClipSimilarityAdapter {
        let client = FalClient::with_key("id:secret", fresh_cache());
        FalClipSimilarityAdapter::new(client)
    }

    #[test]
    fn request_body_serializes_to_expected_shape() {
        let body = ClipSimilarityBody {
            image_url: "https://x/cand.jpg".into(),
            ref_image_url: "https://x/ref.jpg".into(),
        };
        let json = serde_json::to_value(&body).unwrap();
        assert_eq!(json["image_url"], "https://x/cand.jpg");
        assert_eq!(json["ref_image_url"], "https://x/ref.jpg");
    }

    #[test]
    fn response_decodes_minimal_payload() {
        let body = r#"{ "similarity": 0.873 }"#;
        let parsed: ClipSimilarityResponse = serde_json::from_str(body).unwrap();
        assert!((parsed.similarity - 0.873).abs() < 1e-5);
    }

    #[test]
    fn empty_urls_are_rejected() {
        let adapter = mk();
        let bad_ref = IdentityCheckRequest::new("", "https://x/c.jpg");
        let bad_cand = IdentityCheckRequest::new("https://x/r.jpg", "");
        assert!(matches!(
            adapter.check(&bad_ref, 0.85, RunMode::DryRun).unwrap_err(),
            BackendError::InvalidRequest(_)
        ));
        assert!(matches!(
            adapter.check(&bad_cand, 0.85, RunMode::DryRun).unwrap_err(),
            BackendError::InvalidRequest(_)
        ));
    }

    #[test]
    fn threshold_outside_unit_range_is_rejected() {
        let adapter = mk();
        let req = IdentityCheckRequest::new("https://x/r.jpg", "https://x/c.jpg");
        let err = adapter.check(&req, 1.5, RunMode::DryRun).unwrap_err();
        match err {
            BackendError::InvalidRequest(msg) => assert!(msg.contains("threshold")),
            other => panic!("expected InvalidRequest, got {other:?}"),
        }
    }

    #[test]
    fn dry_run_does_not_hit_network_and_does_not_write_asset() {
        let adapter = mk();
        let req = IdentityCheckRequest::new(
            "https://example.com/ref.jpg",
            "https://example.com/cand.jpg",
        );
        let out = adapter.check(&req, 0.85, RunMode::DryRun).unwrap();
        assert_eq!(out.mode, "dry-run");
        assert_eq!(out.response.provider, PROVIDER);
        assert!(!out.response.passes_threshold);
        assert!((out.response.threshold - 0.85).abs() < 1e-6);
    }

    #[test]
    fn cost_estimate_is_cheap_and_carries_provider() {
        let adapter = mk();
        let req =
            IdentityCheckRequest::new("https://x/r.jpg", "https://x/c.jpg");
        let est = adapter.estimate_cost(&req);
        assert_eq!(est.provider, PROVIDER);
        assert!((est.cost_usd - PRICE_PER_CALL_USD).abs() < 1e-6);
        assert!(est.cost_usd < 0.01);
    }

    #[test]
    fn passes_threshold_math_is_inclusive() {
        assert!(passes(0.85, 0.85));
        assert!(passes(0.90, 0.85));
        assert!(!passes(0.84, 0.85));
        assert!(!passes(0.0, 0.85));
        assert!(passes(1.0, 0.85));
    }

    #[test]
    fn live_mode_returns_unimplemented_until_endpoint_lands() {
        let adapter = mk();
        let req = IdentityCheckRequest::new(
            "https://example.com/ref.jpg",
            "https://example.com/cand.jpg",
        );
        let err = adapter
            .check(&req, 0.85, RunMode::Live { max_cost_usd: 1.0 })
            .unwrap_err();
        match err {
            BackendError::Unimplemented(msg) => {
                assert!(msg.contains("fal-clip-similarity"));
            }
            other => panic!("expected Unimplemented, got {other:?}"),
        }
    }

    #[test]
    fn over_budget_blocks_before_unimplemented() {
        let adapter = mk();
        let req = IdentityCheckRequest::new(
            "https://example.com/ref.jpg",
            "https://example.com/cand.jpg",
        );
        let err = adapter
            .check(&req, 0.85, RunMode::Live { max_cost_usd: 0.0 })
            .unwrap_err();
        match err {
            BackendError::OverBudget { .. } => {}
            other => panic!("expected OverBudget, got {other:?}"),
        }
    }
}
