//! Roboflow doctr OCR adapter — `Ocr` cluster.
//!
//! Wraps Roboflow's hosted `doctr/ocr` endpoint. Used by the typography
//! flow to detect baked-in text (signage, license plates, watermarks)
//! before placing HTML overlays.
//!
//! Wire format:
//! ```text
//! POST https://infer.roboflow.com/doctr/ocr?api_key=<KEY>
//! Content-Type: application/json
//! { "image": { "type": "url", "value": "<https-or-data-url>" } }
//! → { "result": "<full extracted text as one string>", "time": 0.42 }
//! ```
//!
//! The `result` is a single flat string. We split on newlines into
//! pseudo-detections (one per non-empty line). doctr does not return
//! bboxes or per-block confidences in this response shape, so both
//! fields are `None` on every detection.
//!
//! Cost: ~$0.001 per call (Roboflow per-inference pricing).

use crate::backends::cache::{utc_now_iso8601, AssetCache, Manifest};
use crate::backends::fal::RoboflowClient;
use crate::backends::image::{
    OcrBackend, OcrDetection, OcrRequest, OcrResult, CLUSTER_OCR,
};
use crate::backends::{
    check_budget, mode_label, BackendCallOutcome, BackendError, CostEstimate, RunMode,
};
use serde::{Deserialize, Serialize};

/// Provider id.
pub const PROVIDER: &str = "roboflow-doctr";

/// Roboflow model path.
pub const MODEL_PATH: &str = "doctr/ocr";

/// Per-call cost estimate (USD). Roboflow per-inference pricing.
pub const PRICE_PER_CALL_USD: f32 = 0.001;

/// Roboflow doctr OCR adapter.
#[derive(Debug, Clone)]
pub struct RoboflowDoctrOcrAdapter {
    client: RoboflowClient,
}

impl RoboflowDoctrOcrAdapter {
    /// Build from a pre-constructed client.
    pub fn new(client: RoboflowClient) -> Self {
        Self { client }
    }
}

impl OcrBackend for RoboflowDoctrOcrAdapter {
    fn name(&self) -> &'static str {
        PROVIDER
    }

    fn estimate_cost(&self, _: &OcrRequest) -> CostEstimate {
        CostEstimate {
            provider: PROVIDER.into(),
            cost_usd: PRICE_PER_CALL_USD,
            explanation: format!("${PRICE_PER_CALL_USD:.4}/call (Roboflow doctr/ocr)"),
        }
    }

    fn recognize(
        &self,
        request: &OcrRequest,
        mode: RunMode,
    ) -> Result<BackendCallOutcome<OcrResult>, BackendError> {
        if request.image_url.trim().is_empty() {
            return Err(BackendError::InvalidRequest("image_url is empty".into()));
        }

        let estimate = self.estimate_cost(request);
        check_budget(&estimate, mode)?;

        let request_hash = AssetCache::request_hash(PROVIDER, CLUSTER_OCR, request)?;
        let cache = self.client.cache();

        if let Some(manifest) = cache.hit(PROVIDER, &request_hash)? {
            let response: OcrResult = serde_json::from_value(manifest.response.clone())
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
            let response = OcrResult {
                provider: PROVIDER.into(),
                detections: vec![],
                combined_text: String::new(),
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

        let body = DoctrBody {
            image: DoctrImage {
                ty: "url".into(),
                value: request.image_url.clone(),
            },
        };
        let parsed: DoctrResponse = self.client.post_sync(MODEL_PATH, &body)?;
        let result = build_result(&parsed.result);

        let manifest = Manifest {
            version: 1,
            provider: PROVIDER.into(),
            cluster: CLUSTER_OCR.into(),
            request_hash: request_hash.clone(),
            request: serde_json::to_value(request)
                .map_err(|e| BackendError::Cache(format!("serialize request for cache: {e}")))?,
            response: serde_json::to_value(&result)
                .map_err(|e| BackendError::Cache(format!("serialize response for cache: {e}")))?,
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
struct DoctrBody {
    image: DoctrImage,
}

#[derive(Debug, Serialize)]
struct DoctrImage {
    #[serde(rename = "type")]
    ty: String,
    value: String,
}

#[derive(Debug, Deserialize)]
struct DoctrResponse {
    #[serde(default)]
    result: String,
}

/// Build the cluster-shaped result from doctr's flat string. One
/// detection per non-empty line, with no bbox or confidence (doctr's
/// simple response shape does not include either). `combined_text`
/// echoes the original string, trimmed.
pub(crate) fn build_result(raw: &str) -> OcrResult {
    let trimmed = raw.trim();
    let detections: Vec<OcrDetection> = trimmed
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(|line| OcrDetection {
            text: line.to_string(),
            bbox: None,
            confidence: None,
        })
        .collect();
    let combined_text = detections
        .iter()
        .map(|d| d.text.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    OcrResult {
        provider: PROVIDER.into(),
        detections,
        combined_text,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fresh_cache() -> std::path::PathBuf {
        let tmp = std::env::temp_dir().join(format!(
            "wavelet-rf-doctr-{}",
            AssetCache::request_hash("seed", "seed", &"x").unwrap()
        ));
        let _ = std::fs::remove_dir_all(&tmp);
        tmp
    }

    fn mk() -> RoboflowDoctrOcrAdapter {
        let client = RoboflowClient::with_key("rf-test-key", fresh_cache());
        RoboflowDoctrOcrAdapter::new(client)
    }

    #[test]
    fn request_body_serializes_to_expected_shape() {
        let body = DoctrBody {
            image: DoctrImage {
                ty: "url".into(),
                value: "https://example.com/x.jpg".into(),
            },
        };
        let json = serde_json::to_value(&body).unwrap();
        assert_eq!(json["image"]["type"], "url");
        assert_eq!(json["image"]["value"], "https://example.com/x.jpg");
    }

    #[test]
    fn response_decodes_minimal_payload() {
        let body = r#"{"result":"the text","time":0.4}"#;
        let parsed: DoctrResponse = serde_json::from_str(body).unwrap();
        assert_eq!(parsed.result, "the text");
    }

    #[test]
    fn build_result_single_line_becomes_one_detection() {
        let r = build_result("the text");
        assert_eq!(r.provider, PROVIDER);
        assert_eq!(r.detections.len(), 1);
        assert_eq!(r.detections[0].text, "the text");
        assert!(r.detections[0].bbox.is_none());
        assert!(r.detections[0].confidence.is_none());
        assert_eq!(r.combined_text, "the text");
    }

    #[test]
    fn build_result_empty_string_yields_no_detections() {
        let r = build_result("");
        assert!(r.detections.is_empty());
        assert_eq!(r.combined_text, "");
        let r2 = build_result("   \n\n  \n");
        assert!(r2.detections.is_empty());
    }

    #[test]
    fn build_result_multi_line_splits_per_line() {
        let r = build_result("STOP\nONE WAY\n\n  DETOUR  ");
        assert_eq!(r.detections.len(), 3);
        assert_eq!(r.detections[0].text, "STOP");
        assert_eq!(r.detections[1].text, "ONE WAY");
        assert_eq!(r.detections[2].text, "DETOUR");
        assert_eq!(r.combined_text, "STOP\nONE WAY\nDETOUR");
    }

    #[test]
    fn empty_image_url_is_rejected() {
        let adapter = mk();
        let req = OcrRequest::new("");
        assert!(matches!(
            adapter.recognize(&req, RunMode::DryRun).unwrap_err(),
            BackendError::InvalidRequest(_)
        ));
    }

    #[test]
    fn dry_run_does_not_hit_network() {
        let adapter = mk();
        let req = OcrRequest::new("https://example.com/x.jpg");
        let out = adapter.recognize(&req, RunMode::DryRun).unwrap();
        assert_eq!(out.mode, "dry-run");
        assert_eq!(out.response.provider, PROVIDER);
        assert!(out.response.detections.is_empty());
        assert_eq!(out.response.combined_text, "");
    }

    #[test]
    fn cost_estimate_is_cheap_and_carries_provider() {
        let adapter = mk();
        let req = OcrRequest::new("https://example.com/x.jpg");
        let est = adapter.estimate_cost(&req);
        assert_eq!(est.provider, PROVIDER);
        assert!((est.cost_usd - PRICE_PER_CALL_USD).abs() < 1e-6);
        assert!(est.cost_usd < 0.01);
    }

    #[test]
    fn over_budget_blocks_before_network() {
        let adapter = mk();
        let req = OcrRequest::new("https://example.com/x.jpg");
        let err = adapter
            .recognize(&req, RunMode::Live { max_cost_usd: 0.0 })
            .unwrap_err();
        match err {
            BackendError::OverBudget { .. } => {}
            other => panic!("expected OverBudget, got {other:?}"),
        }
    }

    #[test]
    fn ocr_request_round_trips() {
        let req = OcrRequest::new("https://example.com/x.jpg");
        let json = serde_json::to_string(&req).unwrap();
        let back: OcrRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(back.image_url, "https://example.com/x.jpg");
    }
}
