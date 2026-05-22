//! Per-scene `<audio>` extraction.
//!
//! The agent author writes a scene's HTML with inline `<audio>` cues:
//!
//! ```html
//! <!doctype html>
//! <html><body>
//!   <h1>Title</h1>
//!   <audio src="music/bed.wav"></audio>
//!   <audio src="sfx/whoosh.wav" data-start="0.2s" data-fade-in="0.1s"></audio>
//!   <audio src="vo/line1.wav" data-start="0.8s" data-pan="-0.3"></audio>
//! </body></html>
//! ```
//!
//! At scene-load time the renderer extracts those cues, lowers them to
//! [`AudioCueSpec`], and merges them with the manifest-level `audio_cues`
//! list so the rest of the audio pipeline is unchanged.
//!
//! `data-start` is interpreted scene-local. `start_frame` of the emitted
//! cue is `scene.start_frame + start_secs * fps`. Without `data-duration`,
//! the cue is clipped to the end of the scene unless `data-spans="all"`
//! is set (in which case it spans through the end of the composition).

use std::path::{Path, PathBuf};

use super::duration::parse_duration;
use super::parse::{collect_elements, ElementKind};
use super::ComposeError;
use crate::render_offline::{AudioCueSpec, SceneSpec};

/// Walk a scene's HTML body and lower every `<audio src=…>` element into an
/// [`AudioCueSpec`]. Paths inside `src` are resolved relative to the scene's
/// own HTML file (i.e. `scene.html_path.parent()`).
///
/// `scene_idx` is used to generate stable cue ids of the form
/// `scene-{scene_idx}-audio-{element_idx}` so id collisions across scenes
/// can't happen.
pub fn extract_scene_audio_cues(
    scene_html: &str,
    scene: &SceneSpec,
    scene_idx: usize,
    fps: u32,
    comp_duration_frames: u32,
) -> Result<Vec<AudioCueSpec>, ComposeError> {
    let elements = collect_elements(scene_html).map_err(|e| ComposeError::Parse {
        path: scene.html_path.display().to_string(),
        reason: e,
    })?;

    let scene_dir = scene.html_path.parent().unwrap_or_else(|| Path::new(""));
    let mut out = Vec::new();
    let mut audio_idx: usize = 0;

    for el in &elements {
        if !matches!(el.kind, ElementKind::Audio) {
            continue;
        }
        let src = el.attr("src").ok_or_else(|| ComposeError::Parse {
            path: scene.html_path.display().to_string(),
            reason: "<audio> missing src".into(),
        })?;

        let start_secs = match el.attr("data-start") {
            Some(v) => parse_duration(v).ok_or_else(|| ComposeError::Invalid {
                field: "data-start",
                value: v.into(),
            })?,
            None => 0.0,
        };
        let fade_in_frames = parse_optional_duration_frames(el.attr("data-fade-in"), fps, "data-fade-in")?;
        let fade_out_frames = parse_optional_duration_frames(el.attr("data-fade-out"), fps, "data-fade-out")?;
        let explicit_dur_frames = match el.attr("data-duration") {
            Some(v) => Some({
                let secs = parse_duration(v).ok_or_else(|| ComposeError::Invalid {
                    field: "data-duration",
                    value: v.into(),
                })?;
                secs_to_frames(secs, fps)
            }),
            None => None,
        };
        let pan = match el.attr("data-pan") {
            Some(v) => {
                let n: f32 = v.parse().map_err(|_| ComposeError::Invalid {
                    field: "data-pan",
                    value: v.into(),
                })?;
                n.clamp(-1.0, 1.0)
            }
            None => 0.0,
        };
        let volume = match el.attr("volume").or_else(|| el.attr("data-volume")) {
            Some(v) => v.parse::<f32>().map_err(|_| ComposeError::Invalid {
                field: "volume",
                value: v.into(),
            })?,
            None => 1.0,
        };
        let spans_all = el
            .attr("data-spans")
            .map(|v| v.eq_ignore_ascii_case("all"))
            .unwrap_or(false);

        let scene_local_start_frame = secs_to_frames(start_secs, fps);
        let start_frame = scene.start_frame.saturating_add(scene_local_start_frame);

        let duration_frames = if spans_all {
            comp_duration_frames.saturating_sub(start_frame)
        } else if let Some(d) = explicit_dur_frames {
            // Clip to scene boundary.
            let scene_end = scene.start_frame + scene.duration_frames;
            let raw_end = start_frame.saturating_add(d);
            raw_end.min(scene_end).saturating_sub(start_frame)
        } else {
            // Default: play through to end of scene.
            let scene_end = scene.start_frame + scene.duration_frames;
            scene_end.saturating_sub(start_frame)
        };

        let resolved_src: PathBuf = if scene_dir.as_os_str().is_empty() {
            PathBuf::from(src)
        } else {
            scene_dir.join(src)
        };
        let id = el
            .attr("id")
            .map(|s| s.to_string())
            .unwrap_or_else(|| format!("scene-{scene_idx}-audio-{audio_idx}"));

        out.push(AudioCueSpec {
            asset_path: resolved_src,
            id,
            start_frame,
            duration_frames,
            volume,
            pan,
            fade_in_frames,
            fade_out_frames,
            duck_targets: Vec::new(),
            duck_db: 0.0,
            align_to_beat: false,
        });
        audio_idx += 1;
    }

    Ok(out)
}

/// Merge scene-derived cues into a manifest cue list. Cues with the same
/// `(asset_path, start_frame)` as a manifest cue are dropped — they're
/// considered a re-declaration of the same cue from two surfaces.
pub fn merge_dedup(
    manifest: &[AudioCueSpec],
    scene_cues: Vec<AudioCueSpec>,
) -> Vec<AudioCueSpec> {
    use std::collections::HashSet;
    let mut seen: HashSet<(PathBuf, u32)> = manifest
        .iter()
        .map(|c| (c.asset_path.clone(), c.start_frame))
        .collect();
    let mut out: Vec<AudioCueSpec> = manifest.to_vec();
    for cue in scene_cues {
        let key = (cue.asset_path.clone(), cue.start_frame);
        if seen.insert(key) {
            out.push(cue);
        }
    }
    out
}

fn parse_optional_duration_frames(
    raw: Option<&str>,
    fps: u32,
    field: &'static str,
) -> Result<u32, ComposeError> {
    match raw {
        Some(v) => {
            let secs = parse_duration(v).ok_or_else(|| ComposeError::Invalid {
                field,
                value: v.into(),
            })?;
            Ok(secs_to_frames(secs, fps))
        }
        None => Ok(0),
    }
}

fn secs_to_frames(secs: f32, fps: u32) -> u32 {
    (secs * fps as f32).round() as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scene(html_path: &str, start_frame: u32, duration_frames: u32) -> SceneSpec {
        SceneSpec {
            html_path: PathBuf::from(html_path),
            start_frame,
            duration_frames,
            transition_in: None,
            video_bg: None,
        }
    }

    #[test]
    fn extracts_three_audio_elements_with_correct_specs() {
        let html = r#"<!doctype html><html><body>
<h1 id="title">FREEFORM</h1>
<audio src="music/scene-bed.wav"></audio>
<audio src="sfx/whoosh.wav" data-start="0.2s" data-fade-in="0.1s" data-fade-out="0.2s"></audio>
<audio src="vo/line1.wav" data-start="0.8s" data-pan="-0.3"></audio>
</body></html>"#;
        let s = scene("scenes/scene-01.html", 30, 60);
        let cues = extract_scene_audio_cues(html, &s, 0, 30, 1000).unwrap();
        assert_eq!(cues.len(), 3);

        assert_eq!(cues[0].asset_path, PathBuf::from("scenes/music/scene-bed.wav"));
        assert_eq!(cues[0].start_frame, 30);
        assert_eq!(cues[0].duration_frames, 60);
        assert_eq!(cues[0].pan, 0.0);
        assert_eq!(cues[0].fade_in_frames, 0);
        assert_eq!(cues[0].id, "scene-0-audio-0");

        assert_eq!(cues[1].asset_path, PathBuf::from("scenes/sfx/whoosh.wav"));
        assert_eq!(cues[1].start_frame, 36);
        assert_eq!(cues[1].fade_in_frames, 3);
        assert_eq!(cues[1].fade_out_frames, 6);
        assert_eq!(cues[1].id, "scene-0-audio-1");

        assert_eq!(cues[2].asset_path, PathBuf::from("scenes/vo/line1.wav"));
        assert_eq!(cues[2].start_frame, 54);
        assert!((cues[2].pan - (-0.3)).abs() < 1e-6);
        assert_eq!(cues[2].id, "scene-0-audio-2");
    }

    #[test]
    fn spans_all_overrides_scene_boundary() {
        let html =
            r#"<audio src="music/track.wav" data-start="0s" data-spans="all"></audio>"#;
        let s = scene("a.html", 0, 60);
        let cues = extract_scene_audio_cues(html, &s, 0, 30, 600).unwrap();
        assert_eq!(cues.len(), 1);
        assert_eq!(cues[0].start_frame, 0);
        assert_eq!(cues[0].duration_frames, 600);
    }

    #[test]
    fn default_duration_clips_to_scene_end() {
        let html = r#"<audio src="x.wav"></audio>"#;
        let s = scene("scenes/a.html", 90, 60);
        let cues = extract_scene_audio_cues(html, &s, 1, 30, 300).unwrap();
        assert_eq!(cues[0].start_frame, 90);
        assert_eq!(cues[0].duration_frames, 60);
    }

    #[test]
    fn explicit_duration_clips_to_scene_end() {
        let html = r#"<audio src="x.wav" data-duration="10s"></audio>"#;
        let s = scene("scenes/a.html", 0, 60);
        let cues = extract_scene_audio_cues(html, &s, 0, 30, 300).unwrap();
        // 10s = 300 frames, but scene is only 60 frames → clipped.
        assert_eq!(cues[0].duration_frames, 60);
    }

    #[test]
    fn pan_out_of_range_clamps() {
        let html = r#"<audio src="x.wav" data-pan="1.5"></audio>"#;
        let s = scene("a.html", 0, 30);
        let cues = extract_scene_audio_cues(html, &s, 0, 30, 30).unwrap();
        assert_eq!(cues[0].pan, 1.0);
    }

    #[test]
    fn missing_src_errors() {
        let html = r#"<audio data-start="1s"></audio>"#;
        let s = scene("a.html", 0, 30);
        let err = extract_scene_audio_cues(html, &s, 0, 30, 30).unwrap_err();
        assert!(matches!(err, ComposeError::Parse { .. }));
    }

    #[test]
    fn merge_dedup_drops_exact_collisions() {
        let manifest = vec![AudioCueSpec {
            asset_path: PathBuf::from("a.wav"),
            id: "m".into(),
            start_frame: 0,
            duration_frames: 30,
            volume: 1.0,
            pan: 0.0,
            fade_in_frames: 0,
            fade_out_frames: 0,
            duck_targets: vec![],
            duck_db: 0.0,
            align_to_beat: false,
        }];
        let scene_cues = vec![
            AudioCueSpec {
                asset_path: PathBuf::from("a.wav"),
                id: "s".into(),
                start_frame: 0,
                duration_frames: 30,
                volume: 1.0,
                pan: 0.0,
                fade_in_frames: 0,
                fade_out_frames: 0,
                duck_targets: vec![],
                duck_db: 0.0,
                align_to_beat: false,
            },
            AudioCueSpec {
                asset_path: PathBuf::from("b.wav"),
                id: "s2".into(),
                start_frame: 0,
                duration_frames: 30,
                volume: 1.0,
                pan: 0.0,
                fade_in_frames: 0,
                fade_out_frames: 0,
                duck_targets: vec![],
                duck_db: 0.0,
                align_to_beat: false,
            },
        ];
        let merged = merge_dedup(&manifest, scene_cues);
        // Manifest cue stays; first scene cue is a duplicate; second scene cue is new.
        assert_eq!(merged.len(), 2);
        assert_eq!(merged[0].id, "m");
        assert_eq!(merged[1].id, "s2");
    }

    #[test]
    fn data_volume_attribute_is_honored() {
        let html = r#"<audio src="x.wav" data-volume="0.5"></audio>"#;
        let s = scene("a.html", 0, 30);
        let cues = extract_scene_audio_cues(html, &s, 0, 30, 30).unwrap();
        assert_eq!(cues[0].volume, 0.5);
    }

    #[test]
    fn no_audio_elements_returns_empty() {
        let html = r#"<!doctype html><html><body><h1>just text</h1></body></html>"#;
        let s = scene("a.html", 0, 30);
        let cues = extract_scene_audio_cues(html, &s, 0, 30, 30).unwrap();
        assert!(cues.is_empty());
    }
}
