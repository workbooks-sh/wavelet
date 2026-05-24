//! `render_offline::render` — extracted from godfile split.

#![allow(missing_docs)]

use crate::audio::{AudioCue, AudioMixer};
use crate::inline_video::{discover_and_seed, update_inline_video_frames, InlineVideo};
use crate::query::diff::decode_rgba_frames;
use crate::render::{load_html_with_base, Renderer};
use crate::video::{Codec, RgbaFrame, VideoEncoder, VideoError};
use blitz_html::HtmlDocument;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use super::types::{Composition, SceneSpec, AudioCueSpec, TransitionSpec, DEFAULT_FRAME_BUDGET_SECS};
use super::stats::{RenderStats, RenderOfflineError, RenderOptions};
use super::scene::{SceneRuntime, build_scene_runtime, sample_video_bg, compose_over, render_scene_frame};
use super::utils::{active_scene, collect_missing_assets, write_stereo_wav};


/// Render `comp` to an MP4 at `out_path` + a sibling WAV at the same stem.
///
/// `root_dir` is the directory all relative paths in the composition resolve
/// against (typically the parent dir of the JSON composition file, or the
/// caller's working directory for in-memory comps).
pub fn render_composition(
    comp: &Composition,
    root_dir: &Path,
    out_path: &Path,
) -> Result<RenderStats, RenderOfflineError> {
    render_composition_with_options(comp, root_dir, out_path, &RenderOptions::default())
}

/// Same as [`render_composition`] but takes a [`RenderOptions`] bundle.
pub fn render_composition_with_options(
    comp: &Composition,
    root_dir: &Path,
    out_path: &Path,
    opts: &RenderOptions,
) -> Result<RenderStats, RenderOfflineError> {
    let render_start = std::time::Instant::now();
    let mut stats = RenderStats::default();

    // Validate scenes fit within the timeline.
    for scene in &comp.scenes {
        let end = scene.start_frame + scene.duration_frames;
        if end > comp.duration_frames {
            return Err(RenderOfflineError::SceneOverflow(
                scene.html_path.to_string_lossy().into_owned(),
                end,
                comp.duration_frames,
            ));
        }
    }

    // Pre-flight: validate every referenced asset exists on disk before
    // we open the encoder. Without this, missing files trigger libavformat
    // warn-and-continue paths that can hang the render pipeline for many
    // minutes (wb-5w9s.1). Fail-fast and let the caller (agent or human)
    // see exactly which paths are missing.
    let missing = collect_missing_assets(comp, root_dir);
    if !missing.is_empty() {
        return Err(RenderOfflineError::MissingAssets(missing));
    }

    // 1. Open video encoder.
    let _ = std::fs::remove_file(out_path);
    let mut encoder = VideoEncoder::open(
        out_path,
        comp.width,
        comp.height,
        comp.fps,
        Codec::H264,
    )?;

    // 2. Cache loaded HtmlDocument per scene-index. Document is owned
    //    per-scene because Blitz mutations are scene-local; we never
    //    cross-tick state between scenes.
    let mut scene_state: HashMap<usize, SceneRuntime> = HashMap::new();

    // Long-lived renderer reused across all frames. Critical for GPU mode —
    // constructing VelloImageRenderer per frame would pay the wgpu device
    // init cost (~100ms) 480 times.
    let mut renderer = Renderer::new(comp.width, comp.height);

    // Per-scene shader-transition pipelines (built lazily). Same shader is
    // expensive to re-build per frame; building once at the transition's
    // first frame and caching by scene index amortizes the cost.
    let mut transitions: HashMap<usize, crate::shader::TransitionPipeline> = HashMap::new();
    // Always attempt to create the wgpu device + queue. Used by:
    //  - Transition shader pipelines (existing).
    //  - Per-element CSS filter apply (wb-5w9s.1.2 phase 2 GPU path).
    // Returns None on headless systems lacking a GPU adapter; per-element
    // filter falls back to the CPU path via apply_chain_cpu_bbox.
    let wgpu_pair = crate::shader::create_wgpu();

    // 3. Walk each frame in order, guarded by a per-frame watchdog.
    //
    // The watchdog is a background thread that reads two atomics the main
    // thread updates:
    //   * `current_frame_started_ns` — wall-clock nanos at start of the
    //     active frame's work (0 == idle/not started).
    //   * `last_pushed_frame` — index of the most recent frame the main
    //     thread successfully pushed to the encoder (-1 before any).
    // It wakes once a second; if the active frame's start is older than
    // the budget, it sets `abort` and prints a diagnostic.
    //
    // The main loop checks `abort` at every safe point (top of loop, after
    // push_frame). We do NOT try to interrupt mid-`push_frame` /
    // mid-`renderer.render` — neither libavformat nor Stylo/Vello expose a
    // safe interrupt primitive, and triggering UB to abort a hang is
    // worse than the hang. This means a truly-wedged frame still requires
    // the parent-level (eval harness, supervisor) timeout to recover; the
    // watchdog at least surfaces a real-time diagnostic so the human knows
    // which frame wedged, and slow-but-completing frames get aborted
    // cleanly with a structured error.
    let abort = Arc::new(AtomicBool::new(false));
    let current_frame_started_ns = Arc::new(AtomicU64::new(0));
    let last_pushed_frame = Arc::new(AtomicI64::new(-1));
    let watchdog_done = Arc::new(AtomicBool::new(false));
    let budget_secs = opts.frame_budget_secs;

    let watchdog_handle = {
        let abort = Arc::clone(&abort);
        let started = Arc::clone(&current_frame_started_ns);
        let last = Arc::clone(&last_pushed_frame);
        let done = Arc::clone(&watchdog_done);
        let epoch = Instant::now();
        std::thread::spawn(move || {
            let budget = Duration::from_secs(budget_secs);
            let mut warned = false;
            while !done.load(Ordering::Relaxed) {
                std::thread::sleep(Duration::from_millis(500));
                if done.load(Ordering::Relaxed) {
                    break;
                }
                let started_ns = started.load(Ordering::Relaxed);
                if started_ns == 0 {
                    continue;
                }
                let started_at = epoch + Duration::from_nanos(started_ns);
                let elapsed = Instant::now().saturating_duration_since(started_at);
                if elapsed >= budget {
                    if !warned {
                        eprintln!(
                            "wavelet watchdog: frame after {} has been running {}s (budget {}s) — aborting at next safe point",
                            last.load(Ordering::Relaxed),
                            elapsed.as_secs(),
                            budget_secs,
                        );
                        warned = true;
                    }
                    abort.store(true, Ordering::Relaxed);
                }
            }
        })
    };

    // Drop-guard so the watchdog thread joins even on early return / panic.
    struct WatchdogGuard {
        done: Arc<AtomicBool>,
        handle: Option<std::thread::JoinHandle<()>>,
    }
    impl Drop for WatchdogGuard {
        fn drop(&mut self) {
            self.done.store(true, Ordering::Relaxed);
            if let Some(h) = self.handle.take() {
                let _ = h.join();
            }
        }
    }
    let _watchdog_guard = WatchdogGuard {
        done: Arc::clone(&watchdog_done),
        handle: Some(watchdog_handle),
    };

    let watchdog_epoch = Instant::now();

    // wb-5w9s.2: progress reporting. Without this, agents see ~100 bytes
    // of stdout/stderr across a multi-minute render — no way to tell
    // working from wedged. Emit on stderr every ~10% (or every 30 frames
    // for very short comps), plus one line at the start so they see SOMETHING
    // within ~1 frame's worth of wall-clock.
    let total_frames = comp.duration_frames.max(1);
    let progress_step_frames = (total_frames / 10).max(1).min(30);
    let render_epoch = Instant::now();
    eprintln!(
        "wavelet render: starting frame 0/{total_frames} ({}x{}@{}fps)",
        comp.width, comp.height, comp.fps,
    );

    let bg_pixels = vec![0u8; (comp.width * comp.height * 4) as usize];
    for frame in 0..comp.duration_frames {
        if abort.load(Ordering::Relaxed) {
            return Err(RenderOfflineError::FrameBudgetExceeded {
                frame_index: frame,
                budget_secs,
                last_frame_index: last_pushed_frame.load(Ordering::Relaxed),
            });
        }
        let frame_start = Instant::now();
        let ns = watchdog_epoch.elapsed().as_nanos() as u64;
        current_frame_started_ns.store(ns.max(1), Ordering::Relaxed);
        let pixels = match active_scene(comp, frame) {
            None => bg_pixels.clone(),
            Some(scene_idx) => {
                let scene = &comp.scenes[scene_idx];
                let local_frame = frame - scene.start_frame;
                let local_t_secs = local_frame as f32 / comp.fps as f32;

                // Check for an active transition into this scene.
                let in_transition = scene
                    .transition_in
                    .as_ref()
                    .filter(|t| t.duration_secs > 0.0 && scene_idx > 0)
                    .map(|t| {
                        let dur = t.duration_secs;
                        let window_frames = (dur * comp.fps as f32) as u32;
                        (t, dur, window_frames)
                    });

                let in_window = in_transition
                    .as_ref()
                    .map(|(_, _, wf)| local_frame < *wf)
                    .unwrap_or(false);

                if in_window {
                    let (t_spec, dur, _wf) = in_transition.as_ref().unwrap();
                    let progress = (local_t_secs / dur).clamp(0.0, 1.0);

                    // Render frame B (this scene at local_t_secs).
                    let runtime_b = scene_state
                        .entry(scene_idx)
                        .or_insert_with(|| build_scene_runtime(scene, root_dir, comp.width, comp.height));
                    let frame_b = render_scene_frame(runtime_b, &mut renderer, local_frame, local_t_secs, comp.fps, wgpu_pair.as_ref());

                    // Render frame A — the previous scene at its final
                    // settled frame. (Cheaper than re-ticking; the previous
                    // scene's motion has already settled by definition.)
                    let prev_scene = &comp.scenes[scene_idx - 1];
                    let prev_last_frame = prev_scene.duration_frames.saturating_sub(1);
                    let prev_last_local =
                        prev_last_frame as f32 / comp.fps as f32;
                    let runtime_a = scene_state.entry(scene_idx - 1).or_insert_with(|| {
                        build_scene_runtime(prev_scene, root_dir, comp.width, comp.height)
                    });
                    let frame_a = render_scene_frame(runtime_a, &mut renderer, prev_last_frame, prev_last_local, comp.fps, wgpu_pair.as_ref());

                    // Build the pipeline lazily.
                    let pipeline = transitions.entry(scene_idx).or_insert_with(|| {
                        let (dev, q) = wgpu_pair
                            .as_ref()
                            .expect("wgpu_pair must be Some when transitions are present")
                            .clone();
                        let src = crate::shader::fx_source(&t_spec.wavelet_fx)
                            .unwrap_or_else(|e| {
                                panic!(
                                    "transition compile failed for scene {scene_idx}: {e}\n\
                                     wavelet_fx source:\n{}",
                                    t_spec.wavelet_fx
                                )
                            });
                        crate::shader::TransitionPipeline::new(dev, q, comp.width, comp.height, &src)
                            .unwrap_or_else(|e| panic!("transition pipeline: {e}"))
                    });
                    let absolute_t = frame as f32 / comp.fps as f32;
                    pipeline.render(&frame_a, &frame_b, absolute_t, progress)
                } else {
                    let runtime = scene_state
                        .entry(scene_idx)
                        .or_insert_with(|| build_scene_runtime(scene, root_dir, comp.width, comp.height));
                    render_scene_frame(runtime, &mut renderer, local_frame, local_t_secs, comp.fps, wgpu_pair.as_ref())
                }
            }
        };
        encoder.push_frame(&RgbaFrame::new(comp.width, comp.height, pixels))?;
        let elapsed = frame_start.elapsed();
        if elapsed.as_secs() >= budget_secs {
            // Frame eventually completed but exceeded budget. Surface the
            // structured error so the agent can abandon this scene instead
            // of letting subsequent (probably-also-slow) frames eat the
            // rest of the wall-clock budget.
            return Err(RenderOfflineError::FrameBudgetExceeded {
                frame_index: frame,
                budget_secs,
                last_frame_index: last_pushed_frame.load(Ordering::Relaxed),
            });
        }
        current_frame_started_ns.store(0, Ordering::Relaxed);
        last_pushed_frame.store(frame as i64, Ordering::Relaxed);
        if abort.load(Ordering::Relaxed) {
            return Err(RenderOfflineError::FrameBudgetExceeded {
                frame_index: frame,
                budget_secs,
                last_frame_index: last_pushed_frame.load(Ordering::Relaxed),
            });
        }
        // wb-5w9s.2: emit progress on completed-frame boundaries that
        // align with progress_step_frames (every ~10% of total). +1 to
        // include the final frame in the report.
        let completed = frame + 1;
        if completed % progress_step_frames == 0 || completed == total_frames {
            let pct = (completed * 100) / total_frames;
            let elapsed_s = render_epoch.elapsed().as_secs_f32();
            let fps = completed as f32 / elapsed_s.max(0.001);
            eprintln!(
                "wavelet render: frame {completed}/{total_frames} ({pct}%) elapsed={elapsed_s:.1}s {fps:.1}fps"
            );
        }
    }
    encoder.finalize()?;
    stats.video_frames = comp.duration_frames as u64;
    stats.mp4_bytes = std::fs::metadata(out_path).map(|m| m.len()).unwrap_or(0);

    // 4. Audio: render to WAV alongside if any cues present.
    //
    // Cues come from two places: the manifest-level `comp.audio_cues` list
    // (from `index.html`'s top-level `<audio>` or a raw `comp.json`), and
    // per-scene `<audio>` elements embedded inside each scene's HTML. We
    // discover the latter by re-reading each scene file (cheap — already on
    // disk; build_scene_runtime reads it lazily for video anyway). Cues with
    // identical `(asset_path, start_frame)` are de-duplicated so a track
    // declared in both surfaces doesn't double-mix.
    let mut combined_cues: Vec<AudioCueSpec> = comp.audio_cues.clone();
    for (scene_idx, scene) in comp.scenes.iter().enumerate() {
        let scene_path = root_dir.join(&scene.html_path);
        let scene_html = match std::fs::read_to_string(&scene_path) {
            Ok(s) => s,
            Err(_) => continue,
        };
        match crate::compose::extract_scene_audio_cues(
            &scene_html,
            scene,
            scene_idx,
            comp.fps,
            comp.duration_frames,
        ) {
            Ok(scene_cues) if !scene_cues.is_empty() => {
                combined_cues = crate::compose::merge_dedup(&combined_cues, scene_cues);
            }
            Ok(_) => {}
            Err(e) => {
                eprintln!(
                    "warning: scene {} audio extraction failed: {e}",
                    scene_path.display()
                );
            }
        }
    }

    if !combined_cues.is_empty() {
        let mut mixer = AudioMixer::new(48_000, comp.fps);
        let mut load_errors: Vec<(String, String)> = Vec::new();
        let mut loaded_cue_count = 0usize;
        for cue in &combined_cues {
            let resolved = root_dir.join(&cue.asset_path);
            let cue_obj = AudioCue {
                asset_path: resolved.clone(),
                id: cue.id.clone(),
                start_frame: cue.start_frame as u64,
                duration_frames: cue.duration_frames as u64,
                volume: cue.volume,
                pan: cue.pan,
                fade_in_frames: cue.fade_in_frames as u64,
                fade_out_frames: cue.fade_out_frames as u64,
                duck_targets: cue.duck_targets.clone(),
                duck_db: cue.duck_db,
                align_to_beat: cue.align_to_beat,
            };
            // A broken <audio src> at this point is a soft-fail: warn
            // to stderr, drop the cue, continue. The audio-presence
            // lint surfaces the missing ref to the agent in a single
            // structured report; failing the render here would be
            // redundant and worse — it would block a sensible "render
            // video-only and fix the audio later" workflow.
            match mixer.add_cue(cue_obj) {
                Ok(()) => {
                    loaded_cue_count += 1;
                }
                Err(e) => {
                    load_errors.push((resolved.display().to_string(), e.to_string()));
                }
            }
        }
        for (path, err) in &load_errors {
            eprintln!("wavelet render: audio cue dropped ({path}): {err}");
        }
        // If every cue failed to load (typical: agent set a broken
        // `<audio src>` and the asset doesn't exist yet), skip the
        // mix entirely. We still want the render to succeed — the
        // audio-presence lint surfaces the missing ref — but we don't
        // want a silent AAC stream that misleads downstream tools
        // ("the mp4 has audio, why is it silent?").
        if loaded_cue_count > 0 {
            let stereo = mixer.render(comp.duration_frames as u64)?;
            let wav_path = out_path.with_extension("wav");
            write_stereo_wav(&wav_path, &stereo, 48_000)?;
            stats.audio_samples_per_channel = (stereo.len() / 2) as u64;
            stats.wav_bytes = std::fs::metadata(&wav_path).map(|m| m.len()).unwrap_or(0);

            if opts.mux_audio && !stereo.is_empty() {
                super::audio_mux::mux_stereo_into_mp4(out_path, &stereo, 48_000)?;
                stats.mp4_bytes = std::fs::metadata(out_path).map(|m| m.len()).unwrap_or(stats.mp4_bytes);
            }
        }
    }

    stats.elapsed_ms = render_start.elapsed().as_millis();
    Ok(stats)
}

