//! ONNX-based OCR engine. Feature-gated behind `ocr` cargo feature.
//!
//! Uses PaddleOCR v5 ONNX models (DBNet detection + SVTR/CTC recognition)
//! sourced from the RapidOCR HuggingFace bundles. On macOS the CoreML
//! execution provider is wired for ~3× speed improvement vs CPU-only.
//!
//! ## Opt-in
//! ```toml
//! [features]
//! ocr = ["ort", "ndarray"]
//! ```
//!
//! ## First-run model download
//! Call [`ensure_models`] before [`run_ocr`]. Models are cached under
//! `~/.wavelet/models/ocr/` (≈ 60 MB total, one-time download).
//!
//! ## Feature gate
//! When the `ocr` feature is **not** enabled, this module still compiles
//! but all public functions return [`OcrError::FeatureDisabled`].

pub mod models;
pub mod run;

pub use run::{OcrBox, OcrResult, run_ocr};
pub use models::ensure_models;

/// Errors from the OCR subsystem.
#[derive(Debug)]
pub enum OcrError {
    /// The `ocr` cargo feature is not enabled. Recompile with
    /// `--features ocr` to activate ONNX inference.
    FeatureDisabled,
    /// Model files are missing and download was not attempted / failed.
    ModelsMissing(String),
    /// ONNX runtime session creation or inference error.
    #[cfg(feature = "ocr")]
    Runtime(ort::Error),
    /// I/O error during model download or frame read.
    Io(std::io::Error),
    /// HTTP error during model download.
    Http(String),
    /// Detection produced no text regions.
    NoRegions,
}

impl std::fmt::Display for OcrError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OcrError::FeatureDisabled => write!(
                f,
                "OCR feature disabled — recompile with `--features ocr`"
            ),
            OcrError::ModelsMissing(p) => {
                write!(f, "OCR models missing at {p} — run `wavelet lint --download-ocr-models`")
            }
            #[cfg(feature = "ocr")]
            OcrError::Runtime(e) => write!(f, "ONNX runtime: {e}"),
            OcrError::Io(e) => write!(f, "I/O: {e}"),
            OcrError::Http(s) => write!(f, "HTTP: {s}"),
            OcrError::NoRegions => write!(f, "no text regions detected"),
        }
    }
}

impl From<std::io::Error> for OcrError {
    fn from(e: std::io::Error) -> Self {
        OcrError::Io(e)
    }
}

#[cfg(feature = "ocr")]
impl From<ort::Error> for OcrError {
    fn from(e: ort::Error) -> Self {
        OcrError::Runtime(e)
    }
}
