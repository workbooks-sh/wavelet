//! OCR inference — detection + recognition pipeline.
//!
//! ## Architecture
//!
//! ```text
//! RGBA frame
//!   │
//!   ▼
//! DBNet det.onnx  ──────► text region bboxes
//!   │
//!   ▼  (per bbox crop)
//! SVTR/CTC rec.onnx ────► character sequence + per-step confidence
//!   │
//!   ▼
//! OcrBox list
//! ```
//!
//! When the `ocr` feature is disabled every public function returns
//! [`OcrError::FeatureDisabled`].

use std::path::Path;

use super::OcrError;
#[cfg(feature = "ocr")]
use super::models;

/// One detected text region with its recognized content and confidence.
#[derive(Debug, Clone)]
pub struct OcrBox {
    /// Recognized UTF-8 text content.
    pub text: String,
    /// Left pixel coordinate of the detection bbox (in the input frame).
    pub x: u32,
    /// Top pixel coordinate.
    pub y: u32,
    /// Bbox width in pixels.
    pub w: u32,
    /// Bbox height in pixels.
    pub h: u32,
    /// Mean per-step max-softmax confidence, in `0.0..=1.0`. Values
    /// below ~0.6 typically indicate garbled or uncertain letterforms.
    pub confidence: f32,
}

/// OCR output for one frame.
#[derive(Debug, Clone)]
pub struct OcrResult {
    /// All detected + recognized boxes, in reading order (top-to-bottom,
    /// left-to-right within rows).
    pub boxes: Vec<OcrBox>,
    /// Backend identifier for the finding message.
    pub backend: &'static str,
}

/// Run OCR on a raw RGBA frame buffer.
///
/// `rgba` must be a row-major top-down buffer of `width × height × 4` bytes.
/// `model_dir` must contain `det.onnx`, `rec.onnx`, and `rec_keys.txt`
/// (use [`models::ensure_models`] to download them).
///
/// Requires the `ocr` cargo feature. Without it, returns
/// [`OcrError::FeatureDisabled`].
pub fn run_ocr(
    rgba: &[u8],
    width: u32,
    height: u32,
    model_dir: &Path,
) -> Result<OcrResult, OcrError> {
    #[cfg(feature = "ocr")]
    {
        run_ocr_impl(rgba, width, height, model_dir)
    }
    #[cfg(not(feature = "ocr"))]
    {
        let _ = (rgba, width, height, model_dir);
        Err(OcrError::FeatureDisabled)
    }
}

/// Stub run_ocr used in mock-based unit tests (no real ONNX sessions).
/// Callable regardless of `ocr` feature flag — accepts a closure that
/// returns a fake `OcrResult` for the given frame.
#[cfg(test)]
pub fn run_ocr_mock<F>(f: F) -> OcrResult
where
    F: FnOnce() -> OcrResult,
{
    f()
}

// ── Feature-gated ONNX implementation ─────────────────────────────────────

#[cfg(feature = "ocr")]
fn run_ocr_impl(
    rgba: &[u8],
    width: u32,
    height: u32,
    model_dir: &Path,
) -> Result<OcrResult, OcrError> {
    use ndarray::{Array, Array4, Axis, s};
    use ort::session::builder::GraphOptimizationLevel;
    use ort::{session::Session, inputs};

    // Load recognition vocabulary once per call (fast — file is ~80 KB).
    // In production the lint rule caches sessions across frames.
    let keys_path = model_dir.join(models::KEYS_FILE);
    let keys = models::load_keys(&keys_path)?;

    // ── Detection ─────────────────────────────────────────────────────────

    let det_path = model_dir.join(models::DET_FILE);
    let det_session = build_session(&det_path)?;

    // Scale input so the longer edge is ≤ 960 (multiple of 32).
    let (det_w, det_h) = scale_for_det(width, height, 960);
    let det_input = preprocess_rgb_for_det(rgba, width, height, det_w, det_h);

    let det_outputs = det_session.run(inputs!["x" => det_input.view()]?)?;
    let det_map = det_outputs["sigmoid_0.tmp_0"]
        .try_extract_tensor::<f32>()?;

    // Extract bounding boxes from the DBNet probability map.
    let boxes_norm = extract_bboxes(det_map.view(), 0.30);

    if boxes_norm.is_empty() {
        return Ok(OcrResult { boxes: vec![], backend: "paddleocr-v5-onnx" });
    }

    // ── Recognition ───────────────────────────────────────────────────────

    let rec_path = model_dir.join(models::REC_FILE);
    let rec_session = build_session(&rec_path)?;

    let mut ocr_boxes: Vec<OcrBox> = Vec::with_capacity(boxes_norm.len());

    for (nx, ny, nw, nh) in &boxes_norm {
        // Back-project normalised coords to original image pixels.
        let x = (nx * width as f32).round() as u32;
        let y = (ny * height as f32).round() as u32;
        let w = (nw * width as f32).round().max(1.0) as u32;
        let h = (nh * height as f32).round().max(1.0) as u32;

        let crop = crop_rgba(rgba, width, height, x, y, w, h);
        let rec_input = preprocess_crop_for_rec(&crop, w, h, 48);

        let rec_outputs = rec_session.run(inputs!["x" => rec_input.view()]?)?;
        // Output shape: [1, seq_len, num_classes]
        let logits = rec_outputs["softmax_11.tmp_0"]
            .try_extract_tensor::<f32>()?;
        let logits_3d = logits.view();

        let (text, confidence) = ctc_decode(&logits_3d, &keys);
        if text.is_empty() {
            continue;
        }

        ocr_boxes.push(OcrBox { text, x, y, w, h, confidence });
    }

    Ok(OcrResult { boxes: ocr_boxes, backend: "paddleocr-v5-onnx" })
}

// ── Helpers (feature-gated) ───────────────────────────────────────────────

#[cfg(feature = "ocr")]
fn build_session(path: &Path) -> Result<ort::session::Session, OcrError> {
    use ort::execution_providers::CoreMLExecutionProvider;
    use ort::session::builder::GraphOptimizationLevel;

    let builder = Session::builder()?
        .with_optimization_level(GraphOptimizationLevel::Level3)?;

    // Wire CoreML on Apple silicon / macOS for ~3× throughput.
    #[cfg(target_os = "macos")]
    let builder = builder.with_execution_providers([
        CoreMLExecutionProvider::default().build(),
    ])?;

    Ok(builder.commit_from_file(path)?)
}

/// Scale `(orig_w, orig_h)` so the longer edge is ≤ `max_edge` and both
/// dimensions are multiples of 32 (DBNet requirement).
#[cfg(feature = "ocr")]
fn scale_for_det(orig_w: u32, orig_h: u32, max_edge: u32) -> (u32, u32) {
    let scale = (max_edge as f32) / (orig_w.max(orig_h) as f32);
    let scale = scale.min(1.0); // never upscale
    let w = round_to_32((orig_w as f32 * scale) as u32);
    let h = round_to_32((orig_h as f32 * scale) as u32);
    (w.max(32), h.max(32))
}

#[cfg(feature = "ocr")]
fn round_to_32(v: u32) -> u32 {
    ((v + 31) / 32) * 32
}

/// Bilinear-downscale RGBA → RGB f32, normalize with ImageNet mean/std,
/// return shape [1, 3, H, W].
#[cfg(feature = "ocr")]
fn preprocess_rgb_for_det(
    rgba: &[u8],
    src_w: u32,
    src_h: u32,
    dst_w: u32,
    dst_h: u32,
) -> ndarray::Array4<f32> {
    use ndarray::Array4;

    let mut out = Array4::<f32>::zeros((1, 3, dst_h as usize, dst_w as usize));
    let mean = [0.485_f32, 0.456, 0.406];
    let std = [0.229_f32, 0.224, 0.225];

    for dy in 0..dst_h as usize {
        for dx in 0..dst_w as usize {
            // Bilinear sample position in source image.
            let sx = (dx as f32 + 0.5) * (src_w as f32 / dst_w as f32) - 0.5;
            let sy = (dy as f32 + 0.5) * (src_h as f32 / dst_h as f32) - 0.5;
            let [r, g, b] = sample_bilinear(rgba, src_w, src_h, sx, sy);
            out[[0, 0, dy, dx]] = (r / 255.0 - mean[0]) / std[0];
            out[[0, 1, dy, dx]] = (g / 255.0 - mean[1]) / std[1];
            out[[0, 2, dy, dx]] = (b / 255.0 - mean[2]) / std[2];
        }
    }
    out
}

/// Sample RGBA `rgba` at float coords `(sx, sy)` with bilinear interp.
/// Returns [R, G, B] in 0..255.
#[cfg(feature = "ocr")]
fn sample_bilinear(rgba: &[u8], w: u32, h: u32, sx: f32, sy: f32) -> [f32; 3] {
    let x0 = sx.floor().clamp(0.0, w as f32 - 1.0) as u32;
    let y0 = sy.floor().clamp(0.0, h as f32 - 1.0) as u32;
    let x1 = (x0 + 1).min(w - 1);
    let y1 = (y0 + 1).min(h - 1);
    let tx = (sx - sx.floor()).clamp(0.0, 1.0);
    let ty = (sy - sy.floor()).clamp(0.0, 1.0);

    let p00 = pixel_rgb(rgba, w, x0, y0);
    let p10 = pixel_rgb(rgba, w, x1, y0);
    let p01 = pixel_rgb(rgba, w, x0, y1);
    let p11 = pixel_rgb(rgba, w, x1, y1);

    std::array::from_fn(|c| {
        lerp(lerp(p00[c], p10[c], tx), lerp(p01[c], p11[c], tx), ty)
    })
}

#[cfg(feature = "ocr")]
fn pixel_rgb(rgba: &[u8], w: u32, x: u32, y: u32) -> [f32; 3] {
    let i = ((y * w + x) * 4) as usize;
    if i + 2 >= rgba.len() {
        return [0.0; 3];
    }
    [rgba[i] as f32, rgba[i + 1] as f32, rgba[i + 2] as f32]
}

#[cfg(feature = "ocr")]
fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

/// Extract bounding boxes from the DBNet sigmoid output tensor.
///
/// Input: shape [1, 1, H, W] probability map.
/// Output: list of (x, y, w, h) in normalised `[0, 1]` coordinates.
///
/// Simplified pipeline: threshold → connected-component bbox. Skips Vatti
/// polygon expansion because it adds ~30 LOC of geometry for marginal gain.
#[cfg(feature = "ocr")]
fn extract_bboxes(
    prob_map: ndarray::ArrayViewD<f32>,
    threshold: f32,
) -> Vec<(f32, f32, f32, f32)> {
    use ndarray::s;

    let shape = prob_map.shape();
    if shape.len() < 4 {
        return Vec::new();
    }
    let (map_h, map_w) = (shape[2], shape[3]);

    // Binary mask at the given threshold.
    let mut mask = vec![false; map_h * map_w];
    for y in 0..map_h {
        for x in 0..map_w {
            mask[y * map_w + x] = prob_map[[0, 0, y, x]] >= threshold;
        }
    }

    // Simple run-length connected-component labelling (4-connectivity).
    let mut labels = vec![0u32; map_h * map_w];
    let mut next_label = 1u32;

    for y in 0..map_h {
        for x in 0..map_w {
            if !mask[y * map_w + x] {
                continue;
            }
            let above = if y > 0 { labels[(y - 1) * map_w + x] } else { 0 };
            let left = if x > 0 { labels[y * map_w + x - 1] } else { 0 };
            let label = match (above, left) {
                (0, 0) => {
                    let l = next_label;
                    next_label += 1;
                    l
                }
                (a, 0) => a,
                (0, b) => b,
                (a, b) => a.min(b), // merge — simplified (no union-find)
            };
            labels[y * map_w + x] = label;
        }
    }

    // Collect bounding boxes per label.
    let n_labels = next_label as usize;
    let mut min_x = vec![map_w; n_labels];
    let mut min_y = vec![map_h; n_labels];
    let mut max_x = vec![0usize; n_labels];
    let mut max_y = vec![0usize; n_labels];
    let mut counts = vec![0u32; n_labels];

    for y in 0..map_h {
        for x in 0..map_w {
            let l = labels[y * map_w + x] as usize;
            if l == 0 {
                continue;
            }
            counts[l] += 1;
            if x < min_x[l] { min_x[l] = x; }
            if y < min_y[l] { min_y[l] = y; }
            if x > max_x[l] { max_x[l] = x; }
            if y > max_y[l] { max_y[l] = y; }
        }
    }

    let mut out = Vec::new();
    let min_area = 16u32; // skip tiny noise regions (< 4×4 px equivalent)

    for l in 1..n_labels {
        if counts[l] < min_area {
            continue;
        }
        let x = min_x[l] as f32 / map_w as f32;
        let y = min_y[l] as f32 / map_h as f32;
        let w = ((max_x[l] - min_x[l] + 1) as f32 / map_w as f32).max(0.0);
        let h = ((max_y[l] - min_y[l] + 1) as f32 / map_h as f32).max(0.0);
        if w > 0.0 && h > 0.0 {
            out.push((x, y, w, h));
        }
    }

    out
}

/// Crop RGBA frame to the given rect. Clamps to frame bounds.
#[cfg(feature = "ocr")]
fn crop_rgba(rgba: &[u8], fw: u32, fh: u32, x: u32, y: u32, w: u32, h: u32) -> Vec<u8> {
    let x = x.min(fw);
    let y = y.min(fh);
    let w = w.min(fw - x);
    let h = h.min(fh - y);
    let mut out = Vec::with_capacity((w * h * 4) as usize);
    for row in y..(y + h) {
        let start = ((row * fw + x) * 4) as usize;
        let end = start + (w * 4) as usize;
        if end <= rgba.len() {
            out.extend_from_slice(&rgba[start..end]);
        }
    }
    out
}

/// Scale a crop to `dst_h` height (maintaining aspect), convert to
/// [1, 3, dst_h, dst_w] f32 tensor normalised to `[-1, 1]`.
///
/// Standard PaddleOCR rec preprocessing: height→48, pad width to
/// multiple of 4, values in `[-1, 1]`.
#[cfg(feature = "ocr")]
fn preprocess_crop_for_rec(
    rgba: &[u8],
    src_w: u32,
    src_h: u32,
    dst_h: u32,
) -> ndarray::Array4<f32> {
    let aspect = src_w as f32 / src_h.max(1) as f32;
    let dst_w = ((dst_h as f32 * aspect).round() as u32).max(1);
    let dst_w = ((dst_w + 3) / 4) * 4; // pad to multiple of 4

    let mut out = ndarray::Array4::<f32>::zeros((1, 3, dst_h as usize, dst_w as usize));
    for dy in 0..dst_h as usize {
        for dx in 0..dst_w as usize {
            let sx = (dx as f32 + 0.5) * (src_w as f32 / dst_w as f32) - 0.5;
            let sy = (dy as f32 + 0.5) * (src_h as f32 / dst_h as f32) - 0.5;
            let [r, g, b] = sample_bilinear(rgba, src_w, src_h, sx, sy);
            out[[0, 0, dy, dx]] = r / 127.5 - 1.0;
            out[[0, 1, dy, dx]] = g / 127.5 - 1.0;
            out[[0, 2, dy, dx]] = b / 127.5 - 1.0;
        }
    }
    out
}

/// CTC greedy decode: argmax over class dimension, collapse repeated
/// labels, drop blanks. Returns (text, mean_confidence).
///
/// Input shape: [1, seq_len, num_classes].
/// `keys[0]` is the blank token by convention (see [`models::load_keys`]).
#[cfg(feature = "ocr")]
fn ctc_decode(
    logits: &ndarray::ArrayViewD<f32>,
    keys: &[String],
) -> (String, f32) {
    let shape = logits.shape();
    if shape.len() < 3 || shape[1] == 0 {
        return (String::new(), 0.0);
    }
    let seq_len = shape[1];
    let num_classes = shape[2];

    let mut chars: Vec<char> = Vec::new();
    let mut conf_sum = 0.0f32;
    let mut prev_label = 0usize; // start with blank

    for t in 0..seq_len {
        let mut best_idx = 0usize;
        let mut best_val = f32::NEG_INFINITY;
        for c in 0..num_classes {
            let v = logits[[0, t, c]];
            if v > best_val {
                best_val = v;
                best_idx = c;
            }
        }
        // Softmax-normalise to get actual probability.
        let max_v = best_val;
        let mut sum = 0.0f32;
        for c in 0..num_classes {
            sum += (logits[[0, t, c]] - max_v).exp();
        }
        let prob = 1.0 / sum; // max already on e^0

        // CTC rules: skip blank (index 0), collapse runs.
        if best_idx != 0 && best_idx != prev_label {
            if let Some(key) = keys.get(best_idx) {
                for ch in key.chars() {
                    chars.push(ch);
                }
                conf_sum += prob;
            }
        }
        prev_label = best_idx;
    }

    let n = chars.len();
    if n == 0 {
        return (String::new(), 0.0);
    }
    let text: String = chars.into_iter().collect();
    let confidence = (conf_sum / n as f32).clamp(0.0, 1.0);
    (text, confidence)
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// Verify that without the `ocr` feature the public API returns the
    /// correct error variant rather than panicking.
    #[test]
    fn run_ocr_without_feature_returns_disabled() {
        #[cfg(not(feature = "ocr"))]
        {
            let rgba = vec![0u8; 4 * 4 * 4];
            let tmp = std::path::PathBuf::from("/tmp/no-models");
            match run_ocr(&rgba, 4, 4, &tmp) {
                Err(OcrError::FeatureDisabled) => {}
                other => panic!("expected FeatureDisabled, got {other:?}"),
            }
        }
        #[cfg(feature = "ocr")]
        {
            // With the feature enabled the call will try to load ONNX models
            // which won't be present in CI. Accept either FeatureDisabled
            // (impossible) or ModelsMissing / Runtime (expected).
            let rgba = vec![0u8; 4 * 4 * 4];
            let tmp = std::path::PathBuf::from("/tmp/no-models-ocr-test");
            // Just ensure it doesn't panic.
            let _ = run_ocr(&rgba, 4, 4, &tmp);
        }
    }

    /// Mock-based test: exercises the OcrBox + OcrResult types without
    /// needing model files.
    #[test]
    fn mock_ocr_result_roundtrips() {
        let result = run_ocr_mock(|| OcrResult {
            boxes: vec![OcrBox {
                text: "NEW BALANCE".to_string(),
                x: 100,
                y: 200,
                w: 300,
                h: 50,
                confidence: 0.91,
            }],
            backend: "test-mock",
        });
        assert_eq!(result.boxes.len(), 1);
        assert_eq!(result.boxes[0].text, "NEW BALANCE");
        assert!((result.boxes[0].confidence - 0.91).abs() < 1e-5);
    }

    #[cfg(feature = "ocr")]
    #[test]
    fn scale_for_det_rounds_to_32() {
        let (w, h) = scale_for_det(1920, 1080, 960);
        assert_eq!(w % 32, 0, "w={w} not multiple of 32");
        assert_eq!(h % 32, 0, "h={h} not multiple of 32");
        // Longer edge should be <= max_edge.
        assert!(w.max(h) <= 960, "longer edge exceeds max");
    }

    #[cfg(feature = "ocr")]
    #[test]
    fn extract_bboxes_empty_map_returns_empty() {
        use ndarray::Array4;
        let map = Array4::<f32>::zeros((1, 1, 8, 8));
        let boxes = extract_bboxes(map.view().into_dyn(), 0.30);
        assert!(boxes.is_empty());
    }

    #[cfg(feature = "ocr")]
    #[test]
    fn extract_bboxes_detects_block() {
        use ndarray::Array4;
        // Paint a 4×4 block of high-probability at (2,2).
        let mut map = Array4::<f32>::zeros((1, 1, 16, 16));
        for y in 2..6 {
            for x in 2..6 {
                map[[0, 0, y, x]] = 0.9;
            }
        }
        let boxes = extract_bboxes(map.view().into_dyn(), 0.30);
        assert!(!boxes.is_empty(), "expected at least one bbox");
    }

    #[cfg(feature = "ocr")]
    #[test]
    fn ctc_decode_collapses_runs_and_drops_blank() {
        // Build a minimal logits tensor: [1, 5, 3] with keys=[blank,A,B].
        // Sequence: A A blank B B → should decode to "AB".
        use ndarray::Array3;
        let keys = vec!["blank".to_string(), "A".to_string(), "B".to_string()];
        // logits shape [1, 5, 3]
        let mut logits = Array3::<f32>::zeros((1, 5, 3));
        // t=0,1: argmax = 1 (A)
        logits[[0, 0, 1]] = 10.0;
        logits[[0, 1, 1]] = 10.0;
        // t=2: argmax = 0 (blank)
        logits[[0, 2, 0]] = 10.0;
        // t=3,4: argmax = 2 (B)
        logits[[0, 3, 2]] = 10.0;
        logits[[0, 4, 2]] = 10.0;
        let (text, _conf) = ctc_decode(&logits.view().into_dyn(), &keys);
        assert_eq!(text, "AB", "CTC should collapse runs and drop blanks");
    }
}
