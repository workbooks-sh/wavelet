//! Structural + behavioral verification of a [`Composition`].
//!
//! Structural checks (fast, no rendering):
//! - Scene HTML files exist + parse.
//! - Audio asset paths exist.
//! - No scene overflows the composition duration.
//!
//! Behavioral checks (with `deep: true`, samples up to 3 frames per scene):
//! - Each scene's mid-frame contains non-background pixels (catches scenes
//!   that animated themselves into invisibility).
//! - Audio cues open successfully via symphonia.

use crate::audio::decoder::DecodedAudio;
use crate::render::{load_html_with_base, render_document_to_rgba};
use crate::render_offline::{Composition, SceneSpec};
use std::path::{Path, PathBuf};

/// One verification finding — a single issue (warning or error) about the
/// composition.
#[derive(Debug, Clone)]
pub struct Finding {
    /// Severity (`error` blocks render; `warning` is advisory).
    pub level: Level,
    /// Where the finding originates — a scene path, asset, or "composition".
    pub origin: String,
    /// Human-readable description.
    pub message: String,
}

/// Finding severity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    /// Will cause render to fail or produce nonsense.
    Error,
    /// Won't fail the render but indicates probable author mistake.
    Warning,
}

/// Run verification. Returns every finding; caller decides what to do with them.
pub fn verify(comp: &Composition, root_dir: &Path, deep: bool) -> Vec<Finding> {
    let mut out: Vec<Finding> = Vec::new();

    // Duration consistency.
    for scene in &comp.scenes {
        let end = scene.start_frame + scene.duration_frames;
        if end > comp.duration_frames {
            out.push(Finding {
                level: Level::Error,
                origin: scene.html_path.to_string_lossy().into_owned(),
                message: format!(
                    "scene ends at frame {end} but composition duration is {}",
                    comp.duration_frames
                ),
            });
        }
    }

    // Per-scene structural checks.
    for (idx, scene) in comp.scenes.iter().enumerate() {
        verify_scene(scene, idx, root_dir, comp.width, comp.height, deep, &mut out);
    }

    // Audio cue checks.
    for (idx, cue) in comp.audio_cues.iter().enumerate() {
        let origin = format!("audio[{}]={}", idx, cue.asset_path.display());
        let resolved = root_dir.join(&cue.asset_path);
        if !resolved.exists() {
            out.push(Finding {
                level: Level::Error,
                origin: origin.clone(),
                message: format!("asset not found: {}", resolved.display()),
            });
            continue;
        }
        if deep {
            if let Err(e) = DecodedAudio::decode(&resolved) {
                out.push(Finding {
                    level: Level::Error,
                    origin,
                    message: format!("symphonia decode probe failed: {e}"),
                });
            }
        }
    }

    out
}

fn verify_scene(
    scene: &SceneSpec,
    idx: usize,
    root_dir: &Path,
    width: u32,
    height: u32,
    deep: bool,
    out: &mut Vec<Finding>,
) {
    let origin = format!("scene[{}]={}", idx, scene.html_path.display());
    let resolved: PathBuf = root_dir.join(&scene.html_path);
    if !resolved.exists() {
        out.push(Finding {
            level: Level::Error,
            origin,
            message: format!("html file not found: {}", resolved.display()),
        });
        return;
    }
    let html = match std::fs::read_to_string(&resolved) {
        Ok(s) => s,
        Err(e) => {
            out.push(Finding {
                level: Level::Error,
                origin,
                message: format!("html read error: {e}"),
            });
            return;
        }
    };

    // Parse + resolve once. Scene HTML's absolute file URL becomes the base
    // so `<img src="../assets/x.jpg">` etc. resolve.
    let absolute = std::fs::canonicalize(&resolved).unwrap_or_else(|_| resolved.clone());
    let base_url = url::Url::from_file_path(&absolute).ok().map(|u| u.to_string());
    let mut doc = load_html_with_base(&html, width, height, base_url);

    if !deep {
        return;
    }

    // Deep: sample mid-frame, check it has non-background pixels.
    let mid_frame = scene.duration_frames / 2;
    let mid_t = mid_frame as f32 / 30.0; // hardcoded fps for the probe; the real render uses comp.fps
    doc.as_mut().resolve(mid_t as f64);
    let pixels = render_document_to_rgba(doc.as_mut(), width, height);
    if pixels_are_uniform(&pixels) {
        out.push(Finding {
            level: Level::Warning,
            origin,
            message: format!(
                "scene mid-frame at t={:.2}s has uniform pixels (likely empty or fully transparent)",
                mid_t
            ),
        });
    }
}

/// True if every pixel in the buffer is identical — a proxy for "nothing
/// rendered." False if any pixel differs from the first.
fn pixels_are_uniform(pixels: &[u8]) -> bool {
    if pixels.len() < 8 {
        return true;
    }
    let first: [u8; 4] = [pixels[0], pixels[1], pixels[2], pixels[3]];
    pixels
        .chunks_exact(4)
        .all(|px| px[0] == first[0] && px[1] == first[1] && px[2] == first[2] && px[3] == first[3])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render_offline::{Composition, SceneSpec};
    use crate::test_utils::BLITZ_GUARD;
    use std::path::PathBuf;

    fn verify_serial(comp: &Composition, root_dir: &Path, deep: bool) -> Vec<Finding> {
        let _g = BLITZ_GUARD.lock().unwrap();
        verify(comp, root_dir, deep)
    }

    fn write_scene(dir: &Path, name: &str, body: &str) {
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(
            dir.join(name),
            format!(
                r#"<!doctype html><html><body style="background:#0a0">{body}</body></html>"#
            ),
        )
        .unwrap();
    }

    #[test]
    fn missing_html_is_error() {
        let comp = Composition {
            width: 64, height: 64, fps: 30, duration_frames: 30,
            scenes: vec![SceneSpec {
                html_path: PathBuf::from("nonexistent.html"),
                start_frame: 0, duration_frames: 30, transition_in: None, video_bg: None,
            }],
            aspect: None,
            audio_cues: vec![],
        };
        let findings = verify_serial(&comp, Path::new("/tmp"), false);
        assert!(findings.iter().any(|f| f.level == Level::Error
            && f.message.contains("not found")));
    }

    #[test]
    fn scene_overflow_is_error() {
        let tmp = std::env::temp_dir().join("wavelet-verify-test-overflow");
        write_scene(&tmp, "a.html", r#"<div>X</div>"#);
        let comp = Composition {
            width: 64, height: 64, fps: 30, duration_frames: 10,
            scenes: vec![SceneSpec {
                html_path: PathBuf::from("a.html"),
                start_frame: 5, duration_frames: 10, transition_in: None, video_bg: None,
            }],
            aspect: None,
            audio_cues: vec![],
        };
        let findings = verify_serial(&comp, &tmp, false);
        assert!(findings.iter().any(|f| f.level == Level::Error
            && f.message.contains("ends at frame")));
    }
}
