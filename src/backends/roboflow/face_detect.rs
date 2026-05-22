//! Roboflow face-detection adapter — `FaceDetect` cluster.
//!
//! Wraps Roboflow's hosted `face-detection-mik1i/18` model (the public
//! yolov8-shaped face detector). Used by the face-crop refine
//! paste-back pipeline (HelloRob template) to find the bbox the refine
//! pass and the alpha-blended paste-back run on.
//!
//! Wire format (probed live):
//!
//! ```text
//! GET https://detect.roboflow.com/face-detection-mik1i/18
//!         ?api_key=<KEY>&image=<https-url>
//! → {
//!     "inference_id": "...",
//!     "image": { "width": 400, "height": 352 },
//!     "predictions": [
//!       { "x": 200.5, "y": 114.0, "width": 141.0, "height": 212.0,
//!         "confidence": 0.89, "class": "face", "class_id": 0,
//!         "detection_id": "..." }
//!     ]
//!   }
//! ```
//!
//! Roboflow's `(x, y)` is the **center** of the box — we convert to
//! top-left + clamp into image bounds before returning.
//!
//! Note: this endpoint uses a different host than the rest of our
//! Roboflow integrations (`detect.roboflow.com` vs `infer.roboflow.com`)
//! and is GET-shaped rather than JSON POST. The shared `RoboflowClient`
//! caches under a different base URL, so this adapter holds the API key
//! directly and shares only the `AssetCache` via the client.
//!
//! Cost: ~$0.001 per call (Roboflow hosted-inference pricing).

use crate::backends::cache::{utc_now_iso8601, AssetCache, Manifest};
use crate::backends::fal::RoboflowClient;
use crate::backends::http_client::ROBOFLOW_KEY_ENV;
use crate::backends::image::{
    FaceDetectBackend, FaceDetectRequest, FaceDetectResult, FaceDetection, CLUSTER_FACE_DETECT,
};
use crate::backends::{
    check_budget, mode_label, BackendCallOutcome, BackendError, CostEstimate, RunMode,
};
use serde::Deserialize;

/// Provider id.
pub const PROVIDER: &str = "roboflow-face-detection-mik1i";

/// Roboflow model path.
pub const MODEL_PATH: &str = "face-detection-mik1i/18";

/// Detect host — different from `infer.roboflow.com` used by clip + doctr.
pub const DETECT_BASE: &str = "https://detect.roboflow.com";

/// Per-call cost estimate (USD). Roboflow per-inference pricing.
pub const PRICE_PER_CALL_USD: f32 = 0.001;

/// Roboflow face-detection adapter.
#[derive(Debug, Clone)]
pub struct RoboflowFaceDetectAdapter {
    api_key: String,
    cache: AssetCache,
}

impl RoboflowFaceDetectAdapter {
    /// Build from an explicit key and a cache. The cache is typically
    /// the one shared by the rest of the Roboflow adapters in the
    /// session, which is why we accept a `RoboflowClient`.
    pub fn from_client(api_key: impl Into<String>, client: &RoboflowClient) -> Self {
        Self {
            api_key: api_key.into(),
            cache: client.cache().clone(),
        }
    }

    /// Convenience constructor for tests and standalone use.
    pub fn with_key(api_key: impl Into<String>, cache: AssetCache) -> Self {
        Self {
            api_key: api_key.into(),
            cache,
        }
    }

    /// Build from `ROBOFLOW_API_KEY`.
    pub fn from_env(cache: AssetCache) -> Result<Self, BackendError> {
        let raw = std::env::var(ROBOFLOW_KEY_ENV)
            .map_err(|_| BackendError::MissingCredential(ROBOFLOW_KEY_ENV.into()))?;
        if raw.trim().is_empty() {
            return Err(BackendError::MissingCredential(ROBOFLOW_KEY_ENV.into()));
        }
        Ok(Self::with_key(raw, cache))
    }

    /// Compose the GET URL with the image-URL query param. `url::Url`
    /// would be cleaner but we already pay the URL-builder tax via
    /// percent-encoding helpers — keep the path light here.
    fn build_url(&self, image_url: &str) -> String {
        format!(
            "{DETECT_BASE}/{MODEL_PATH}?api_key={key}&image={image}",
            key = percent_encode(&self.api_key),
            image = percent_encode(image_url),
        )
    }
}

impl FaceDetectBackend for RoboflowFaceDetectAdapter {
    fn name(&self) -> &'static str {
        PROVIDER
    }

    fn estimate_cost(&self, _: &FaceDetectRequest) -> CostEstimate {
        CostEstimate {
            provider: PROVIDER.into(),
            cost_usd: PRICE_PER_CALL_USD,
            explanation: format!(
                "${PRICE_PER_CALL_USD:.4}/call (Roboflow face-detection-mik1i/18)"
            ),
        }
    }

    fn detect_faces(
        &self,
        request: &FaceDetectRequest,
        mode: RunMode,
    ) -> Result<BackendCallOutcome<FaceDetectResult>, BackendError> {
        if request.image_url.trim().is_empty() {
            return Err(BackendError::InvalidRequest("image_url is empty".into()));
        }
        if !(0.0..=1.0).contains(&request.min_confidence) {
            return Err(BackendError::InvalidRequest(format!(
                "min_confidence {} outside [0.0, 1.0]",
                request.min_confidence
            )));
        }
        if request.image_url.starts_with("data:") {
            return Err(BackendError::InvalidRequest(
                "roboflow face-detect currently only accepts HTTPS URLs (data: URLs would need a \
                 base64 POST path — not wired)"
                    .into(),
            ));
        }

        let estimate = self.estimate_cost(request);
        check_budget(&estimate, mode)?;

        let request_hash =
            AssetCache::request_hash(PROVIDER, CLUSTER_FACE_DETECT, request)?;
        if let Some(manifest) = self.cache.hit(PROVIDER, &request_hash)? {
            let response: FaceDetectResult = serde_json::from_value(manifest.response.clone())
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
            let response = FaceDetectResult {
                provider: PROVIDER.into(),
                image_width: 0,
                image_height: 0,
                detections: vec![],
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

        let url = self.build_url(&request.image_url);
        let raw = ureq::get(&url)
            .set("Accept", "application/json")
            .call();
        let body = match raw {
            Ok(r) => r
                .into_string()
                .map_err(|e| BackendError::Transport(e.to_string()))?,
            Err(ureq::Error::Status(status, response)) => {
                let body = response.into_string().unwrap_or_default();
                return Err(BackendError::HttpStatus {
                    status,
                    body: truncate(body),
                });
            }
            Err(e) => return Err(BackendError::Transport(e.to_string())),
        };
        let parsed: DetectResponse = serde_json::from_str(&body)
            .map_err(|e| BackendError::Decode(format!("face-detect: {e}")))?;
        let result = build_result(parsed, request.min_confidence);

        let manifest = Manifest {
            version: 1,
            provider: PROVIDER.into(),
            cluster: CLUSTER_FACE_DETECT.into(),
            request_hash: request_hash.clone(),
            request: serde_json::to_value(request)
                .map_err(|e| BackendError::Cache(format!("serialize request: {e}")))?,
            response: serde_json::to_value(&result)
                .map_err(|e| BackendError::Cache(format!("serialize response: {e}")))?,
            cost_estimate_usd: estimate.cost_usd,
            asset_path: None,
            created_at: utc_now_iso8601(),
        };
        self.cache.store(&manifest)?;

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

/// Wire response from `detect.roboflow.com/...`. `predictions[]` carries
/// the boxes; `image` echoes the source dimensions.
#[derive(Debug, Deserialize)]
struct DetectResponse {
    #[serde(default)]
    image: DetectImageDims,
    #[serde(default)]
    predictions: Vec<DetectPrediction>,
}

#[derive(Debug, Default, Deserialize)]
struct DetectImageDims {
    #[serde(default)]
    width: u32,
    #[serde(default)]
    height: u32,
}

#[derive(Debug, Deserialize)]
struct DetectPrediction {
    /// Center-X of the box in image pixels.
    x: f32,
    /// Center-Y.
    y: f32,
    width: f32,
    height: f32,
    confidence: f32,
}

/// Convert the wire response to the cluster-shaped result. Drops
/// detections below `min_confidence`, converts center-coords to
/// top-left, clamps into image bounds, sorts by descending confidence.
fn build_result(raw: DetectResponse, min_confidence: f32) -> FaceDetectResult {
    let iw = raw.image.width;
    let ih = raw.image.height;
    let mut dets: Vec<FaceDetection> = raw
        .predictions
        .into_iter()
        .filter(|p| p.confidence + 1e-6 >= min_confidence)
        .map(|p| {
            let half_w = (p.width * 0.5).max(0.0);
            let half_h = (p.height * 0.5).max(0.0);
            let x = (p.x - half_w).max(0.0).round() as u32;
            let y = (p.y - half_h).max(0.0).round() as u32;
            let w = p.width.max(0.0).round() as u32;
            let h = p.height.max(0.0).round() as u32;
            let (x, y, w, h) = clamp_bbox(x, y, w, h, iw, ih);
            FaceDetection {
                bbox: [x, y, w, h],
                confidence: p.confidence,
            }
        })
        .collect();
    dets.sort_by(|a, b| {
        b.confidence
            .partial_cmp(&a.confidence)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    FaceDetectResult {
        provider: PROVIDER.into(),
        image_width: iw,
        image_height: ih,
        detections: dets,
    }
}

/// Clamp `[x, y, w, h]` into `[0, image_w) × [0, image_h)`. When the
/// source dims are zero (unknown), the bbox is passed through unclamped.
fn clamp_bbox(
    mut x: u32,
    mut y: u32,
    mut w: u32,
    mut h: u32,
    iw: u32,
    ih: u32,
) -> (u32, u32, u32, u32) {
    if iw == 0 || ih == 0 {
        return (x, y, w, h);
    }
    if x >= iw {
        x = iw.saturating_sub(1);
    }
    if y >= ih {
        y = ih.saturating_sub(1);
    }
    if x + w > iw {
        w = iw.saturating_sub(x);
    }
    if y + h > ih {
        h = ih.saturating_sub(y);
    }
    (x, y, w, h)
}

/// Minimal percent-encoder — enough to make image URLs and API keys
/// safe to drop into a query string. Encodes everything that isn't an
/// unreserved RFC-3986 character.
fn percent_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        let safe = b.is_ascii_alphanumeric()
            || b == b'-'
            || b == b'_'
            || b == b'.'
            || b == b'~';
        if safe {
            out.push(b as char);
        } else {
            out.push('%');
            out.push_str(&format!("{b:02X}"));
        }
    }
    out
}

/// Truncate a response body for inclusion in error messages.
fn truncate(s: String) -> String {
    if s.len() <= 512 {
        s
    } else {
        format!("{}…", &s[..512])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fresh_cache() -> AssetCache {
        let tmp = std::env::temp_dir().join(format!(
            "wavelet-rf-facedet-{}",
            AssetCache::request_hash("seed", "seed", &"x").unwrap()
        ));
        let _ = std::fs::remove_dir_all(&tmp);
        AssetCache::new(tmp)
    }

    fn mk() -> RoboflowFaceDetectAdapter {
        RoboflowFaceDetectAdapter::with_key("rf-test-key", fresh_cache())
    }

    #[test]
    fn url_encodes_image_param() {
        let a = mk();
        let url = a.build_url("https://example.com/photo with spaces.jpg?v=1");
        assert!(url.starts_with(DETECT_BASE));
        assert!(url.contains("api_key=rf-test-key"));
        assert!(url.contains("photo%20with%20spaces.jpg"));
        // The '?' inside the image URL must be percent-encoded so the
        // server sees a single query block — `%3F` is the canonical
        // encoding.
        assert!(url.contains("%3Fv%3D1"));
    }

    #[test]
    fn build_result_converts_center_coords_to_top_left() {
        let raw = DetectResponse {
            image: DetectImageDims {
                width: 400,
                height: 352,
            },
            predictions: vec![DetectPrediction {
                x: 200.5,
                y: 114.0,
                width: 141.0,
                height: 212.0,
                confidence: 0.89,
            }],
        };
        let r = build_result(raw, 0.5);
        assert_eq!(r.image_width, 400);
        assert_eq!(r.image_height, 352);
        assert_eq!(r.detections.len(), 1);
        let [x, y, w, h] = r.detections[0].bbox;
        // 200.5 - 70.5 = 130 ; 114 - 106 = 8
        assert_eq!((x, y, w, h), (130, 8, 141, 212));
    }

    #[test]
    fn build_result_drops_below_threshold() {
        let raw = DetectResponse {
            image: DetectImageDims {
                width: 100,
                height: 100,
            },
            predictions: vec![
                DetectPrediction {
                    x: 50.0,
                    y: 50.0,
                    width: 20.0,
                    height: 20.0,
                    confidence: 0.3,
                },
                DetectPrediction {
                    x: 50.0,
                    y: 50.0,
                    width: 20.0,
                    height: 20.0,
                    confidence: 0.95,
                },
            ],
        };
        let r = build_result(raw, 0.5);
        assert_eq!(r.detections.len(), 1);
        assert!((r.detections[0].confidence - 0.95).abs() < 1e-6);
    }

    #[test]
    fn build_result_sorts_by_descending_confidence() {
        let raw = DetectResponse {
            image: DetectImageDims {
                width: 100,
                height: 100,
            },
            predictions: vec![
                DetectPrediction {
                    x: 50.0,
                    y: 50.0,
                    width: 20.0,
                    height: 20.0,
                    confidence: 0.6,
                },
                DetectPrediction {
                    x: 50.0,
                    y: 50.0,
                    width: 20.0,
                    height: 20.0,
                    confidence: 0.95,
                },
                DetectPrediction {
                    x: 50.0,
                    y: 50.0,
                    width: 20.0,
                    height: 20.0,
                    confidence: 0.8,
                },
            ],
        };
        let r = build_result(raw, 0.5);
        assert_eq!(r.detections.len(), 3);
        assert!(r.detections[0].confidence >= r.detections[1].confidence);
        assert!(r.detections[1].confidence >= r.detections[2].confidence);
    }

    #[test]
    fn build_result_clamps_bbox_into_image() {
        let raw = DetectResponse {
            image: DetectImageDims {
                width: 100,
                height: 100,
            },
            predictions: vec![DetectPrediction {
                // Center near right edge, oversized — would overshoot.
                x: 95.0,
                y: 95.0,
                width: 40.0,
                height: 40.0,
                confidence: 0.9,
            }],
        };
        let r = build_result(raw, 0.5);
        let [x, y, w, h] = r.detections[0].bbox;
        assert!(x + w <= 100);
        assert!(y + h <= 100);
    }

    #[test]
    fn empty_image_url_rejected() {
        let a = mk();
        assert!(matches!(
            a.detect_faces(&FaceDetectRequest::new(""), RunMode::DryRun)
                .unwrap_err(),
            BackendError::InvalidRequest(_)
        ));
    }

    #[test]
    fn data_url_rejected() {
        let a = mk();
        let req = FaceDetectRequest::new("data:image/png;base64,iVBORw0KGgo=");
        assert!(matches!(
            a.detect_faces(&req, RunMode::DryRun).unwrap_err(),
            BackendError::InvalidRequest(_)
        ));
    }

    #[test]
    fn bad_min_confidence_rejected() {
        let a = mk();
        let req = FaceDetectRequest::new("https://x/y.jpg").with_min_confidence(2.0);
        assert!(matches!(
            a.detect_faces(&req, RunMode::DryRun).unwrap_err(),
            BackendError::InvalidRequest(_)
        ));
    }

    #[test]
    fn dry_run_does_not_hit_network() {
        let a = mk();
        let req = FaceDetectRequest::new("https://x/y.jpg");
        let out = a.detect_faces(&req, RunMode::DryRun).unwrap();
        assert_eq!(out.mode, "dry-run");
        assert_eq!(out.response.provider, PROVIDER);
        assert!(out.response.detections.is_empty());
    }

    #[test]
    fn over_budget_blocks_before_network() {
        let a = mk();
        let req = FaceDetectRequest::new("https://x/y.jpg");
        let err = a
            .detect_faces(&req, RunMode::Live { max_cost_usd: 0.0 })
            .unwrap_err();
        assert!(matches!(err, BackendError::OverBudget { .. }));
    }

    #[test]
    fn cost_estimate_carries_provider() {
        let a = mk();
        let req = FaceDetectRequest::new("https://x/y.jpg");
        let est = a.estimate_cost(&req);
        assert_eq!(est.provider, PROVIDER);
        assert!(est.cost_usd < 0.01);
    }

    #[test]
    fn wire_response_parses_real_payload() {
        // Captured from a live probe of face-detection-mik1i/18:
        let body = r#"{"inference_id":"ff262f97","time":0.05,
            "image":{"width":400,"height":352},
            "predictions":[{"x":200.5,"y":114.0,"width":141.0,"height":212.0,
                "confidence":0.8912945985794067,"class":"face","class_id":0,
                "detection_id":"7e90f115"}]}"#;
        let parsed: DetectResponse = serde_json::from_str(body).unwrap();
        assert_eq!(parsed.image.width, 400);
        assert_eq!(parsed.predictions.len(), 1);
        let r = build_result(parsed, 0.5);
        assert_eq!(r.detections.len(), 1);
    }
}
