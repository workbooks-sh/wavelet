//! Roboflow CLIP-embedding adapter — `IdentitySimilarity` cluster.
//!
//! Wire shape (probed live):
//!
//! ```text
//! POST https://infer.roboflow.com/clip/embed_image?api_key=<KEY>
//! { "image": { "type": "url", "value": "<https-url>" } }
//! → { "embeddings": [[ -0.219, -0.648, ... ]] }   // 768-d, ViT-L/14
//! ```
//!
//! Roboflow's `embeddings` is a list-of-lists — for a single-image
//! request it's `[[vector]]`. We pull `embeddings[0]` as the vector.
//!
//! The adapter does two POSTs per `check` (reference + candidate),
//! then computes cosine similarity locally via
//! [`crate::backends::image::cosine_similarity`]. Compared with a
//! provider-side similarity endpoint this costs the same in calls but
//! gives us the raw vectors — useful if we later want to retain
//! reference embeddings and skip the re-embed on repeated checks.
//!
//! Cost: $0.0005/call × 2 calls = ~$0.001 per check.

use crate::backends::cache::{utc_now_iso8601, AssetCache, Manifest};
use crate::backends::fal::RoboflowClient;
use crate::backends::image::{
    cosine_similarity, IdentityCheckRequest, IdentityCheckResult, IdentitySimilarityBackend,
    CLUSTER_IDENTITY_CHECK,
};
use crate::backends::{
    check_budget, mode_label, BackendCallOutcome, BackendError, CostEstimate, RunMode,
};
use serde::{Deserialize, Serialize};

/// Provider id.
pub const PROVIDER: &str = "roboflow-clip";

/// Roboflow path — no model id, just the bare CLIP-embed verb.
pub const EMBED_PATH: &str = "clip/embed_image";

/// Per-embed-call cost (USD). Two calls per `check`.
pub const PRICE_PER_EMBED_USD: f32 = 0.0005;

/// Per-check cost (USD) — two embed calls plus rounding headroom.
pub const PRICE_PER_CALL_USD: f32 = PRICE_PER_EMBED_USD * 2.0;

/// Roboflow CLIP adapter.
#[derive(Debug, Clone)]
pub struct RoboflowClipAdapter {
    client: RoboflowClient,
}

impl RoboflowClipAdapter {
    /// Build from a pre-constructed client.
    pub fn new(client: RoboflowClient) -> Self {
        Self { client }
    }

    /// Embed one image URL. Returns the 768-d ViT-L/14 vector.
    fn embed(&self, image_url: &str) -> Result<Vec<f32>, BackendError> {
        let body = ClipEmbedBody::from_url(image_url);
        let parsed: ClipEmbedResponse = self.client.post_sync(EMBED_PATH, &body)?;
        parsed.into_vector()
    }
}

impl IdentitySimilarityBackend for RoboflowClipAdapter {
    fn name(&self) -> &'static str {
        PROVIDER
    }

    fn estimate_cost(&self, _: &IdentityCheckRequest) -> CostEstimate {
        CostEstimate {
            provider: PROVIDER.into(),
            cost_usd: PRICE_PER_CALL_USD,
            explanation: format!(
                "${PRICE_PER_EMBED_USD:.4}/embed × 2 = ${PRICE_PER_CALL_USD:.4}/check (CLIP ViT-L/14)"
            ),
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
        reject_data_url(&request.reference_url, "reference_url")?;
        reject_data_url(&request.candidate_url, "candidate_url")?;

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

        let ref_vec = self.embed(&request.reference_url)?;
        let cand_vec = self.embed(&request.candidate_url)?;
        if ref_vec.len() != cand_vec.len() {
            return Err(BackendError::Decode(format!(
                "embedding dims differ: reference={}, candidate={}",
                ref_vec.len(),
                cand_vec.len()
            )));
        }
        let similarity = cosine_similarity(&ref_vec, &cand_vec);
        let result = IdentityCheckResult {
            provider: PROVIDER.into(),
            similarity,
            passes_threshold: passes(similarity, threshold),
            threshold,
        };

        let manifest = Manifest {
            version: 1,
            provider: PROVIDER.into(),
            cluster: CLUSTER_IDENTITY_CHECK.into(),
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

/// Wire body Roboflow's `clip/embed_image` accepts. The `image` field is
/// itself a typed object — `type: "url"` for an HTTPS URL.
#[derive(Debug, Serialize)]
struct ClipEmbedBody {
    image: ImageRef,
}

#[derive(Debug, Serialize)]
struct ImageRef {
    #[serde(rename = "type")]
    kind: &'static str,
    value: String,
}

impl ClipEmbedBody {
    fn from_url(url: &str) -> Self {
        Self {
            image: ImageRef {
                kind: "url",
                value: url.to_string(),
            },
        }
    }
}

/// Wire response shape — `embeddings` is `Vec<Vec<f32>>` (list of
/// vectors). For a single-image request, the outer list has length 1.
#[derive(Debug, Deserialize)]
struct ClipEmbedResponse {
    embeddings: Vec<Vec<f32>>,
}

impl ClipEmbedResponse {
    fn into_vector(self) -> Result<Vec<f32>, BackendError> {
        let mut iter = self.embeddings.into_iter();
        match iter.next() {
            Some(v) if !v.is_empty() => Ok(v),
            Some(_) => Err(BackendError::Decode(
                "roboflow clip/embed_image: inner embedding vector empty".into(),
            )),
            None => Err(BackendError::Decode(
                "roboflow clip/embed_image: embeddings array empty".into(),
            )),
        }
    }
}

/// Reject `data:` URLs — Roboflow's `type:"url"` path needs an HTTPS
/// fetch; data URLs require switching to `type:"base64"` which isn't
/// wired here yet. Flag as a follow-up (the F1 local-path UX produces
/// `data:` URLs).
fn reject_data_url(url: &str, label: &str) -> Result<(), BackendError> {
    if url.starts_with("data:") {
        return Err(BackendError::InvalidRequest(format!(
            "{label} is a data: URL; roboflow-clip currently only accepts HTTPS URLs \
             (base64 path is a follow-up — pass an https:// URL or use the fal backend)"
        )));
    }
    Ok(())
}

/// Decide whether `similarity` clears `threshold` (small epsilon so
/// `0.85` vs `0.85` reads as pass).
pub(crate) fn passes(similarity: f32, threshold: f32) -> bool {
    similarity + 1e-6 >= threshold
}

/// Quantize threshold for cache-key stability — `0.8501` ≡ `0.8502` but
/// `0.85` ≢ `0.90`.
fn threshold_bucket(threshold: f32) -> u32 {
    (threshold.clamp(0.0, 1.0) * 1000.0).round() as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fresh_cache() -> std::path::PathBuf {
        let tmp = std::env::temp_dir().join(format!(
            "wavelet-roboflow-clip-{}",
            AssetCache::request_hash("seed", "seed", &"x").unwrap()
        ));
        let _ = std::fs::remove_dir_all(&tmp);
        tmp
    }

    fn mk() -> RoboflowClipAdapter {
        let client = RoboflowClient::with_key("rf-test-key", fresh_cache());
        RoboflowClipAdapter::new(client)
    }

    #[test]
    fn request_body_serializes_to_expected_shape() {
        let body = ClipEmbedBody::from_url("https://example.com/ref.jpg");
        let json = serde_json::to_value(&body).unwrap();
        assert_eq!(json["image"]["type"], "url");
        assert_eq!(json["image"]["value"], "https://example.com/ref.jpg");
    }

    #[test]
    fn response_decoder_pulls_first_inner_vector() {
        let body = r#"{"inference_id":null,"frame_id":null,"time":0.2,
            "embeddings":[[ -0.219, -0.648, 0.5 ]]}"#;
        let parsed: ClipEmbedResponse = serde_json::from_str(body).unwrap();
        let v = parsed.into_vector().unwrap();
        assert_eq!(v.len(), 3);
        assert!((v[0] - -0.219).abs() < 1e-5);
        assert!((v[2] - 0.5).abs() < 1e-5);
    }

    #[test]
    fn empty_embeddings_array_errors_decode() {
        let body = r#"{"embeddings":[]}"#;
        let parsed: ClipEmbedResponse = serde_json::from_str(body).unwrap();
        match parsed.into_vector().unwrap_err() {
            BackendError::Decode(msg) => assert!(msg.contains("empty")),
            other => panic!("expected Decode, got {other:?}"),
        }
    }

    #[test]
    fn empty_inner_vector_errors_decode() {
        let body = r#"{"embeddings":[[]]}"#;
        let parsed: ClipEmbedResponse = serde_json::from_str(body).unwrap();
        assert!(matches!(parsed.into_vector(), Err(BackendError::Decode(_))));
    }

    #[test]
    fn cosine_identical_is_one() {
        let v = vec![0.1f32, -0.4, 0.7, 0.2, -0.05];
        assert!((cosine_similarity(&v, &v) - 1.0).abs() < 1e-5);
    }

    #[test]
    fn cosine_negated_is_minus_one() {
        let a = vec![0.3f32, -0.5, 0.8];
        let b: Vec<f32> = a.iter().map(|x| -x).collect();
        assert!((cosine_similarity(&a, &b) - -1.0).abs() < 1e-5);
    }

    #[test]
    fn cosine_against_hand_computed() {
        let a = vec![1.0f32, 2.0, 3.0];
        let b = vec![4.0f32, 5.0, 6.0];
        // dot = 4+10+18 = 32; |a|=sqrt(14), |b|=sqrt(77)
        let expected = 32.0_f32 / (14.0_f32.sqrt() * 77.0_f32.sqrt());
        let got = cosine_similarity(&a, &b);
        assert!((got - expected).abs() < 1e-5, "got {got} expected {expected}");
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
    fn data_urls_are_rejected_with_helpful_message() {
        let adapter = mk();
        let req = IdentityCheckRequest::new("data:image/png;base64,iVBOR", "https://x/c.jpg");
        let err = adapter.check(&req, 0.85, RunMode::DryRun).unwrap_err();
        match err {
            BackendError::InvalidRequest(msg) => {
                assert!(msg.contains("data:"));
                assert!(msg.contains("HTTPS"));
            }
            other => panic!("expected InvalidRequest, got {other:?}"),
        }
    }

    #[test]
    fn threshold_outside_unit_range_is_rejected() {
        let adapter = mk();
        let req = IdentityCheckRequest::new("https://x/r.jpg", "https://x/c.jpg");
        assert!(matches!(
            adapter.check(&req, 1.5, RunMode::DryRun).unwrap_err(),
            BackendError::InvalidRequest(_)
        ));
        assert!(matches!(
            adapter.check(&req, -0.01, RunMode::DryRun).unwrap_err(),
            BackendError::InvalidRequest(_)
        ));
    }

    #[test]
    fn dry_run_does_not_hit_network() {
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
        let req = IdentityCheckRequest::new("https://x/r.jpg", "https://x/c.jpg");
        let est = adapter.estimate_cost(&req);
        assert_eq!(est.provider, PROVIDER);
        assert!((est.cost_usd - PRICE_PER_CALL_USD).abs() < 1e-6);
        assert!(est.cost_usd < 0.01);
        assert!(est.explanation.contains("ViT-L/14"));
    }

    #[test]
    fn over_budget_blocks_before_network() {
        let adapter = mk();
        let req = IdentityCheckRequest::new("https://x/r.jpg", "https://x/c.jpg");
        let err = adapter
            .check(&req, 0.85, RunMode::Live { max_cost_usd: 0.0 })
            .unwrap_err();
        assert!(matches!(err, BackendError::OverBudget { .. }));
    }

    #[test]
    fn passes_threshold_math() {
        assert!(passes(0.85, 0.85));
        assert!(passes(0.99, 0.85));
        assert!(!passes(0.7, 0.85));
        assert!(passes(1.0, 0.85));
        assert!(!passes(0.0, 0.85));
    }

    #[test]
    fn threshold_bucket_is_stable() {
        assert_eq!(threshold_bucket(0.85), 850);
        assert_eq!(threshold_bucket(0.8501), 850);
        assert_ne!(threshold_bucket(0.85), threshold_bucket(0.90));
        assert_eq!(threshold_bucket(1.5), 1000);
        assert_eq!(threshold_bucket(-1.0), 0);
    }

    #[test]
    fn name_is_provider_constant() {
        let adapter = mk();
        assert_eq!(adapter.name(), "roboflow-clip");
        assert_eq!(adapter.name(), PROVIDER);
    }
}
