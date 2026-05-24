//! Model-file management for Depth Anything V2 Small.
//!
//! Downloads the fp16-quantized ONNX model from Hugging Face on first
//! use and caches it at `~/.wavelet/models/depth/depth-anything-v2-small.onnx`.
//! The file is ~25 MB; subsequent runs skip the download entirely.

use std::path::{Path, PathBuf};

/// Remote location of the fp16 ONNX model.
///
/// The `onnx-community/depth-anything-v2-small` repository on Hugging
/// Face hosts a pre-quantized fp16 model that was produced by
/// `onnxconverter_common.float16.convert_float_to_float16(...,
/// keep_io_types=True)`. The IO stays fp32 so the Rust caller does not
/// need to cast input/output tensors.
pub const MODEL_URL: &str =
    "https://huggingface.co/onnx-community/depth-anything-v2-small/resolve/main/onnx/model_fp16.onnx";

/// Filename used on disk inside the model cache directory.
pub const MODEL_FILE: &str = "depth-anything-v2-small.onnx";

/// Return the local path to the model file, downloading it from
/// Hugging Face if it does not exist yet.
///
/// Creates `~/.wavelet/models/depth/` on first call.
///
/// # Errors
///
/// Returns a `String` describing the failure when the cache directory
/// cannot be created, the download fails, or the file cannot be written.
pub fn ensure_model() -> Result<PathBuf, String> {
    let path = model_path()?;
    if path.exists() {
        return Ok(path);
    }
    eprintln!(
        "wavelet depth: downloading Depth Anything V2 Small (~25 MB) → {}",
        path.display()
    );
    download(MODEL_URL, &path)?;
    Ok(path)
}

/// Resolve `~/.wavelet/models/depth/<MODEL_FILE>`.
fn model_path() -> Result<PathBuf, String> {
    let home = home_dir().ok_or_else(|| "cannot determine home directory".to_string())?;
    let dir = home.join(".wavelet").join("models").join("depth");
    std::fs::create_dir_all(&dir)
        .map_err(|e| format!("create model dir {}: {e}", dir.display()))?;
    Ok(dir.join(MODEL_FILE))
}

/// Download `url` to `dest`, writing the file atomically via a tmp
/// sibling. Uses `ureq` — the same blocking HTTP client used elsewhere
/// in wavelet so we don't pull in an async runtime just for one fetch.
fn download(url: &str, dest: &Path) -> Result<(), String> {
    let resp = ureq::get(url)
        .call()
        .map_err(|e| format!("fetch {url}: {e}"))?;

    let tmp = dest.with_extension("onnx.tmp");
    {
        let mut file = std::fs::File::create(&tmp)
            .map_err(|e| format!("create {}: {e}", tmp.display()))?;
        std::io::copy(&mut resp.into_reader(), &mut file)
            .map_err(|e| format!("write {}: {e}", tmp.display()))?;
    }
    std::fs::rename(&tmp, dest)
        .map_err(|e| format!("rename {} → {}: {e}", tmp.display(), dest.display()))?;
    Ok(())
}

/// Portable home-directory probe. Tries `$HOME`, falls back to the
/// passwd entry on Unix. Returns `None` when neither is available
/// (e.g. no-login CI environments without a real home).
fn home_dir() -> Option<PathBuf> {
    if let Ok(h) = std::env::var("HOME") {
        if !h.is_empty() {
            return Some(PathBuf::from(h));
        }
    }
    // On Unix-like systems, try std's deprecated helper as a fallback.
    #[allow(deprecated)]
    std::env::home_dir()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_path_is_under_home() {
        if let Ok(p) = model_path() {
            assert!(p.to_string_lossy().contains(".wavelet"));
            assert!(p.to_string_lossy().contains("depth"));
        }
    }
}
