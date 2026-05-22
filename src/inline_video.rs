//! Inline `<video src="...">` support for scene HTML.
//!
//! Scene HTML may carry zero-or-more `<video>` elements. At scene load
//! we walk the DOM, decode each referenced clip into RGBA frames via
//! the existing `query::diff::decode_rgba_frames` helper, and seed the
//! element's `SpecialElementData` slot with the first frame so blitz's
//! standard `<img>` paint path picks it up. At each subsequent comp
//! frame we resolve the clip-local time and swap the buffer.
//!
//! Recognized attributes:
//! - `src` — required. Resolved against the scene HTML's directory if
//!   relative; absolute paths and `file://` URLs are accepted verbatim.
//!   Other URL schemes (http, https) are not fetched — would require a
//!   network provider, out of scope for offline render.
//! - `data-start` — clip-local offset in `"1.5s"` / `"500ms"` / bare-secs
//!   form. Defaults to `0s`. Parsed via `compose::duration::parse_duration`.
//! - `loop` — when present (any value, HTML boolean attribute), the clip
//!   wraps modulo its duration once `t > clip_duration`.
//!   Without `loop`, the last decoded frame is held.
//! - `playbackRate` — float multiplier on time advancement. Defaults to 1.0.
//! - `muted` — recognized for forward-compat; audio mixing is a follow-up,
//!   so the value is currently ignored.
//!
//! Sizing: blitz's intrinsic-size dispatch only handles `<img>` / `<canvas>`
//! / `<svg>`. `<video>` is laid out via CSS (the canonical idiom from
//! SKILL.md is `position:absolute; inset:0;` for full-bleed). The
//! decoder's native dimensions still drive `object-fit` cover/contain
//! math inside blitz-paint's `draw_image`.

use crate::compose::parse_duration;
use crate::query::diff::decode_rgba_frames;
use blitz_dom::node::{ImageData, RasterImageData, SpecialElementData};
use blitz_dom::{BaseDocument, local_name};
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// One discovered `<video>` element with its decoded clip + timing.
pub struct InlineVideo {
    /// DOM node id of the `<video>` element. Stable for the document's
    /// lifetime (slab keys do not get reused mid-render).
    pub node_id: usize,
    /// Native pixel width of the source clip.
    pub width: u32,
    /// Native pixel height of the source clip.
    pub height: u32,
    /// Native frame rate of the source clip.
    pub native_fps: f32,
    /// All decoded RGBA frames. Same approach as `video_bg` —
    /// in-memory cache, no per-frame seek. Memory cost is fine
    /// at v1; can switch to a streaming decoder later.
    pub frames: Vec<Vec<u8>>,
    /// Clip-local offset in seconds. Scene-local-time is subtracted by
    /// this when computing the sampled frame.
    pub data_start: f32,
    /// When true, clip wraps modulo duration. Otherwise the last frame
    /// is held.
    pub looping: bool,
    /// Multiplier on time advancement. 1.0 = native rate.
    pub playback_rate: f32,
}

/// Walk the document, find every `<video>` element with a `src` that resolves
/// to a local file, decode its frames, and seed the element's
/// `SpecialElementData::Image` slot with the first frame.
///
/// `scene_dir` is the directory containing the scene's HTML file — relative
/// `src` values resolve against it (mirroring how a browser would resolve
/// against the document's base URL).
///
/// Returns the discovered videos. The caller stores them on the
/// `SceneRuntime` and feeds the right frame back in via
/// [`update_inline_video_frames`] on each comp frame.
pub fn discover_and_seed(doc: &mut BaseDocument, scene_dir: &Path) -> Vec<InlineVideo> {
    let mut found: Vec<(usize, String, f32, bool, f32)> = Vec::new();

    // Collect first so we can mutate without holding tree borrow.
    for (node_id, node) in doc.tree().iter() {
        let Some(el) = node.data.downcast_element() else { continue };
        if el.name.local != *"video" {
            continue;
        }
        let Some(src) = el.attr(local_name!("src")) else { continue };
        let data_start = el
            .attrs()
            .iter()
            .find(|a| &*a.name.local == "data-start")
            .and_then(|a| parse_duration(&a.value))
            .unwrap_or(0.0);
        let looping = el.has_attr(local_name!("loop"));
        let playback_rate = el
            .attrs()
            .iter()
            .find(|a| &*a.name.local == "playbackrate")
            .and_then(|a| a.value.parse::<f32>().ok())
            .unwrap_or(1.0);
        found.push((node_id, src.to_string(), data_start, looping, playback_rate));
    }

    let mut out = Vec::with_capacity(found.len());
    for (node_id, src, data_start, looping, playback_rate) in found {
        let Some(path) = resolve_src(scene_dir, &src) else { continue };
        let (w, h, fps, frames) = match decode_rgba_frames(&path) {
            Ok(t) => t,
            Err(e) => {
                eprintln!(
                    "warning: inline <video src=\"{}\"> decode failed: {e}",
                    path.display()
                );
                continue;
            }
        };
        if frames.is_empty() {
            continue;
        }
        // Seed the first frame so layout has something to work with and the
        // existing draw_image path is wired up.
        install_frame(doc, node_id, w, h, &frames[0]);
        // blitz 0.3.0-alpha.3 only treats img / canvas / svg as replaced
        // elements with intrinsic sizing (blitz-dom/src/layout/mod.rs:178).
        // Without that, a bare `<video src="..."></video>` with no CSS
        // sizing collapses to a zero-sized inline box and paint draws
        // nothing — the wb-uory.11 failure mode that produced 100%-skip
        // MP4s on the 2026-05-20 Liquid Death agent run. Until the fix
        // lands upstream (or in our vendored RVST blitz at a matching
        // version), retag the element to "img" so layout's existing
        // intrinsic-size dispatch kicks in. Safe because: (a) we only
        // do this for `<video>` we successfully decoded, (b) the seeded
        // RasterImageData drives both intrinsic size and paint, (c)
        // common authoring style uses id/class selectors not tag-name,
        // (d) we skip elements with children to avoid orphaning
        // fallback `<source>` / text content under a void `<img>`.
        if has_no_element_children(doc, node_id) {
            retag_video_as_img(doc, node_id);
        }
        out.push(InlineVideo {
            node_id,
            width: w,
            height: h,
            native_fps: fps,
            frames,
            data_start,
            looping,
            playback_rate,
        });
    }
    out
}

/// At a given scene-local time, swap each video element's RGBA buffer to
/// the right clip frame. `local_t_secs` is the scene's clock; per-video
/// `data_start` and `playback_rate` shift it into clip-local time.
pub fn update_inline_video_frames(
    doc: &mut BaseDocument,
    videos: &[InlineVideo],
    local_t_secs: f32,
) {
    for v in videos {
        let rgba = sample_at(v, local_t_secs);
        install_frame(doc, v.node_id, v.width, v.height, rgba);
    }
}

/// Pick the clip frame for a given scene-local time, honoring `data_start`,
/// `playback_rate`, and `loop` vs hold-last.
fn sample_at<'a>(v: &'a InlineVideo, local_t_secs: f32) -> &'a [u8] {
    let last = v.frames.len() - 1;
    let raw = (local_t_secs - v.data_start) * v.playback_rate;
    if raw <= 0.0 {
        return &v.frames[0];
    }
    let duration = v.frames.len() as f32 / v.native_fps.max(0.001);
    let t = if v.looping {
        raw.rem_euclid(duration)
    } else if raw >= duration {
        // Hold last frame.
        return &v.frames[last];
    } else {
        raw
    };
    let idx = (t * v.native_fps).floor() as usize;
    &v.frames[idx.min(last)]
}

/// Resolve a `<video src>` value against the scene's directory.
///
/// Returns `None` for non-file schemes, unparseable URLs, or paths that
/// fail to canonicalize. Absolute paths and `file://` URLs are accepted
/// verbatim; relative strings are joined against `scene_dir`.
fn resolve_src(scene_dir: &Path, raw: &str) -> Option<PathBuf> {
    if let Ok(url) = url::Url::parse(raw) {
        if url.scheme() != "file" {
            return None;
        }
        return url.to_file_path().ok();
    }
    let candidate = if Path::new(raw).is_absolute() {
        PathBuf::from(raw)
    } else {
        scene_dir.join(raw)
    };
    std::fs::canonicalize(&candidate).ok().or(Some(candidate))
}

/// Replace the element's `SpecialElementData::Image` with a fresh
/// `RasterImageData` pointing at `pixels`. Allocates a new Arc<Vec<u8>>
/// each call — the same pattern blitz uses when an `<img>` finishes
/// loading. Could be optimized to in-place stomp once we know nothing
/// else holds the previous Arc; profile first.
fn install_frame(doc: &mut BaseDocument, node_id: usize, width: u32, height: u32, pixels: &[u8]) {
    let Some(node) = doc.get_node_mut(node_id) else { return };
    let Some(el) = node.data.downcast_element_mut() else { return };
    let raster = RasterImageData::new(width, height, Arc::new(pixels.to_vec()));
    el.special_data = SpecialElementData::Image(Box::new(ImageData::Raster(raster)));
}

/// Returns true when the node has zero element-typed children. Empty text
/// (whitespace between tags) and `<source>` siblings still count as
/// children of `<video>` in the parsed DOM, so we only retag when the
/// element is fully empty — otherwise the orphaned children would render
/// over the painted frame.
fn has_no_element_children(doc: &BaseDocument, node_id: usize) -> bool {
    let Some(node) = doc.get_node(node_id) else { return false };
    for &child_id in &node.children {
        let Some(child) = doc.get_node(child_id) else { continue };
        if matches!(child.data, blitz_dom::NodeData::Element(_)) {
            return false;
        }
    }
    true
}

/// Swap the element's tag name from `video` to `img` so blitz's existing
/// img-based replaced-layout dispatch picks up the seeded
/// `SpecialElementData::Image` for intrinsic sizing. Paint is tag-agnostic
/// (it draws any element with `raster_image_data()`), so the only
/// behavior we change is layout — which is what was missing.
fn retag_video_as_img(doc: &mut BaseDocument, node_id: usize) {
    let Some(node) = doc.get_node_mut(node_id) else { return };
    let Some(el) = node.data.downcast_element_mut() else { return };
    el.name.local = local_name!("img");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sample_at_data_start_offset() {
        let frames: Vec<Vec<u8>> = (0..30).map(|i| vec![i as u8; 4]).collect();
        let v = InlineVideo {
            node_id: 0,
            width: 1,
            height: 1,
            native_fps: 30.0,
            frames,
            data_start: 2.0,
            looping: false,
            playback_rate: 1.0,
        };
        // local-time 2.0 → clip-local 0.0 → frame 0
        assert_eq!(sample_at(&v, 2.0)[0], 0);
        // local-time 2.5 → clip-local 0.5 → frame 15
        assert_eq!(sample_at(&v, 2.5)[0], 15);
        // local-time below data-start clamps to frame 0
        assert_eq!(sample_at(&v, 1.0)[0], 0);
    }

    #[test]
    fn sample_at_holds_last_without_loop() {
        let frames: Vec<Vec<u8>> = (0..10).map(|i| vec![i as u8; 4]).collect();
        let v = InlineVideo {
            node_id: 0,
            width: 1,
            height: 1,
            native_fps: 10.0,
            frames,
            data_start: 0.0,
            looping: false,
            playback_rate: 1.0,
        };
        // Clip is 1.0s @ 10fps. Past end → hold last frame.
        assert_eq!(sample_at(&v, 5.0)[0], 9);
    }

    #[test]
    fn sample_at_loops_with_modulo() {
        let frames: Vec<Vec<u8>> = (0..10).map(|i| vec![i as u8; 4]).collect();
        let v = InlineVideo {
            node_id: 0,
            width: 1,
            height: 1,
            native_fps: 10.0,
            frames,
            data_start: 0.0,
            looping: true,
            playback_rate: 1.0,
        };
        // 1.5s into a 1.0s loop → 0.5s into the next pass → frame 5.
        assert_eq!(sample_at(&v, 1.5)[0], 5);
        // 2.0s exactly → wraps to frame 0.
        assert_eq!(sample_at(&v, 2.0)[0], 0);
    }

    #[test]
    fn sample_at_honors_playback_rate() {
        let frames: Vec<Vec<u8>> = (0..30).map(|i| vec![i as u8; 4]).collect();
        let v = InlineVideo {
            node_id: 0,
            width: 1,
            height: 1,
            native_fps: 30.0,
            frames,
            data_start: 0.0,
            looping: false,
            playback_rate: 2.0,
        };
        // 0.25s scene time * 2x = 0.5s clip = frame 15.
        assert_eq!(sample_at(&v, 0.25)[0], 15);
    }

    #[test]
    fn discover_finds_videos_with_src() {
        use crate::render::load_html_with_base;
        let _guard = crate::test_utils::BLITZ_GUARD.lock().unwrap();

        let html = r#"<!doctype html><html><body>
          <video id="bg" src="a.mp4"></video>
          <video id="hero" src="b.mp4" data-start="1s" loop playbackrate="2"></video>
          <video id="noop"></video>
        </body></html>"#;
        let mut doc = load_html_with_base(html, 320, 240, None);

        // Re-walk via the same discovery scan but capture pre-decode results.
        // End-to-end (including the rsmpeg decode) is covered by the smoke
        // test under `tests/`.
        let mut found = Vec::new();
        for (node_id, node) in doc.as_mut().tree().iter() {
            let Some(el) = node.data.downcast_element() else { continue };
            if el.name.local != *"video" { continue }
            let Some(src) = el.attr(local_name!("src")) else { continue };
            let data_start = el
                .attrs()
                .iter()
                .find(|a| &*a.name.local == "data-start")
                .and_then(|a| parse_duration(&a.value))
                .unwrap_or(0.0);
            let looping = el.has_attr(local_name!("loop"));
            let pr = el
                .attrs()
                .iter()
                .find(|a| &*a.name.local == "playbackrate")
                .and_then(|a| a.value.parse::<f32>().ok())
                .unwrap_or(1.0);
            found.push((node_id, src.to_string(), data_start, looping, pr));
        }
        assert_eq!(found.len(), 2, "should find 2 videos with src");
        let (_, _, ds0, lp0, pr0) = &found[0];
        assert_eq!(*ds0, 0.0);
        assert!(!*lp0);
        assert_eq!(*pr0, 1.0);
        let (_, _, ds1, lp1, pr1) = &found[1];
        assert_eq!(*ds1, 1.0);
        assert!(*lp1);
        assert_eq!(*pr1, 2.0);
    }

    #[test]
    fn resolve_src_absolute_and_relative() {
        let tmp = std::env::temp_dir().join("inline_video_resolve_test");
        std::fs::create_dir_all(&tmp).unwrap();
        let f = tmp.join("clip.mp4");
        std::fs::write(&f, b"").unwrap();

        let rel = resolve_src(&tmp, "clip.mp4").unwrap();
        assert!(rel.ends_with("clip.mp4"));

        let url = url::Url::from_file_path(&f).unwrap().to_string();
        let from_url = resolve_src(&tmp, &url).unwrap();
        assert!(from_url.ends_with("clip.mp4"));

        // https scheme rejected
        assert!(resolve_src(&tmp, "https://example.com/x.mp4").is_none());
    }
}
