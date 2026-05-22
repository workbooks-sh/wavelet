//! Baked-text detector.
//!
//! Status: **stub**.
//!
//! The intent is to flag text already rendered into a generated still
//! (license plates, signage, watermarks) so an HTML overlay does not
//! collide with it. We probed three Fal-hosted OCR endpoints on
//! 2026-05-18:
//!
//! | endpoint              | sync (`fal.run`) | queue (`queue.fal.run`) |
//! |-----------------------|------------------|-------------------------|
//! | `fal-ai/easyocr`      | 404              | 404                     |
//! | `fal-ai/tesseract`    | 404              | 404                     |
//! | `fal-ai/got-ocr`      | 404              | 200 (queued)            |
//!
//! Only `fal-ai/got-ocr` exists, and only via the queue API — which
//! requires submit + poll + fetch (multi-step). The existing
//! `FalClient::post_sync` adapter is synchronous and assumes
//! `fal.run/<model>`, so wiring `got-ocr` here would require a new
//! queue adapter on `FalClient`. That belongs in its own change.
//! Until it lands, this module returns `AnalysisError::Unimplemented`.
//!
//! A pure-Rust local fallback (e.g. `tesseract-rs`, `rusty-tesseract`)
//! brings a C dependency and 100 MB of language data — out of scope
//! for now.

use super::{AnalysisError, BoundingRect};
use serde::{Deserialize, Serialize};
use std::path::Path;

/// One OCR hit.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OcrHit {
    /// Recognized text.
    pub text: String,
    /// Bounding box in image pixel coordinates.
    pub bbox: BoundingRect,
    /// Confidence in `0.0..=1.0` (provider-reported when available).
    pub confidence: f32,
}

/// OCR analysis result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OcrReport {
    /// Backend identifier (e.g. `"fal-ai/got-ocr"` once wired).
    pub backend: String,
    /// Detected text hits.
    pub hits: Vec<OcrHit>,
}

/// Run OCR on the image at `image_path`. Currently always returns
/// `AnalysisError::Unimplemented` — see module docs.
pub fn analyze(_image_path: &Path) -> Result<OcrReport, AnalysisError> {
    Err(AnalysisError::Unimplemented(
        "ocr backend not wired — fal-ai/got-ocr is the only viable endpoint and requires the queue API (submit/poll/fetch)",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn dummy_path() -> PathBuf {
        PathBuf::from("/tmp/no-such-image.png")
    }

    #[test]
    fn analyze_returns_unimplemented() {
        match analyze(&dummy_path()) {
            Err(AnalysisError::Unimplemented(msg)) => {
                assert!(
                    msg.contains("got-ocr"),
                    "error message should mention got-ocr: {msg}"
                );
            }
            other => panic!("expected Unimplemented, got {other:?}"),
        }
    }

    #[test]
    fn hit_roundtrips_json() {
        let hit = OcrHit {
            text: "STOP".into(),
            bbox: BoundingRect::new(10, 20, 100, 40),
            confidence: 0.93,
        };
        let s = serde_json::to_string(&hit).unwrap();
        let back: OcrHit = serde_json::from_str(&s).unwrap();
        assert_eq!(back.text, "STOP");
        assert_eq!(back.bbox.w, 100);
        assert!((back.confidence - 0.93).abs() < 1e-6);
    }

    #[test]
    fn report_with_no_hits_serializes() {
        let rep = OcrReport {
            backend: "stub".into(),
            hits: vec![],
        };
        let s = serde_json::to_string(&rep).unwrap();
        assert!(s.contains("\"backend\":\"stub\""));
        assert!(s.contains("\"hits\":[]"));
    }
}
