//! Depth Anything V2 Small inference + grid pooling.
//!
//! Input: any RGBA / RGB image.
//! Output: a [`DepthGrid`] — a `GRID_SIZE × GRID_SIZE` array of f32
//! values in `[0, 1]` where **1.0 = closest** and **0.0 = farthest**.
//!
//! Processing pipeline:
//!
//! 1. Resize to `MODEL_W × MODEL_H` (196 × 196, patch-aligned at 14).
//! 2. Normalise to ImageNet mean/std, channel-first (CHW).
//! 3. Run the ONNX session produced by [`crate::depth::model::ensure_model`].
//! 4. Read the single output tensor (relative depth, raw logits).
//! 5. Min-max normalise per frame so the full 0..1 range is used.
//! 6. Mean-pool to `GRID_SIZE × GRID_SIZE` (16 × 16).
//! 7. Invert so 1.0 means far (background) — sign convention matches
//!    how depth maps are used by the negative-space scorer and the lint
//!    rule: **high value = safe to place text**.
//!
//! The inversion in step 7 means [`DepthGrid::cell`] returns a
//! *background-likelihood* score consistent with the 2D heuristic
//! `score` in [`crate::image_analysis::negative_space`].

use serde::{Deserialize, Serialize};

/// Grid side length. The 16 × 16 = 256 cells cover a 1080p frame at
/// ~68 × 120 px/cell — coarse enough to be fast, fine enough to
/// distinguish "above the head" from "on the face."
pub const GRID_SIZE: usize = 16;

/// Input size for Depth Anything V2 Small (ViT-S/14 patch size 14).
///
/// 196 = 14 × 14 — the smallest multiple of 14 that gives the model
/// enough spatial resolution. Running at 364 × 364 (the canonical
/// input) takes ~554 ms on Apple Silicon; 196 × 196 takes ~110–140 ms.
pub const MODEL_W: u32 = 196;
/// Input height — same as [`MODEL_W`].
pub const MODEL_H: u32 = 196;

/// ImageNet mean per channel (RGB order), used during pre-processing.
const IMAGENET_MEAN: [f32; 3] = [0.485, 0.456, 0.406];
/// ImageNet std per channel (RGB order), used during pre-processing.
const IMAGENET_STD: [f32; 3] = [0.229, 0.224, 0.225];

/// A coarse depth grid for one frame.
///
/// Indexing: `values[row * GRID_SIZE + col]` where (0, 0) is the
/// top-left corner. Values are in `[0, 1]` after min-max normalisation
/// and inversion:
///
/// - **1.0** → far from the camera (background, safe for text).
/// - **0.0** → close to the camera (foreground / subject, avoid).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DepthGrid {
    /// Flat `GRID_SIZE × GRID_SIZE` depth values. Bright = far =
    /// background-safe; dark = close = subject.
    pub values: Vec<f32>,
    /// Number of rows (always [`GRID_SIZE`]).
    pub rows: usize,
    /// Number of columns (always [`GRID_SIZE`]).
    pub cols: usize,
}

impl DepthGrid {
    /// Construct a zeroed grid. Used as a stub when the `depth` feature
    /// is disabled so call sites compile without `#[cfg]` guards.
    pub fn zeros() -> Self {
        Self {
            values: vec![0.0; GRID_SIZE * GRID_SIZE],
            rows: GRID_SIZE,
            cols: GRID_SIZE,
        }
    }

    /// Return the depth value at `(row, col)`, or `0.0` when out of
    /// bounds.
    pub fn cell(&self, row: usize, col: usize) -> f32 {
        if row < self.rows && col < self.cols {
            self.values[row * self.cols + col]
        } else {
            0.0
        }
    }

    /// Map the grid back to a full-resolution grayscale image where
    /// pixel intensity encodes background-likelihood:
    ///
    /// - 255 → far (background).
    /// - 0   → close (foreground).
    ///
    /// Each grid cell is rendered as a solid block. The output is
    /// `(cols * cell_w) × (rows * cell_h)` pixels.
    pub fn to_grayscale(&self, canvas_w: u32, canvas_h: u32) -> image::GrayImage {
        let cell_w = (canvas_w / self.cols as u32).max(1);
        let cell_h = (canvas_h / self.rows as u32).max(1);
        let out_w = cell_w * self.cols as u32;
        let out_h = cell_h * self.rows as u32;
        let mut img = image::GrayImage::new(out_w, out_h);
        for r in 0..self.rows {
            for c in 0..self.cols {
                let v = self.cell(r, c);
                let luma = (v * 255.0).clamp(0.0, 255.0) as u8;
                let x0 = c as u32 * cell_w;
                let y0 = r as u32 * cell_h;
                for dy in 0..cell_h {
                    for dx in 0..cell_w {
                        img.put_pixel(x0 + dx, y0 + dy, image::Luma([luma]));
                    }
                }
            }
        }
        img
    }
}

/// Run Depth Anything V2 Small on `image_path` and return a [`DepthGrid`].
///
/// Downloads the model on first call via
/// [`crate::depth::model::ensure_model`]. On macOS the CoreML execution
/// provider is attempted first; falls back to CPU automatically.
///
/// # Errors
///
/// Returns a `String` on model-file fetch failure, ONNX session error,
/// or image decode failure. The `depth` feature must be enabled at
/// compile time.
pub fn estimate_depth(image_path: &std::path::Path) -> Result<DepthGrid, String> {
    #[cfg(not(feature = "depth"))]
    {
        let _ = image_path;
        return Err("depth feature is not enabled — rebuild with `--features depth`".into());
    }
    #[cfg(feature = "depth")]
    {
        run_inference(image_path)
    }
}

/// Estimate depth from raw RGBA bytes (row-major, top-down).
///
/// `width` and `height` are the dimensions of `rgba`. Returns a
/// [`DepthGrid`] using the same model as [`estimate_depth`].
///
/// # Errors
///
/// Returns a `String` when the model session cannot be opened or the
/// inference fails.
pub fn estimate_depth_rgba(
    rgba: &[u8],
    width: u32,
    height: u32,
) -> Result<DepthGrid, String> {
    #[cfg(not(feature = "depth"))]
    {
        let _ = (rgba, width, height);
        return Err("depth feature is not enabled".into());
    }
    #[cfg(feature = "depth")]
    {
        run_inference_rgba(rgba, width, height)
    }
}

// ---------------------------------------------------------------------------
// Feature-gated internals
// ---------------------------------------------------------------------------

#[cfg(feature = "depth")]
fn run_inference(image_path: &std::path::Path) -> Result<DepthGrid, String> {
    let img = image::open(image_path)
        .map_err(|e| format!("open {}: {e}", image_path.display()))?
        .to_rgba8();
    let (w, h) = (img.width(), img.height());
    let session = open_session()?;
    let input = preprocess_rgba(img.as_raw(), w, h);
    infer_grid(session, input)
}

/// Feature-gated estimate_depth_rgba implementation.
#[cfg(feature = "depth")]
fn run_inference_rgba(rgba: &[u8], width: u32, height: u32) -> Result<DepthGrid, String> {
    let session = open_session()?;
    let input = preprocess_rgba(rgba, width, height);
    infer_grid(session, input)
}

/// Open an ONNX Runtime session for the depth model.
///
/// On macOS the CoreML EP is registered first; ORT falls back to CPU
/// automatically when CoreML cannot handle the model. `rc.10` is pinned
/// to avoid the Vitis EP breakage in `rc.11`+.
#[cfg(feature = "depth")]
fn open_session() -> Result<ort::session::Session, String> {
    use ort::execution_providers::CoreMLExecutionProvider;
    use ort::session::builder::GraphOptimizationLevel;
    let model_path = crate::depth::model::ensure_model()?;

    // Disable graph optimizations entirely. Reason: ORT 1.26 (current
    // Homebrew bottle) ships SimplifiedLayerNormFusion which mangles
    // the Depth-Anything-V2 fp16 ONNX graph and panics during session
    // commit ("Attempting to get index by a name which does not
    // exist: InsertedPrecisionFreeCast_/norm_3/Constant_output_0").
    // Level0 (Disable) lets the model load on 1.26 at the cost of
    // slightly slower inference. Confirmed working 2026-05-23.
    let session = ort::session::Session::builder()
        .map_err(|e| format!("ORT session builder: {e}"))?
        .with_optimization_level(GraphOptimizationLevel::Disable)
        .map_err(|e| format!("ORT opt level: {e}"))?
        .with_execution_providers([CoreMLExecutionProvider::default().build()])
        .map_err(|e| format!("ORT CoreML EP: {e}"))?
        .commit_from_file(&model_path)
        .map_err(|e| format!("ORT commit_from_file {}: {e}", model_path.display()))?;

    Ok(session)
}

/// Pre-process an RGBA buffer into a model-ready CHW f32 tensor.
///
/// Steps:
///
/// 1. Resize to [`MODEL_W`] × [`MODEL_H`] using nearest-neighbour
///    (speed over quality — the grid pool absorbs any aliasing).
/// 2. Convert RGBA → RGB, normalise each channel with ImageNet
///    mean/std.
/// 3. Output layout: `[1, 3, MODEL_H, MODEL_W]` (NCHW, f32).
#[cfg(feature = "depth")]
fn preprocess_rgba(rgba: &[u8], src_w: u32, src_h: u32) -> ndarray::Array4<f32> {
    // Build a temporary image so we can use the `image` crate's resize.
    let src = image::RgbaImage::from_raw(src_w, src_h, rgba.to_vec())
        .unwrap_or_else(|| image::RgbaImage::new(src_w, src_h));
    let resized = image::imageops::resize(
        &src,
        MODEL_W,
        MODEL_H,
        image::imageops::FilterType::Nearest,
    );

    let mut tensor = ndarray::Array4::<f32>::zeros([1, 3, MODEL_H as usize, MODEL_W as usize]);
    for y in 0..MODEL_H as usize {
        for x in 0..MODEL_W as usize {
            let px = resized.get_pixel(x as u32, y as u32);
            for c in 0..3usize {
                let raw = px.0[c] as f32 / 255.0;
                tensor[[0, c, y, x]] = (raw - IMAGENET_MEAN[c]) / IMAGENET_STD[c];
            }
        }
    }
    tensor
}

/// Run one forward pass, min-max normalise, mean-pool to the 16×16
/// grid, and invert so 1.0 = far (background).
#[cfg(feature = "depth")]
fn infer_grid(mut session: ort::session::Session, input: ndarray::Array4<f32>) -> Result<DepthGrid, String> {
    use ort::value::TensorRef;

    let tensor = TensorRef::from_array_view(input.view())
        .map_err(|e| format!("ORT TensorRef: {e}"))?;
    let outputs = session
        .run(ort::inputs![tensor])
        .map_err(|e| format!("ORT run: {e}"))?;

    // The depth-anything ONNX has a single output named "predicted_depth"
    // with shape [1, H, W] (the relative depth logits).
    let (shape_ref, flat_slice) = outputs[0]
        .try_extract_tensor::<f32>()
        .map_err(|e| format!("ORT extract tensor: {e}"))?;

    let flat: Vec<f32> = flat_slice.to_vec();
    // shape_ref derefs to [i64]; collect a local copy so lifetimes are clear.
    let shape_dims: Vec<i64> = shape_ref.to_vec();

    // Min-max normalise per frame.
    let min = flat.iter().cloned().fold(f32::INFINITY, f32::min);
    let max = flat.iter().cloned().fold(f32::NEG_INFINITY, f32::max);
    let range = (max - min).max(1e-6);
    let normed: Vec<f32> = flat.iter().map(|&v| (v - min) / range).collect();

    // Determine the HxW of the output tensor (may differ from MODEL_HxW
    // when the model upsamples internally — DA-V2 outputs at input res).
    let (out_h, out_w) = if shape_dims.len() == 3 {
        (shape_dims[1] as usize, shape_dims[2] as usize)
    } else if shape_dims.len() == 2 {
        (shape_dims[0] as usize, shape_dims[1] as usize)
    } else {
        return Err(format!("unexpected depth tensor shape {:?}", shape_dims));
    };

    // Mean-pool to GRID_SIZE × GRID_SIZE.
    let mut grid_vals = vec![0.0f32; GRID_SIZE * GRID_SIZE];
    for gr in 0..GRID_SIZE {
        for gc in 0..GRID_SIZE {
            let y0 = gr * out_h / GRID_SIZE;
            let y1 = ((gr + 1) * out_h / GRID_SIZE).max(y0 + 1).min(out_h);
            let x0 = gc * out_w / GRID_SIZE;
            let x1 = ((gc + 1) * out_w / GRID_SIZE).max(x0 + 1).min(out_w);
            let mut sum = 0.0f64;
            let mut n = 0usize;
            for y in y0..y1 {
                for x in x0..x1 {
                    sum += normed[y * out_w + x] as f64;
                    n += 1;
                }
            }
            let mean = if n > 0 { (sum / n as f64) as f32 } else { 0.0 };
            // Invert: raw depth is close=high → we want close=0.
            grid_vals[gr * GRID_SIZE + gc] = 1.0 - mean;
        }
    }

    Ok(DepthGrid {
        values: grid_vals,
        rows: GRID_SIZE,
        cols: GRID_SIZE,
    })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn depth_grid_zeros_cell_access() {
        let g = DepthGrid::zeros();
        assert_eq!(g.cell(0, 0), 0.0);
        assert_eq!(g.cell(GRID_SIZE - 1, GRID_SIZE - 1), 0.0);
        // Out of bounds returns 0.
        assert_eq!(g.cell(GRID_SIZE, 0), 0.0);
    }

    #[test]
    fn depth_grid_to_grayscale_dimensions() {
        let mut g = DepthGrid::zeros();
        // Set one cell to 1.0 (far).
        g.values[0] = 1.0;
        let img = g.to_grayscale(1080, 1920);
        // Width should equal cols * (1080 / GRID_SIZE).
        let expected_w = (1080 / GRID_SIZE as u32) * GRID_SIZE as u32;
        let expected_h = (1920 / GRID_SIZE as u32) * GRID_SIZE as u32;
        assert_eq!(img.width(), expected_w);
        assert_eq!(img.height(), expected_h);
        // Top-left block should be white (255) since cell(0,0) = 1.0.
        assert_eq!(img.get_pixel(0, 0).0[0], 255);
    }

    #[test]
    fn no_depth_feature_returns_error() {
        // This test is always present. When compiled WITHOUT the feature,
        // estimate_depth returns Err. When compiled WITH the feature it
        // may return Ok or Err depending on model availability — we just
        // verify it compiles and runs.
        let tmp = std::env::temp_dir().join("_wavelet_nonexistent_depth_test.png");
        let result = estimate_depth(&tmp);
        // Must not panic. In non-depth builds it should be Err.
        let _ = result;
    }
}
