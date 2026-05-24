//! Model download + cache for PaddleOCR v5 ONNX weights.
//!
//! Download target: `~/.wavelet/models/ocr/`
//!
//! | File          | Source URL                          | Size  |
//! |---------------|-------------------------------------|-------|
//! | `det.onnx`    | RapidOCR PP-OCRv5 detection ONNX    | ~4 MB |
//! | `rec.onnx`    | RapidOCR PP-OCRv5 recognition ONNX  | ~52 MB|
//! | `rec_keys.txt`| Character vocabulary (6623 entries) | ~80 KB|
//!
//! Models are from the RapidOCR HuggingFace repo:
//! `https://huggingface.co/SWHL/RapidOCR/resolve/main/`
//!
//! Set `WAVELET_OCR_MODEL_DIR` to override the default cache path.
//! This lets tests provide fixture models without touching `~/.wavelet`.

use std::path::{Path, PathBuf};

use super::OcrError;

/// Expected file names inside the model directory.
pub const DET_FILE: &str = "ch_PP-OCRv5_det_infer.onnx";
/// Recognition ONNX model filename.
pub const REC_FILE: &str = "ch_PP-OCRv5_rec_infer.onnx";
/// Character vocabulary filename.
pub const KEYS_FILE: &str = "ppocr_keys_v1.txt";

/// Download URLs for each model file.
const DET_URL: &str =
    "https://huggingface.co/SWHL/RapidOCR/resolve/main/models/ch_PP-OCRv5_det_infer.onnx";
/// Recognition model URL.
const REC_URL: &str =
    "https://huggingface.co/SWHL/RapidOCR/resolve/main/models/ch_PP-OCRv5_rec_infer.onnx";
/// Keys file URL.
const KEYS_URL: &str =
    "https://huggingface.co/SWHL/RapidOCR/resolve/main/models/ppocr_keys_v1.txt";

/// Returns the model directory path. Checks `WAVELET_OCR_MODEL_DIR`
/// first, falls back to `~/.wavelet/models/ocr/`.
pub fn model_dir() -> PathBuf {
    if let Ok(d) = std::env::var("WAVELET_OCR_MODEL_DIR") {
        return PathBuf::from(d);
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
    PathBuf::from(home).join(".wavelet").join("models").join("ocr")
}

/// Returns `true` if all three model files exist in `dir`.
pub fn models_present(dir: &Path) -> bool {
    dir.join(DET_FILE).exists() && dir.join(REC_FILE).exists() && dir.join(KEYS_FILE).exists()
}

/// Ensure all model files are present, downloading any that are missing.
///
/// Uses `ureq` for synchronous HTTP; expects a network connection.
/// Skips files that already exist. Returns the model directory path.
pub fn ensure_models() -> Result<PathBuf, OcrError> {
    let dir = model_dir();
    std::fs::create_dir_all(&dir)?;

    let pairs = [
        (DET_FILE, DET_URL),
        (REC_FILE, REC_URL),
        (KEYS_FILE, KEYS_URL),
    ];

    for (file, url) in &pairs {
        let dest = dir.join(file);
        if dest.exists() {
            continue;
        }
        eprintln!("wavelet ocr: downloading {file} …");
        download_file(url, &dest)?;
        eprintln!("wavelet ocr: saved {}", dest.display());
    }

    Ok(dir)
}

/// Download `url` to `dest` using ureq. Streams through a 64 KB buffer.
fn download_file(url: &str, dest: &Path) -> Result<(), OcrError> {
    let response = ureq::get(url)
        .call()
        .map_err(|e| OcrError::Http(e.to_string()))?;

    if response.status() != 200 {
        return Err(OcrError::Http(format!(
            "HTTP {} for {url}",
            response.status()
        )));
    }

    // Write to a temp file then rename for atomic delivery.
    let tmp = dest.with_extension("tmp");
    {
        let mut reader = response.into_reader();
        let mut file = std::fs::File::create(&tmp)?;
        std::io::copy(&mut reader, &mut file)?;
    }
    std::fs::rename(&tmp, dest)?;
    Ok(())
}

/// Load the recognition character vocabulary from `keys_path`.
/// Returns a `Vec<String>` with a blank-symbol inserted at index 0
/// (CTC convention: label 0 is always the blank token).
pub fn load_keys(keys_path: &Path) -> Result<Vec<String>, OcrError> {
    let text = std::fs::read_to_string(keys_path)?;
    let mut keys = vec!["blank".to_string()]; // index 0 = CTC blank
    for line in text.lines() {
        let s = line.trim();
        if !s.is_empty() {
            keys.push(s.to_string());
        }
    }
    Ok(keys)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_dir_uses_env_override() {
        // Safety: tests may run in parallel; use a thread-local env override
        // pattern. We just verify the env var is respected at all.
        let tmp = std::env::temp_dir().join("wavelet-ocr-test-models");
        // Set + restore env var within a single-threaded unit test.
        let prev = std::env::var("WAVELET_OCR_MODEL_DIR").ok();
        std::env::set_var("WAVELET_OCR_MODEL_DIR", &tmp);
        let result = model_dir();
        match prev {
            Some(v) => std::env::set_var("WAVELET_OCR_MODEL_DIR", v),
            None => std::env::remove_var("WAVELET_OCR_MODEL_DIR"),
        }
        assert_eq!(result, tmp);
    }

    #[test]
    fn models_absent_when_dir_missing() {
        let absent = PathBuf::from("/tmp/no-such-wavelet-ocr-dir-xyzabc");
        assert!(!models_present(&absent));
    }

    #[test]
    fn load_keys_prepends_blank() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(tmp.path(), "a\nb\nc\n").unwrap();
        let keys = load_keys(tmp.path()).unwrap();
        assert_eq!(keys[0], "blank");
        assert_eq!(keys[1], "a");
        assert_eq!(keys.len(), 4); // blank + a + b + c
    }
}
