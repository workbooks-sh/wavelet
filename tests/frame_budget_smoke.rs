//! wb-5w9s.1.1: per-frame budget enforcement smoke test.
//!
//! Pre-fix: pathological CSS in agent-authored scenes could wedge the
//! Blitz/Stylo/Vello pipeline so render produced no frames for tens of
//! minutes (the 2026-05-21 004-liquid-death eval burned 18+ min on a
//! single frame). The render_offline watchdog now bounds wall-clock
//! per frame; this test locks in the contract.
//!
//! Strategy: drive the public render API against the minimal tier1
//! fixture with `frame_budget_secs = 0`. The watchdog wakes every
//! 500ms; on its first tick (~500ms after frame 0 begins), elapsed
//! will exceed the zero budget and the abort flag fires, returning
//! `FrameBudgetExceeded` before frame 0 completes.
//!
//! With a healthy >0 budget, the same scene renders in well under
//! 100ms — this configuration deliberately starves the renderer to
//! verify the abort path fires structurally.

use wavelet::render_offline::{
    render_composition_with_options, Composition, RenderOfflineError, RenderOptions, SceneSpec,
};
use std::path::PathBuf;

#[test]
fn frame_budget_zero_returns_frame_budget_exceeded() {
    let crate_dir = env!("CARGO_MANIFEST_DIR");
    let root_dir = PathBuf::from(crate_dir).join("tests/fixtures");
    let scene_full = root_dir.join("tier1_scene.html");
    assert!(scene_full.exists(), "fixture not found: {scene_full:?}");

    let tmp = tempfile::tempdir().expect("tmpdir");
    let out_mp4 = tmp.path().join("out.mp4");

    let comp = Composition {
        width: 320,
        height: 240,
        fps: 30,
        duration_frames: 30, // 1 second of video
        aspect: None,
        scenes: vec![SceneSpec {
            html_path: PathBuf::from("tier1_scene.html"),
            start_frame: 0,
            duration_frames: 30,
            transition_in: None,
            video_bg: None,
        }],
        audio_cues: vec![],
    };

    let opts = RenderOptions {
        frame_budget_secs: 0,
        mux_audio: false,
    };

    let result = render_composition_with_options(&comp, &root_dir, &out_mp4, &opts);

    match result {
        Err(RenderOfflineError::FrameBudgetExceeded {
            frame_index,
            budget_secs,
            last_frame_index,
        }) => {
            assert_eq!(budget_secs, 0, "budget_secs should echo the configured value");
            assert!(
                frame_index < 30,
                "frame_index {frame_index} out of comp range"
            );
            assert!(
                last_frame_index < frame_index as i64,
                "last_frame_index {last_frame_index} should be < tripped frame {frame_index}"
            );
        }
        Err(other) => panic!("expected FrameBudgetExceeded, got: {other:?}"),
        Ok(stats) => panic!(
            "expected FrameBudgetExceeded with budget=0, but render completed successfully (stats={stats:?})"
        ),
    }
}
