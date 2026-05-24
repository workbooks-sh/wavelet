//! `text-on-subject` lint rule.
//!
//! Flags HTML text overlays that are positioned over the foreground
//! subject in the underlying video. The check uses the depth-map grid
//! produced by Depth Anything V2 Small to determine whether the mean
//! depth under a text element's bounding box is in the FOREGROUND
//! (bright = close = subject) region.
//!
//! # Activation
//!
//! The rule is **opt-in** and never runs as part of the default rule
//! set. Enable it explicitly:
//!
//! ```text
//! wavelet lint scene.html --rules text-on-subject
//! ```
//!
//! Running it requires:
//!
//! 1. The `depth` Cargo feature compiled in.
//! 2. The Depth Anything V2 Small model (~25 MB, fetched automatically
//!    on first use to `~/.wavelet/models/depth/`).
//! 3. A rendered MP4 passed via `--mp4` so individual frames can be
//!    sampled. Without the MP4 the rule falls back to the HTML-rendered
//!    scene, which lacks the actual video underlay.
//!
//! # Threshold
//!
//! A text element is flagged when the mean depth-grid value under its
//! bounding box exceeds [`FOREGROUND_THRESHOLD`] (default 0.6). The
//! grid encodes **0 = far (background)** and **1 = close (foreground)**
//! — so values above 0.6 indicate the text region overlaps with the
//! subject.
//!
//! # Findings
//!
//! Severity is `Warn` by default. The message includes the mean depth
//! value and the text element selector so the author can reposition the
//! overlay or add a scrim.

use super::mp4_frames;
use super::report::{LintFinding, Severity};
use super::text_readability::is_text_candidate;
use crate::depth::depth_anything::estimate_depth_rgba;
use crate::lint::text_readability_contrast::RenderedFrame;
use crate::query::{FrameSnapshot, Rect};
use std::path::{Path, PathBuf};

/// Rule identifier — used in `--rules` filters and in finding output.
pub const RULE: &str = "text-on-subject";

/// Depth threshold above which a cell is considered foreground.
/// Range `[0, 1]` where 1 = closest to the camera. The 0.6 floor was
/// calibrated so that typical "arm's-length human subject" frames
/// produce foreground readings above 0.65, while flat backgrounds (sky,
/// wall) stay below 0.35.
pub const FOREGROUND_THRESHOLD: f32 = 0.6;

/// Run the `text-on-subject` rule against one snapshot.
///
/// `mp4_path` is optional. When given, a frame is extracted at
/// `snap.t_secs` via ffmpeg and the depth model runs on the real video
/// pixels. When absent, the rule falls back to sampling the
/// HTML-rendered scene, which is less accurate because the placeholder
/// video element is opaque.
///
/// Returns one [`LintFinding`] per text element whose mean depth is
/// above [`FOREGROUND_THRESHOLD`].
pub fn run(
    snap: &FrameSnapshot,
    scene_path: &Path,
    mp4_path: Option<&Path>,
) -> Vec<LintFinding> {
    let (canvas_w, canvas_h) = snap.viewport;
    if canvas_w == 0 || canvas_h == 0 {
        return Vec::new();
    }

    // Acquire the frame to run depth estimation on.
    let frame: RenderedFrame = match mp4_path {
        Some(mp4) => {
            match mp4_frames::sample_frame_rgba(mp4, snap.t_secs, canvas_w, canvas_h) {
                Some(f) => f,
                None => {
                    eprintln!(
                        "wavelet lint text-on-subject: ffmpeg sample failed at t={:.2}s — \
                         rule skipped for scene {}",
                        snap.t_secs,
                        scene_path.display()
                    );
                    return Vec::new();
                }
            }
        }
        None => {
            // No MP4 provided — render the HTML scene for the frame.
            match render_scene_frame(scene_path, snap.t_secs, canvas_w, canvas_h) {
                Some(f) => f,
                None => return Vec::new(),
            }
        }
    };

    // Compute the depth grid for this frame.
    let depth_grid = match estimate_depth_rgba(&frame.rgba, frame.width, frame.height) {
        Ok(g) => g,
        Err(e) => {
            eprintln!(
                "wavelet lint text-on-subject: depth model failed ({e}) — rule skipped"
            );
            return Vec::new();
        }
    };

    let mut findings = Vec::new();
    let mut seen: Vec<usize> = Vec::new();

    for (idx, node) in snap.nodes.iter().enumerate() {
        if !is_text_candidate(node) {
            continue;
        }
        if !node.bbox.has_area() {
            continue;
        }
        if node.computed_opacity <= 0.0 {
            continue;
        }
        if seen.contains(&node.id) {
            continue;
        }
        seen.push(node.id);

        // Map the element's bounding box into depth-grid cells and
        // compute the mean depth value under the element.
        let mean_depth = mean_depth_under_bbox(
            node.bbox,
            canvas_w,
            canvas_h,
            &depth_grid,
        );

        if mean_depth < FOREGROUND_THRESHOLD {
            // Depth indicates background — text placement is fine.
            continue;
        }

        let selector = best_text_selector(snap, node, idx);
        findings.push(LintFinding {
            rule: RULE.to_string(),
            severity: Severity::Warn,
            scene_path: scene_path.to_path_buf(),
            t_secs: snap.t_secs,
            element_selector: selector,
            element_bbox: node.bbox,
            message: format!(
                "text overlay lands on the foreground subject (mean depth {:.2} > \
                 threshold {:.2}). Move the overlay into the negative space \
                 (above the head, against the sky, on a flat wall) so the \
                 text does not obscure the subject.",
                mean_depth, FOREGROUND_THRESHOLD,
            ),
            fix_hint: "Use `wavelet image negative-space --use-depth` to find \
                       a clean background region, then position the text overlay \
                       in one of the top-ranked cells."
                .to_string(),
            subkind: None,
        });
    }

    findings
}

/// Sample the depth grid over the fraction of the canvas covered by
/// `bbox`. Returns the mean of all grid cells whose centres fall inside
/// the bbox region.
fn mean_depth_under_bbox(
    bbox: Rect,
    canvas_w: u32,
    canvas_h: u32,
    grid: &crate::depth::depth_anything::DepthGrid,
) -> f32 {
    use crate::depth::depth_anything::GRID_SIZE;

    if canvas_w == 0 || canvas_h == 0 || grid.rows == 0 || grid.cols == 0 {
        return 0.0;
    }

    // Convert bbox to fractional canvas coordinates.
    let fx0 = (bbox.x / canvas_w as f32).clamp(0.0, 1.0);
    let fy0 = (bbox.y / canvas_h as f32).clamp(0.0, 1.0);
    let fx1 = ((bbox.x + bbox.w) / canvas_w as f32).clamp(0.0, 1.0);
    let fy1 = ((bbox.y + bbox.h) / canvas_h as f32).clamp(0.0, 1.0);

    if fx1 <= fx0 || fy1 <= fy0 {
        return 0.0;
    }

    // Grid cell ranges covered by the bbox.
    let c0 = (fx0 * GRID_SIZE as f32).floor() as usize;
    let c1 = (fx1 * GRID_SIZE as f32).ceil() as usize;
    let r0 = (fy0 * GRID_SIZE as f32).floor() as usize;
    let r1 = (fy1 * GRID_SIZE as f32).ceil() as usize;

    let c0 = c0.min(GRID_SIZE - 1);
    let c1 = c1.min(GRID_SIZE);
    let r0 = r0.min(GRID_SIZE - 1);
    let r1 = r1.min(GRID_SIZE);

    if c1 <= c0 || r1 <= r0 {
        return 0.0;
    }

    let mut sum = 0.0f64;
    let mut n = 0usize;
    for r in r0..r1 {
        for c in c0..c1 {
            // Note: DepthGrid.cell() returns background likelihood
            // (1=far, 0=close). For this rule we need the INVERSE
            // (foreground likelihood) to compare against the threshold.
            // So: foreground_likelihood = 1.0 - cell(r, c).
            sum += (1.0 - grid.cell(r, c)) as f64;
            n += 1;
        }
    }
    if n == 0 {
        return 0.0;
    }
    (sum / n as f64) as f32
}

/// Build the best CSS selector string for a text node. Mirrors the
/// logic in `text_readability::best_selector` but is local to this
/// module to avoid the cross-module coupling.
fn best_text_selector(
    snap: &FrameSnapshot,
    node: &crate::query::NodeSnapshot,
    _idx: usize,
) -> String {
    if let Some(id) = &node.element_id {
        return format!("#{id}");
    }
    if let Some(first_class) = node.classes.first() {
        return format!(".{first_class}");
    }
    // Parent context: walk up one level for a more unique path.
    if let Some(parent_id) = node.parent {
        if let Some(parent) = snap.nodes.iter().find(|n| n.id == parent_id) {
            if let Some(pid) = &parent.element_id {
                return format!("#{pid} > {}", node.tag);
            }
            if let Some(pc) = parent.classes.first() {
                return format!(".{pc} > {}", node.tag);
            }
        }
    }
    node.tag.clone()
}

/// Render the HTML scene to an RGBA frame at `t_secs`.
fn render_scene_frame(
    scene_path: &Path,
    t_secs: f32,
    width: u32,
    height: u32,
) -> Option<RenderedFrame> {
    use crate::render::{load_html_with_base, Renderer};
    let html = std::fs::read_to_string(scene_path).ok()?;
    let absolute = std::fs::canonicalize(scene_path)
        .unwrap_or_else(|_| scene_path.to_path_buf());
    let base_url = url::Url::from_file_path(&absolute)
        .ok()
        .map(|u| u.to_string());
    let mut doc = load_html_with_base(&html, width, height, base_url);
    doc.as_mut().resolve(t_secs as f64);
    let mut renderer = Renderer::new(width, height);
    let rgba = renderer.render(doc.as_mut());
    Some(RenderedFrame { width, height, rgba })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::depth::depth_anything::{DepthGrid, GRID_SIZE};
    use crate::query::Rect;

    /// Build a [`DepthGrid`] where every cell has the same background
    /// likelihood value.
    fn uniform_grid(bg_likelihood: f32) -> DepthGrid {
        DepthGrid {
            values: vec![bg_likelihood; GRID_SIZE * GRID_SIZE],
            rows: GRID_SIZE,
            cols: GRID_SIZE,
        }
    }

    #[test]
    fn foreground_grid_returns_high_depth() {
        // All cells bg_likelihood = 0.0 → foreground likelihood = 1.0.
        let grid = uniform_grid(0.0);
        let bbox = Rect { x: 0.0, y: 0.0, w: 1080.0, h: 1920.0 };
        let mean = mean_depth_under_bbox(bbox, 1080, 1920, &grid);
        assert!(
            (mean - 1.0).abs() < 0.01,
            "full-canvas foreground grid should return ≈1.0, got {mean}"
        );
    }

    #[test]
    fn background_grid_returns_low_depth() {
        // All cells bg_likelihood = 1.0 → foreground likelihood = 0.0.
        let grid = uniform_grid(1.0);
        let bbox = Rect { x: 0.0, y: 0.0, w: 1080.0, h: 1920.0 };
        let mean = mean_depth_under_bbox(bbox, 1080, 1920, &grid);
        assert!(
            mean < 0.01,
            "full-canvas background grid should return ≈0.0, got {mean}"
        );
    }

    #[test]
    fn zero_size_bbox_returns_zero() {
        let grid = uniform_grid(0.5);
        let bbox = Rect { x: 100.0, y: 100.0, w: 0.0, h: 0.0 };
        let mean = mean_depth_under_bbox(bbox, 1080, 1920, &grid);
        assert_eq!(mean, 0.0);
    }

    #[test]
    fn offcanvas_bbox_clamped() {
        let grid = uniform_grid(0.0);
        let bbox = Rect { x: -500.0, y: -500.0, w: 200.0, h: 200.0 };
        // Bbox is completely off-canvas — should return 0.0.
        let mean = mean_depth_under_bbox(bbox, 1080, 1920, &grid);
        assert_eq!(mean, 0.0);
    }

    #[test]
    fn depth_threshold_constant_in_range() {
        assert!(
            FOREGROUND_THRESHOLD > 0.0 && FOREGROUND_THRESHOLD < 1.0,
            "threshold must be in (0, 1)"
        );
    }
}
