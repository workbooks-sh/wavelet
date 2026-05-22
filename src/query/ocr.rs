//! OCR-based text verification — "is the string my comp.json says should
//! render in `#headline` actually present in the pixels at this frame?"
//! Phase 5 of epic wb-q4a6.
//!
//! Implementation: shell out to the `tesseract` binary. Zero linked code,
//! zero added binary footprint; tesseract is a documented runtime dep
//! (`brew install tesseract` on macOS, the equivalent on linux).
//!
//! The critical optimization is **crop before OCR**: we use the
//! FrameSnapshot's per-element bbox to extract a ~200×80 patch and feed
//! only that to Tesseract. Full-frame OCR at 1280×720 takes ~600ms;
//! cropped OCR is ~80–150ms.

use super::pixels::FramePixels;
use super::snapshot::{FrameSnapshot, Rect};
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::PathBuf;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

/// Result of a single `--text-visible` query.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TextVisibleResult {
    /// True when OCR output matches `expected` within `tolerance` edits.
    pub ok: bool,
    /// The expected string the caller supplied.
    pub expected: String,
    /// The string OCR returned (whitespace-normalized, trimmed). None on
    /// OCR / IO failure — see `error`.
    pub detected: Option<String>,
    /// Levenshtein distance between `expected` (lowercased, whitespace-
    /// collapsed) and `detected` (same normalization). None on failure.
    pub edit_distance: Option<u32>,
    /// Max allowed edit distance (passed in by caller).
    pub tolerance: u32,
    /// Selector the bbox came from. None when caller didn't restrict.
    pub selector: Option<String>,
    /// Cropped region used for OCR. None when full-frame.
    pub bbox: Option<Rect>,
    /// Human-readable error when OCR could not run.
    pub error: Option<String>,
}

/// Run Tesseract on the cropped region of `pixels` corresponding to the
/// first node matching `in_selector` (or the full frame when None), and
/// check whether the recognized text contains `expected` within `tolerance`
/// edits.
///
/// `padding` is the pixel margin added around the bbox before cropping;
/// 8 px is a good default — Tesseract is more accurate with a few pixels
/// of background on each edge.
pub fn text_visible(
    snap: &FrameSnapshot,
    pixels: &FramePixels,
    expected: &str,
    in_selector: Option<&str>,
    tolerance: u32,
    padding: u32,
) -> TextVisibleResult {
    if which_tesseract().is_none() {
        return TextVisibleResult {
            ok: false,
            expected: expected.to_string(),
            detected: None,
            edit_distance: None,
            tolerance,
            selector: in_selector.map(String::from),
            bbox: None,
            error: Some(
                "tesseract binary not found in PATH. install with `brew install tesseract` on macOS"
                    .into(),
            ),
        };
    }

    let bbox = match in_selector {
        Some(sel) => match snap.select(sel).first() {
            Some(n) => Some(pad_bbox(n.bbox, padding as f32, pixels.width, pixels.height)),
            None => {
                return TextVisibleResult {
                    ok: false,
                    expected: expected.to_string(),
                    detected: None,
                    edit_distance: None,
                    tolerance,
                    selector: Some(sel.to_string()),
                    bbox: None,
                    error: Some(format!("selector {} not found in scene-graph at t={:.2}s", sel, snap.t_secs)),
                };
            }
        },
        None => None,
    };

    let crop = match bbox {
        Some(b) => crop_rgba(pixels, b),
        None => (pixels.rgba.clone(), pixels.width, pixels.height),
    };

    let png_path = match write_temp_png(&crop.0, crop.1, crop.2) {
        Ok(p) => p,
        Err(e) => {
            return TextVisibleResult {
                ok: false,
                expected: expected.to_string(),
                detected: None,
                edit_distance: None,
                tolerance,
                selector: in_selector.map(String::from),
                bbox,
                error: Some(format!("temp png write failed: {e}")),
            };
        }
    };

    let raw = match run_tesseract(&png_path) {
        Ok(s) => s,
        Err(e) => {
            let _ = std::fs::remove_file(&png_path);
            return TextVisibleResult {
                ok: false,
                expected: expected.to_string(),
                detected: None,
                edit_distance: None,
                tolerance,
                selector: in_selector.map(String::from),
                bbox,
                error: Some(format!("tesseract: {e}")),
            };
        }
    };
    let _ = std::fs::remove_file(&png_path);

    let detected = normalize(&raw);
    let want = normalize(expected);
    let dist = levenshtein(&detected, &want);
    let ok = dist as u32 <= tolerance;

    TextVisibleResult {
        ok,
        expected: expected.to_string(),
        detected: Some(raw.trim().to_string()),
        edit_distance: Some(dist as u32),
        tolerance,
        selector: in_selector.map(String::from),
        bbox,
        error: None,
    }
}

/// Cached probe — `tesseract --version` succeeded somewhere in PATH.
fn which_tesseract() -> Option<()> {
    static SEEN: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    if *SEEN.get_or_init(|| {
        Command::new("tesseract")
            .arg("--version")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }) {
        Some(())
    } else {
        None
    }
}

/// Pad a bbox by `pad` pixels on each side, clamped to the frame.
fn pad_bbox(b: Rect, pad: f32, w: u32, h: u32) -> Rect {
    let x = (b.x - pad).max(0.0);
    let y = (b.y - pad).max(0.0);
    let x2 = (b.x + b.w + pad).min(w as f32);
    let y2 = (b.y + b.h + pad).min(h as f32);
    Rect {
        x,
        y,
        w: (x2 - x).max(1.0),
        h: (y2 - y).max(1.0),
    }
}

/// Extract a rectangular region of the frame buffer as a fresh RGBA Vec.
/// Returns `(rgba, width, height)` for the cropped patch.
fn crop_rgba(pixels: &FramePixels, b: Rect) -> (Vec<u8>, u32, u32) {
    let x0 = b.x.max(0.0) as u32;
    let y0 = b.y.max(0.0) as u32;
    let x1 = ((b.x + b.w) as i32).min(pixels.width as i32).max(0) as u32;
    let y1 = ((b.y + b.h) as i32).min(pixels.height as i32).max(0) as u32;
    let w = x1.saturating_sub(x0).max(1);
    let h = y1.saturating_sub(y0).max(1);
    let mut out = Vec::with_capacity((w * h * 4) as usize);
    for y in y0..y1 {
        let row_start = ((y * pixels.width + x0) * 4) as usize;
        let row_end = ((y * pixels.width + x1) * 4) as usize;
        out.extend_from_slice(&pixels.rgba[row_start..row_end]);
    }
    (out, w, h)
}

/// Write an RGBA buffer to a temp PNG. Returns the file path.
fn write_temp_png(rgba: &[u8], w: u32, h: u32) -> std::io::Result<PathBuf> {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    let path = std::env::temp_dir().join(format!(
        "wavelet-ocr-{}-{}.png",
        std::process::id(),
        nanos
    ));
    let file = std::fs::File::create(&path)?;
    let mut encoder = png::Encoder::new(file, w, h);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder
        .write_header()
        .map_err(|e| std::io::Error::other(format!("png header: {e}")))?;
    writer
        .write_image_data(rgba)
        .map_err(|e| std::io::Error::other(format!("png write: {e}")))?;
    writer
        .finish()
        .map_err(|e| std::io::Error::other(format!("png finish: {e}")))?;
    Ok(path)
}

/// Run `tesseract <png> stdout --psm 7` (single-line) and return the text.
/// Falls back to `--psm 6` (block) if the single-line pass returns empty.
fn run_tesseract(png_path: &PathBuf) -> Result<String, String> {
    let psm7 = run_tesseract_with_psm(png_path, "7")?;
    let trimmed = psm7.trim();
    if !trimmed.is_empty() {
        return Ok(psm7);
    }
    run_tesseract_with_psm(png_path, "6")
}

fn run_tesseract_with_psm(png_path: &PathBuf, psm: &str) -> Result<String, String> {
    let out = Command::new("tesseract")
        .arg(png_path)
        .arg("stdout")
        .arg("--psm")
        .arg(psm)
        // Quiet — tesseract logs to stderr by default; we don't want it
        // polluting the agent's JSON.
        .arg("-l")
        .arg("eng")
        .stderr(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .output()
        .map_err(|e| format!("spawn: {e}"))?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        return Err(format!("nonzero exit: {err}"));
    }
    String::from_utf8(out.stdout).map_err(|e| format!("utf8: {e}"))
}

/// Lowercase + collapse runs of whitespace to single spaces + trim. Makes
/// the comparison resilient to tesseract's variable-whitespace output.
fn normalize(s: &str) -> String {
    s.to_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Levenshtein edit distance with the standard O(n*m) DP. Used to gate
/// OCR-vs-expected matches.
fn levenshtein(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let (n, m) = (a.len(), b.len());
    if n == 0 {
        return m;
    }
    if m == 0 {
        return n;
    }
    let mut prev: Vec<usize> = (0..=m).collect();
    let mut cur: Vec<usize> = vec![0; m + 1];
    for i in 1..=n {
        cur[0] = i;
        for j in 1..=m {
            let cost = if a[i - 1] == b[j - 1] { 0 } else { 1 };
            cur[j] = (prev[j] + 1).min(cur[j - 1] + 1).min(prev[j - 1] + cost);
        }
        // For the next row, what was `cur` becomes `prev`. Swap.
        // Manually allocating new vecs each iter would be wasteful for
        // long strings; mem::swap is constant-time.
        let _ = std::mem::replace(&mut prev, cur.clone());
    }
    prev[m]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_collapses_whitespace_and_lowercases() {
        assert_eq!(normalize("  Hello   WORLD\n"), "hello world");
        assert_eq!(normalize("ALL\tCAPS\nDONE"), "all caps done");
    }

    #[test]
    fn levenshtein_basic() {
        assert_eq!(levenshtein("", ""), 0);
        assert_eq!(levenshtein("abc", "abc"), 0);
        assert_eq!(levenshtein("abc", "abd"), 1);
        assert_eq!(levenshtein("kitten", "sitting"), 3);
        assert_eq!(levenshtein("", "abc"), 3);
        assert_eq!(levenshtein("abc", ""), 3);
    }

    #[test]
    fn pad_bbox_clamps_to_frame() {
        let b = Rect { x: 5.0, y: 5.0, w: 100.0, h: 50.0 };
        let r = pad_bbox(b, 20.0, 200, 100);
        // top-left clamps to (0,0), right edge unclamped, bottom clamps to 100.
        assert_eq!(r.x, 0.0);
        assert_eq!(r.y, 0.0);
        assert!(r.w >= 125.0);
        assert!(r.h <= 100.0);
    }
}
