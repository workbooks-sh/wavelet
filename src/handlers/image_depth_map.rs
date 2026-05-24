//! Handler for `wavelet image depth-map <input.png>`.
//!
//! When `--out` is given, writes a grayscale PNG to the destination
//! path. The PNG encodes background likelihood: **bright pixels are
//! far from the camera** (background-safe for text) and **dark pixels
//! are close** (subject / foreground). This is the inverse of the
//! conventional depth-map sign so the output composites visually with
//! the negative-space heatmap from `wavelet image negative-space`.
//!
//! When `--out` is omitted, prints the raw 16×16 [`DepthGrid`] as JSON
//! to stdout in the same `{ok, result, exec_ms}` envelope that all
//! other `wavelet image` subcommands use.
//!
//! Requires the `depth` Cargo feature. Returns exit code 2 with an
//! error JSON payload when the feature is not compiled in.

use crate::depth::depth_anything::{estimate_depth, DepthGrid};
use crate::handlers::util::emit_analysis;
use crate::image_analysis::AnalysisError;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

/// Dispatch `wavelet image depth-map`.
///
/// `image` is the path to the source PNG/JPG.
/// `out` is the optional output PNG path.
/// `pretty` controls JSON pretty-printing (only relevant when `out` is
/// `None`).
pub fn handle_image_depth_map(
    image: PathBuf,
    out: Option<PathBuf>,
    pretty: bool,
) -> ExitCode {
    match out {
        Some(out_path) => run_to_png(&image, &out_path),
        None => emit_analysis(pretty, || {
            estimate_depth(&image).map_err(AnalysisError::Decode)
        }),
    }
}

/// Run depth estimation and write the result as a grayscale PNG.
fn run_to_png(image: &Path, out: &Path) -> ExitCode {
    let grid = match estimate_depth(image) {
        Ok(g) => g,
        Err(e) => {
            eprintln!("wavelet image depth-map: {e}");
            return ExitCode::from(2);
        }
    };
    // Use the source image dimensions for the output so the depth PNG
    // lines up with the source for visual comparison.
    let (src_w, src_h) = match image_dims(image) {
        Some(d) => d,
        None => {
            // Fall back to a 16×16 grid-sized PNG.
            (crate::depth::depth_anything::GRID_SIZE as u32,
             crate::depth::depth_anything::GRID_SIZE as u32)
        }
    };
    let gray = grid.to_grayscale(src_w, src_h);
    match gray.save(out) {
        Ok(()) => {
            eprintln!("wavelet image depth-map: wrote {}", out.display());
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("wavelet image depth-map: save {}: {e}", out.display());
            ExitCode::from(2)
        }
    }
}

/// Read the pixel dimensions of an image file without decoding the full
/// payload. Returns `None` on any error so callers can degrade
/// gracefully.
fn image_dims(path: &Path) -> Option<(u32, u32)> {
    use image::GenericImageView;
    let img = image::open(path).ok()?;
    Some(img.dimensions())
}

/// Expose the grid as a serialisable type for the JSON path. The
/// [`DepthGrid`] already derives `Serialize` so `emit_analysis` wraps
/// it directly. This re-export makes the test module and the dispatch
/// site explicit about what they're serialising.
pub use crate::depth::depth_anything::DepthGrid as DepthMapResult;

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn nonexistent_input_returns_failure() {
        let code = handle_image_depth_map(
            PathBuf::from("/nonexistent/input.png"),
            None,
            false,
        );
        // Without the depth feature the error message is different but
        // the code must still be non-success.
        assert_ne!(code, ExitCode::SUCCESS);
    }
}
