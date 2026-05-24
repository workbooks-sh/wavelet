//! `render_offline::utils` — extracted from godfile split.

#![allow(missing_docs)]

use std::path::{Path, PathBuf};
use std::collections::HashSet;
use super::types::{Composition, AudioCueSpec};

pub(super) fn active_scene(comp: &Composition, frame: u32) -> Option<usize> {
    comp.scenes.iter().position(|s| {
        frame >= s.start_frame && frame < s.start_frame + s.duration_frames
    })
}

/// Walk every asset referenced by `comp` and return the subset that
/// doesn't exist on disk. Resolved paths are returned absolute so the
/// caller can print them to the agent verbatim.
///
/// Checked paths:
/// - `scene.html_path` for each scene
/// - `scene.video_bg` if Some
/// - `<video src>` inside each scene HTML (resolved relative to the
///   scene's own directory, matching browser semantics)
///
/// Audio refs are NOT pre-flighted here — a broken `<audio src>` is
/// caught downstream by the renderer (warn + drop the cue + continue
/// video-only) and surfaced to the agent by the `audio-presence`
/// lint. Pre-flight-aborting on audio would block the legitimate
/// "render video now, fix the broken music ref later" workflow.
///
/// Scene HTML files that fail to read are themselves reported as
/// missing (they were referenced but unreadable). Inline `src` attributes
/// pointing at non-file URLs (http://, data:, file://) are skipped —
/// only path-shaped refs are validated.
pub(super) fn collect_missing_assets(comp: &Composition, root_dir: &Path) -> Vec<PathBuf> {
    fn check(missing: &mut Vec<PathBuf>, abs: PathBuf) {
        if !abs.exists() {
            missing.push(abs);
        }
    }

    let mut missing: Vec<PathBuf> = Vec::new();

    for scene in &comp.scenes {
        let scene_abs = root_dir.join(&scene.html_path);
        if !scene_abs.exists() {
            missing.push(scene_abs);
            continue;
        }

        if let Some(bg) = scene.video_bg.as_ref() {
            check(&mut missing, root_dir.join(bg));
        }

        let scene_dir = scene_abs.parent().map(Path::to_path_buf).unwrap_or_else(|| root_dir.to_path_buf());
        let scene_html = match std::fs::read_to_string(&scene_abs) {
            Ok(s) => s,
            Err(_) => continue,
        };
        let elements = match crate::compose::collect_elements(&scene_html) {
            Ok(els) => els,
            Err(_) => continue,
        };
        for el in &elements {
            use crate::compose::ElementKind;
            if !matches!(el.kind, ElementKind::Video) {
                continue;
            }
            let Some(src) = el.attr("src") else { continue };
            if src.starts_with("http://")
                || src.starts_with("https://")
                || src.starts_with("data:")
                || src.starts_with("file://")
            {
                continue;
            }
            check(&mut missing, scene_dir.join(src));
        }
    }

    missing
}

/// Write a stereo f32 buffer to a 16-bit PCM WAV file. Pure-Rust — no
/// `hound` dep needed for this much.
pub(super) fn write_stereo_wav(path: &Path, samples: &[f32], sample_rate: u32) -> std::io::Result<()> {
    use std::io::Write;
    let mut f = std::fs::File::create(path)?;
    let num_samples = samples.len() as u32;
    let byte_rate = sample_rate * 2 * 16 / 8;
    let block_align = 2u16 * 16 / 8;
    let data_size = num_samples * 2;
    let chunk_size = 36 + data_size;

    f.write_all(b"RIFF")?;
    f.write_all(&chunk_size.to_le_bytes())?;
    f.write_all(b"WAVE")?;
    // fmt chunk
    f.write_all(b"fmt ")?;
    f.write_all(&16u32.to_le_bytes())?;
    f.write_all(&1u16.to_le_bytes())?; // PCM
    f.write_all(&2u16.to_le_bytes())?; // 2 channels
    f.write_all(&sample_rate.to_le_bytes())?;
    f.write_all(&byte_rate.to_le_bytes())?;
    f.write_all(&block_align.to_le_bytes())?;
    f.write_all(&16u16.to_le_bytes())?; // bits per sample
    // data chunk
    f.write_all(b"data")?;
    f.write_all(&data_size.to_le_bytes())?;
    for &s in samples {
        let clamped = s.clamp(-1.0, 1.0);
        let int = (clamped * i16::MAX as f32) as i16;
        f.write_all(&int.to_le_bytes())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::types::*;
    use super::super::render::{render_composition, render_composition_with_options};
    use super::super::stats::*;
    use super::*;

    #[test]
    fn active_scene_picks_right_index() {
        let comp = Composition {
            width: 64, height: 64, fps: 30, duration_frames: 30,
            scenes: vec![
                SceneSpec {
                    html_path: PathBuf::from("a.html"),
                    start_frame: 0, duration_frames: 10, transition_in: None, video_bg: None,
                },
                SceneSpec {
                    html_path: PathBuf::from("b.html"),
                    start_frame: 10, duration_frames: 10, transition_in: None, video_bg: None,
                },
            ],
            aspect: None,
            audio_cues: vec![],
        };
        assert_eq!(active_scene(&comp, 0), Some(0));
        assert_eq!(active_scene(&comp, 9), Some(0));
        assert_eq!(active_scene(&comp, 10), Some(1));
        assert_eq!(active_scene(&comp, 19), Some(1));
        assert_eq!(active_scene(&comp, 20), None);
    }

    #[test]
    fn scene_overflow_detected() {
        let comp = Composition {
            width: 64, height: 64, fps: 30, duration_frames: 10,
            scenes: vec![SceneSpec {
                html_path: PathBuf::from("a.html"),
                start_frame: 5, duration_frames: 10, transition_in: None, video_bg: None,
            }],
            aspect: None,
            audio_cues: vec![],
        };
        let err = render_composition(
            &comp,
            Path::new("."),
            Path::new("/tmp/wavelet-overflow.mp4"),
        )
        .unwrap_err();
        assert!(matches!(err, RenderOfflineError::SceneOverflow(_, 15, 10)));
    }

    #[test]
    fn renders_two_scenes_back_to_back() {
        // Write two scene HTML files.
        let tmp = std::env::temp_dir().join("wavelet-orchestrator-test");
        std::fs::create_dir_all(&tmp).unwrap();
        std::fs::write(
            tmp.join("a.html"),
            r#"<!doctype html><html><body style="background:#0a0">A</body></html>"#,
        ).unwrap();
        std::fs::write(
            tmp.join("b.html"),
            r#"<!doctype html><html><body style="background:#00a">B</body></html>"#,
        ).unwrap();

        let comp = Composition {
            width: 64, height: 64, fps: 30, duration_frames: 6,
            scenes: vec![
                SceneSpec {
                    html_path: PathBuf::from("a.html"),
                    start_frame: 0, duration_frames: 3, transition_in: None, video_bg: None,
                },
                SceneSpec {
                    html_path: PathBuf::from("b.html"),
                    start_frame: 3, duration_frames: 3, transition_in: None, video_bg: None,
                },
            ],
            aspect: None,
            audio_cues: vec![],
        };
        let out = tmp.join("out.mp4");
        let stats = render_composition(&comp, &tmp, &out).expect("render");
        assert_eq!(stats.video_frames, 6);
        assert!(stats.mp4_bytes > 100);
        assert_eq!(stats.audio_samples_per_channel, 0);
    }

    #[test]
    fn frame_budget_exceeded_when_budget_is_zero() {
        // Stub regression: with a 0-second budget every frame is over
        // budget by definition (elapsed.as_secs() >= 0), so the first
        // frame trips FrameBudgetExceeded. Real hangs (pathological
        // CSS) are the production trigger; this test pins the
        // structured-error contract that handlers + agent harnesses
        // rely on without needing a known-pathological scene checked
        // into the repo.
        let tmp = std::env::temp_dir().join("wavelet-budget-test");
        std::fs::create_dir_all(&tmp).unwrap();
        std::fs::write(
            tmp.join("a.html"),
            r#"<!doctype html><html><body style="background:#0a0">A</body></html>"#,
        ).unwrap();

        let comp = Composition {
            width: 64, height: 64, fps: 30, duration_frames: 3,
            scenes: vec![SceneSpec {
                html_path: PathBuf::from("a.html"),
                start_frame: 0, duration_frames: 3, transition_in: None, video_bg: None,
            }],
            aspect: None,
            audio_cues: vec![],
        };
        let out = tmp.join("out.mp4");
        let opts = RenderOptions { frame_budget_secs: 0, mux_audio: false };
        let err = render_composition_with_options(&comp, &tmp, &out, &opts)
            .expect_err("render should fail with FrameBudgetExceeded");
        match err {
            RenderOfflineError::FrameBudgetExceeded { frame_index, budget_secs, last_frame_index } => {
                assert_eq!(frame_index, 0);
                assert_eq!(budget_secs, 0);
                assert_eq!(last_frame_index, -1, "no frames should have been pushed before the budget trip");
            }
            other => panic!("expected FrameBudgetExceeded, got {other:?}"),
        }
    }

    #[test]
    fn composition_aspect_round_trips_through_serde() {
        use crate::aspect::AspectRatio;

        let comp = Composition {
            width: 720,
            height: 1280,
            fps: 30,
            duration_frames: 30,
            aspect: Some(AspectRatio::Vertical9x16),
            scenes: vec![],
            audio_cues: vec![],
        };
        let json = serde_json::to_string(&comp).unwrap();
        assert!(json.contains("\"aspect\":\"9:16\""));
        let back: Composition = serde_json::from_str(&json).unwrap();
        assert_eq!(back.aspect, Some(AspectRatio::Vertical9x16));

        // Omitting `aspect` in the JSON is fine — `#[serde(default)]`
        // keeps older comp.json files compatible.
        let legacy = r#"{
            "width": 1280, "height": 720, "fps": 30, "duration_frames": 1,
            "scenes": []
        }"#;
        let comp: Composition = serde_json::from_str(legacy).unwrap();
        assert_eq!(comp.aspect, None);

        // Skipped on serialization when None — no aspect field leaks
        // into freshly-written comp.json files that don't set one.
        let json = serde_json::to_string(&comp).unwrap();
        assert!(!json.contains("aspect"));
    }
}

