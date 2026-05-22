//! HTML-manifest parser for multi-scene compositions.
//!
//! The agent author writes a single top-level `index.html` that lists scenes
//! and audio cues. This module parses that file into the in-memory
//! [`Composition`] that [`crate::render_offline::render_composition`] consumes.
//!
//! ## Manifest shape
//!
//! ```html
//! <!doctype html>
//! <html><head>
//!   <title>Tree Runner Spot</title>
//!   <meta name="resolution" content="1280x720">
//!   <meta name="fps" content="30">
//!   <meta name="duration" content="15s">
//! </head><body>
//!   <section data-scene-href="scenes/01-title.html"   data-duration="3s"></section>
//!   <section data-scene-href="scenes/02-product.html" data-duration="6s"
//!            data-transition-in="crossfade" data-transition-duration="0.5s"></section>
//!   <audio src="music/track.wav" data-spans="all"></audio>
//!   <audio src="vo/line1.wav" data-start="2s" data-fade-in="0.2s"></audio>
//! </body></html>
//! ```
//!
//! Scope: top-level `<section>` and `<audio>` elements only — no nesting.

use crate::render_offline::{AudioCueSpec, Composition, SceneSpec, TransitionSpec};
use std::path::{Path, PathBuf};

mod duration;
mod parse;
mod resolve;
mod scene_audio;

pub use duration::parse_resolution;
pub use duration::{parse_duration};
pub use parse::{collect_elements, Element, ElementKind};
pub use resolve::{extract_audio_clip_cues, resolve_clip_refs, ResolveError};
pub use scene_audio::{extract_scene_audio_cues, merge_dedup};

/// Errors raised by [`load_index_html`].
#[derive(Debug, thiserror::Error)]
pub enum ComposeError {
    /// Failed to read the manifest file.
    #[error("read {path}: {source}")]
    Io {
        /// Path the caller tried to read.
        path: String,
        /// Underlying I/O error.
        #[source]
        source: std::io::Error,
    },
    /// Manifest parse failure — malformed HTML or unparseable attributes.
    #[error("parse {path}: {reason}")]
    Parse {
        /// Path of the manifest being parsed.
        path: String,
        /// Reason the parse failed.
        reason: String,
    },
    /// A required `<meta>` tag was missing.
    #[error("missing required meta tag: {0}")]
    MissingMeta(&'static str),
    /// An attribute value couldn't be coerced to its expected type.
    #[error("invalid {field}: {value}")]
    Invalid {
        /// Field name (e.g. `data-duration`).
        field: &'static str,
        /// The offending value.
        value: String,
    },
}

/// Parse an `index.html` manifest into a [`Composition`].
///
/// Relative `data-scene-href` / `<audio src>` paths are kept relative — the
/// caller (`render_composition`) resolves them against the manifest's parent
/// directory passed as `root_dir`.
pub fn load_index_html(path: &Path) -> Result<Composition, ComposeError> {
    let html = std::fs::read_to_string(path).map_err(|source| ComposeError::Io {
        path: path.display().to_string(),
        source,
    })?;
    parse_manifest(&html, path)
}

fn parse_manifest(html: &str, path: &Path) -> Result<Composition, ComposeError> {
    let path_str = path.display().to_string();

    let metas = parse::collect_meta(html);
    let resolution_raw = metas
        .iter()
        .find(|m| m.name.eq_ignore_ascii_case("resolution"))
        .map(|m| m.content.clone())
        .ok_or(ComposeError::MissingMeta("resolution"))?;
    let (width, height) =
        parse_resolution(&resolution_raw).ok_or_else(|| ComposeError::Invalid {
            field: "meta[name=resolution]",
            value: resolution_raw.clone(),
        })?;

    let fps_raw = metas
        .iter()
        .find(|m| m.name.eq_ignore_ascii_case("fps"))
        .map(|m| m.content.clone())
        .ok_or(ComposeError::MissingMeta("fps"))?;
    let fps: u32 = fps_raw.trim().parse().map_err(|_| ComposeError::Invalid {
        field: "meta[name=fps]",
        value: fps_raw.clone(),
    })?;
    if fps == 0 {
        return Err(ComposeError::Invalid { field: "meta[name=fps]", value: fps_raw });
    }

    let elements = parse::collect_elements(html).map_err(|e| ComposeError::Parse {
        path: path_str.clone(),
        reason: e,
    })?;

    let mut scenes = Vec::new();
    let mut audio_cues = Vec::new();
    let mut next_start_frame: u32 = 0;

    for el in &elements {
        match el.kind {
            ElementKind::Section => {
                let href = el.attr("data-scene-href").ok_or_else(|| ComposeError::Parse {
                    path: path_str.clone(),
                    reason: "<section> missing data-scene-href".into(),
                })?;
                let dur_raw =
                    el.attr("data-duration").ok_or_else(|| ComposeError::Parse {
                        path: path_str.clone(),
                        reason: format!("<section data-scene-href=\"{href}\"> missing data-duration"),
                    })?;
                let dur_secs = parse_duration(dur_raw).ok_or_else(|| ComposeError::Invalid {
                    field: "data-duration",
                    value: dur_raw.into(),
                })?;
                let duration_frames = secs_to_frames(dur_secs, fps);

                let transition_in = match el.attr("data-transition-in") {
                    Some(kind) if kind != "cut" => {
                        let dur_raw = el.attr("data-transition-duration").unwrap_or("0.5s");
                        let dur = parse_duration(dur_raw).ok_or_else(|| ComposeError::Invalid {
                            field: "data-transition-duration",
                            value: dur_raw.into(),
                        })?;
                        Some(transition_for(kind, dur).ok_or_else(|| ComposeError::Invalid {
                            field: "data-transition-in",
                            value: kind.into(),
                        })?)
                    }
                    _ => None,
                };

                scenes.push(SceneSpec {
                    html_path: PathBuf::from(href),
                    start_frame: next_start_frame,
                    duration_frames,
                    transition_in,
                    video_bg: None,
                });
                next_start_frame += duration_frames;
            }
            ElementKind::Audio => {
                let src = el.attr("src").ok_or_else(|| ComposeError::Parse {
                    path: path_str.clone(),
                    reason: "<audio> missing src".into(),
                })?;
                audio_cues.push(parse_audio(el, src, fps, &path_str)?);
            }
            // <video> in the top-level manifest is unsupported — videos
            // live inside scene HTML files. We collect ElementKind::Video
            // for the render pre-flight asset check; here in the manifest
            // parser they're just ignored.
            ElementKind::Video => {}
        }
    }

    let manifest_dir = path.parent().unwrap_or_else(|| Path::new("."));
    for scene in &scenes {
        let scene_path = manifest_dir.join(&scene.html_path);
        if !scene_path.exists() {
            continue;
        }
        let Ok(scene_html) = std::fs::read_to_string(&scene_path) else {
            continue;
        };
        let scene_dir = scene_path
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| manifest_dir.to_path_buf());
        let cues = resolve::extract_audio_clip_cues(
            &scene_html,
            &scene_dir,
            fps,
            scene.start_frame,
            scene.duration_frames,
        )
        .map_err(|e| ComposeError::Parse {
            path: scene_path.display().to_string(),
            reason: e.to_string(),
        })?;
        audio_cues.extend(cues);
    }

    // Composition duration: explicit meta wins; otherwise sum of scenes.
    let total_frames = if let Some(meta) =
        metas.iter().find(|m| m.name.eq_ignore_ascii_case("duration"))
    {
        let secs = parse_duration(&meta.content).ok_or_else(|| ComposeError::Invalid {
            field: "meta[name=duration]",
            value: meta.content.clone(),
        })?;
        secs_to_frames(secs, fps)
    } else {
        next_start_frame
    };

    // Resolve audio cue spans now that we know total_frames.
    for (i, el) in elements
        .iter()
        .filter(|e| matches!(e.kind, ElementKind::Audio))
        .enumerate()
    {
        if el.attr("data-spans").map(|v| v.eq_ignore_ascii_case("all")).unwrap_or(false) {
            let start = audio_cues[i].start_frame;
            audio_cues[i].duration_frames = total_frames.saturating_sub(start);
        }
    }

    Ok(Composition {
        width,
        height,
        fps,
        duration_frames: total_frames,
        aspect: None,
        scenes,
        audio_cues,
    })
}

fn parse_audio(
    el: &Element,
    src: &str,
    fps: u32,
    _path: &str,
) -> Result<AudioCueSpec, ComposeError> {
    let start_secs = match el.attr("data-start") {
        Some(v) => parse_duration(v).ok_or_else(|| ComposeError::Invalid {
            field: "data-start",
            value: v.into(),
        })?,
        None => 0.0,
    };
    let dur_frames = match el.attr("data-duration") {
        Some(v) => {
            let secs = parse_duration(v).ok_or_else(|| ComposeError::Invalid {
                field: "data-duration",
                value: v.into(),
            })?;
            secs_to_frames(secs, fps)
        }
        None => 0,
    };
    let fade_in_frames = match el.attr("data-fade-in") {
        Some(v) => {
            let secs = parse_duration(v).ok_or_else(|| ComposeError::Invalid {
                field: "data-fade-in",
                value: v.into(),
            })?;
            secs_to_frames(secs, fps)
        }
        None => 0,
    };
    let fade_out_frames = match el.attr("data-fade-out") {
        Some(v) => {
            let secs = parse_duration(v).ok_or_else(|| ComposeError::Invalid {
                field: "data-fade-out",
                value: v.into(),
            })?;
            secs_to_frames(secs, fps)
        }
        None => 0,
    };
    let volume: f32 = match el.attr("data-volume") {
        Some(v) => v.parse().map_err(|_| ComposeError::Invalid {
            field: "data-volume",
            value: v.into(),
        })?,
        None => 1.0,
    };
    let id = el
        .attr("id")
        .map(|s| s.to_string())
        .unwrap_or_else(|| derive_cue_id(src));

    Ok(AudioCueSpec {
        asset_path: PathBuf::from(src),
        id,
        start_frame: secs_to_frames(start_secs, fps),
        duration_frames: dur_frames,
        volume,
        pan: 0.0,
        fade_in_frames,
        fade_out_frames,
        duck_targets: Vec::new(),
        duck_db: 0.0,
        align_to_beat: false,
    })
}

fn derive_cue_id(src: &str) -> String {
    Path::new(src)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("audio")
        .to_string()
}

fn secs_to_frames(secs: f32, fps: u32) -> u32 {
    (secs * fps as f32).round() as u32
}

fn transition_for(kind: &str, duration_secs: f32) -> Option<TransitionSpec> {
    // The renderer applies inline wavelet_fx source — we ship a curated set of
    // named transitions that resolve to known-good wavelet_fx programs. Unknown
    // names fall through as an `Invalid` error to the caller.
    let wavelet_fx = match kind {
        "crossfade" | "fade" => Some(SHADY_CROSSFADE.to_string()),
        other if other.starts_with("shader:") => {
            // shader:<name> — author-supplied wavelet_fx literal expected on disk;
            // for now we only know crossfade so anything else errors.
            return None;
        }
        _ => None,
    }?;
    Some(TransitionSpec { wavelet_fx, duration_secs })
}

/// Default crossfade wavelet_fx program. `src(0)` = outgoing, `src(1)` = incoming,
/// `progress` ∈ [0, 1] bound by the orchestrator. WaveletFx is Hydra-shaped
/// (method-chain DSL, not WGSL); the canonical crossfade is `.blend(rhs,
/// progress).out()`, NOT `mix(a, b, t)`.
const SHADY_CROSSFADE: &str = "src(0).blend(src(1), progress).out()";

#[cfg(test)]
mod tests {
    use super::*;

    fn write_tmp(name: &str, body: &str) -> PathBuf {
        let dir = std::env::temp_dir().join("wavelet-compose-tests");
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join(name);
        std::fs::write(&p, body).unwrap();
        p
    }

    #[test]
    fn parses_minimal_single_scene() {
        let p = write_tmp(
            "minimal.html",
            r#"<!doctype html><html><head>
<meta name="resolution" content="640x360">
<meta name="fps" content="30">
</head><body>
<section data-scene-href="a.html" data-duration="2s"></section>
</body></html>"#,
        );
        let comp = load_index_html(&p).unwrap();
        assert_eq!((comp.width, comp.height), (640, 360));
        assert_eq!(comp.fps, 30);
        assert_eq!(comp.scenes.len(), 1);
        assert_eq!(comp.scenes[0].start_frame, 0);
        assert_eq!(comp.scenes[0].duration_frames, 60);
        assert_eq!(comp.duration_frames, 60);
        assert_eq!(comp.scenes[0].html_path, PathBuf::from("a.html"));
    }

    #[test]
    fn three_scenes_cumulative_start_frames() {
        let p = write_tmp(
            "three.html",
            r#"<!doctype html><html><head>
<meta name="resolution" content="1280x720">
<meta name="fps" content="30">
</head><body>
<section data-scene-href="a.html" data-duration="3s"></section>
<section data-scene-href="b.html" data-duration="6s"></section>
<section data-scene-href="c.html" data-duration="3s"></section>
</body></html>"#,
        );
        let comp = load_index_html(&p).unwrap();
        assert_eq!(comp.scenes.len(), 3);
        let starts: Vec<u32> = comp.scenes.iter().map(|s| s.start_frame).collect();
        assert_eq!(starts, vec![0, 90, 270]);
        assert_eq!(comp.duration_frames, 360);
    }

    #[test]
    fn missing_resolution_errors() {
        let p = write_tmp(
            "noresolution.html",
            r#"<!doctype html><html><head><meta name="fps" content="30"></head><body></body></html>"#,
        );
        let err = load_index_html(&p).unwrap_err();
        assert!(matches!(err, ComposeError::MissingMeta("resolution")));
    }

    #[test]
    fn missing_fps_errors() {
        let p = write_tmp(
            "nofps.html",
            r#"<!doctype html><html><head><meta name="resolution" content="1280x720"></head><body></body></html>"#,
        );
        let err = load_index_html(&p).unwrap_err();
        assert!(matches!(err, ComposeError::MissingMeta("fps")));
    }

    #[test]
    fn duration_parses_all_forms() {
        assert!((parse_duration("3s").unwrap() - 3.0).abs() < 1e-6);
        assert!((parse_duration("1500ms").unwrap() - 1.5).abs() < 1e-6);
        assert!((parse_duration("0.5s").unwrap() - 0.5).abs() < 1e-6);
        assert!((parse_duration("45").unwrap() - 45.0).abs() < 1e-6);
    }

    #[test]
    fn transition_in_attached() {
        let p = write_tmp(
            "transition.html",
            r#"<!doctype html><html><head>
<meta name="resolution" content="1280x720">
<meta name="fps" content="30">
</head><body>
<section data-scene-href="a.html" data-duration="2s"></section>
<section data-scene-href="b.html" data-duration="2s"
         data-transition-in="crossfade" data-transition-duration="0.5s"></section>
</body></html>"#,
        );
        let comp = load_index_html(&p).unwrap();
        assert!(comp.scenes[0].transition_in.is_none());
        let t = comp.scenes[1].transition_in.as_ref().expect("crossfade present");
        assert!((t.duration_secs - 0.5).abs() < 1e-6);
        assert!(t.wavelet_fx.contains("blend"));
    }

    #[test]
    fn audio_spans_all_covers_full_comp() {
        let p = write_tmp(
            "audio_all.html",
            r#"<!doctype html><html><head>
<meta name="resolution" content="1280x720">
<meta name="fps" content="30">
</head><body>
<section data-scene-href="a.html" data-duration="2s"></section>
<section data-scene-href="b.html" data-duration="4s"></section>
<audio src="music/track.wav" data-spans="all"></audio>
</body></html>"#,
        );
        let comp = load_index_html(&p).unwrap();
        assert_eq!(comp.audio_cues.len(), 1);
        let cue = &comp.audio_cues[0];
        assert_eq!(cue.start_frame, 0);
        assert_eq!(cue.duration_frames, 180);
        assert_eq!(cue.asset_path, PathBuf::from("music/track.wav"));
    }

    #[test]
    fn audio_data_start_offsets() {
        let p = write_tmp(
            "audio_start.html",
            r#"<!doctype html><html><head>
<meta name="resolution" content="1280x720">
<meta name="fps" content="30">
</head><body>
<section data-scene-href="a.html" data-duration="5s"></section>
<audio src="vo/line.wav" data-start="2s" data-duration="2s" data-fade-in="0.2s"></audio>
</body></html>"#,
        );
        let comp = load_index_html(&p).unwrap();
        let cue = &comp.audio_cues[0];
        assert_eq!(cue.start_frame, 60);
        assert_eq!(cue.duration_frames, 60);
        assert_eq!(cue.fade_in_frames, 6);
    }
}
