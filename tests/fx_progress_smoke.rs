//! Smoke test for wavelet_fx-driven scene transitions — the path that
//! powers `data-transition-in="crossfade"` / `wipe` / `dip-to-black`.
//!
//! Before the fix for wb-e8jh.2 every classified transition panicked
//! at runtime with `wavelet_fx parse: unknown identifier 'progress'`. This
//! exercise builds a 2-scene comp with an inline wavelet_fx crossfade that
//! references `progress` as a bare identifier (the form agents
//! actually write) and asserts the render completes cleanly.

use wavelet::render_offline::{render_composition, Composition, SceneSpec, TransitionSpec};
use std::path::PathBuf;

#[test]
fn two_scene_crossfade_with_bare_progress_identifier_renders() {
    let tmp = std::env::temp_dir().join("wavelet-wavelet_fx-progress-smoke");
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();

    // Scene A: solid red. Scene B: solid blue. Mid-transition the
    // crossfade should paint magenta-ish pixels — a clean exit is the
    // gate for this test; pixel verification is a follow-on.
    let scene_a = tmp.join("a.html");
    std::fs::write(
        &scene_a,
        r#"<!doctype html><html><body style="margin:0;background:#ff0000;width:100vw;height:100vh"></body></html>"#,
    )
    .unwrap();
    let scene_b = tmp.join("b.html");
    std::fs::write(
        &scene_b,
        r#"<!doctype html><html><body style="margin:0;background:#0000ff;width:100vw;height:100vh"></body></html>"#,
    )
    .unwrap();

    // Use the bare-`progress` form on purpose — that's the regression
    // surface. Equivalent to `prop("progress")` post-fix.
    let wavelet_fx = "src(0).blend(src(1), progress).out()".to_string();

    let fps = 30;
    let scene_a_frames = 15; // 0.5s
    let scene_b_frames = 30; // 1.0s
    let transition_secs = 0.5;
    let total_frames = scene_a_frames + scene_b_frames;

    let comp = Composition {
        width: 160,
        height: 120,
        fps,
        duration_frames: total_frames,
        aspect: None,
        scenes: vec![
            SceneSpec {
                html_path: PathBuf::from("a.html"),
                start_frame: 0,
                duration_frames: scene_a_frames,
                transition_in: None,
                video_bg: None,
            },
            SceneSpec {
                html_path: PathBuf::from("b.html"),
                start_frame: scene_a_frames,
                duration_frames: scene_b_frames,
                transition_in: Some(TransitionSpec {
                    wavelet_fx,
                    duration_secs: transition_secs,
                }),
                video_bg: None,
            },
        ],
        audio_cues: vec![],
    };

    let out = tmp.join("out.mp4");
    let stats = render_composition(&comp, &tmp, &out).expect("render must succeed");
    assert_eq!(stats.video_frames, total_frames as u64);
    assert!(stats.mp4_bytes > 0, "output mp4 must be non-empty");
    assert!(out.exists(), "output mp4 must exist on disk");
}
