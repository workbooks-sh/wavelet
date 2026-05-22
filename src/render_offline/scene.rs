//! `render_offline::scene` — extracted from godfile split.

#![allow(missing_docs)]

use crate::inline_video::{discover_and_seed, update_inline_video_frames, InlineVideo};
use crate::query::diff::decode_rgba_frames;
use crate::render::{load_html_with_base, Renderer};
use blitz_html::HtmlDocument;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64};
use std::sync::Arc;
use super::types::{Composition, SceneSpec};

pub(super) struct SceneRuntime {
    doc: HtmlDocument,
    /// Pre-decoded background-video frames, if the scene has `video_bg`.
    /// One RGBA buffer per source-clip frame, indexed by `clip_frame_idx`.
    video_bg_frames: Vec<Vec<u8>>,
    /// Native frame rate of the source clip. Used to map comp-frame
    /// indices to clip-frame indices time-correctly so a 16fps Wan clip
    /// composes properly under a 30fps composition.
    video_bg_native_fps: f32,
    /// Inline `<video>` elements discovered in the scene HTML. Each carries
    /// its own decoded frame buffer + timing; the per-frame loop seeks
    /// each one independently and swaps the buffer into the element's
    /// `SpecialElementData::Image` slot so blitz-paint's standard image
    /// path renders it.
    inline_videos: Vec<InlineVideo>,
    /// Body-level `filter:` chain extracted at scene load. Applied as a
    /// whole-frame post-process via [`crate::css_filter::apply_chain_cpu`]
    /// after each frame's render. Empty when the scene declares no
    /// filter on body. Non-body filters are stripped at load time
    /// (preventing Blitz/Vello hangs) and surfaced via the diagnostic
    /// list in [`build_scene_runtime`].
    body_filter_chain: Vec<crate::css_filter::FilterFn>,
    /// Per-element `filter:` chains keyed by injected `data-wavelet-fxid`
    /// markers. The render path walks the DOM each frame, finds every
    /// element with one of these markers, resolves the element's
    /// absolute bbox via Blitz's `absolute_position()` +
    /// `final_layout.size`, and applies the chain to that region of
    /// the rendered RGBA via [`crate::css_filter::apply_chain_cpu_bbox`].
    element_filter_chains: Vec<(String, Vec<crate::css_filter::FilterFn>)>,
}

pub(super) fn build_scene_runtime(
    scene: &SceneSpec,
    root_dir: &Path,
    width: u32,
    height: u32,
) -> SceneRuntime {
    let resolved = root_dir.join(&scene.html_path);
    let html = std::fs::read_to_string(&resolved)
        .unwrap_or_else(|e| panic!("scene html unreadable {}: {e}", resolved.display()));
    // Set base_url to the scene file's absolute path as file:// so relative
    // <img src> / <link href> references resolve against the scene's dir.
    let absolute = std::fs::canonicalize(&resolved).unwrap_or(resolved.clone());
    let base_url = url::Url::from_file_path(&absolute)
        .ok()
        .map(|u| u.to_string());
    let scene_dir_for_resolve = absolute
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| root_dir.to_path_buf());
    let html = match crate::compose::resolve_clip_refs(&html, &scene_dir_for_resolve) {
        Ok(rewritten) => rewritten,
        Err(e) => panic!("scene html clip-ref resolve failed {}: {e}", resolved.display()),
    };
    // CSS `filter:` hijack — extract body-level filter for whole-frame
    // post-process, strip all `filter:` declarations so they never reach
    // Blitz/Vello (where they hang on common values like blur(>=4px) +
    // multi drop-shadow + filter on <video>). Non-body filters are
    // reported but not applied in v1; per-element bbox scoping is
    // round-2 follow-on under wb-5w9s.7.
    let hijack = crate::css_filter::hijack_filters_in_html(&html);
    if !hijack.stripped_no_apply.is_empty() {
        eprintln!(
            "scene {}: stripped {} filter declaration(s) without applying — \
             selector shape not yet supported by per-element apply \
             (currently: simple class + inline style only):",
            resolved.display(),
            hijack.stripped_no_apply.len(),
        );
        for d in &hijack.stripped_no_apply {
            eprintln!("  - {d}");
        }
    }
    if !hijack.element_filter_chains.is_empty() {
        eprintln!(
            "scene {}: per-element CSS filter applied to {} element chain(s) via wavelet-fx",
            resolved.display(),
            hijack.element_filter_chains.len(),
        );
    }
    let html = hijack.stripped_html;
    let body_filter_chain = hijack.body_filter_chain;
    let element_filter_chains = hijack.element_filter_chains;
    let mut doc = load_html_with_base(&html, width, height, base_url);
    let scene_dir = absolute.parent().unwrap_or(root_dir).to_path_buf();
    let inline_videos = discover_and_seed(doc.as_mut(), &scene_dir);
    // Re-resolve layout in case any inline-video element's intrinsic image
    // slot mutated the tree's replaced-content shape.
    if !inline_videos.is_empty() {
        doc.as_mut().resolve(0.0);
    }

    // Decode the background video once if present. Scales any source size to
    // our comp's (width, height) via swscale (already happens in
    // decode_rgba_frames). Frames are RGBA8.
    let (video_bg_frames, video_bg_native_fps) = if let Some(bg_path) = scene.video_bg.as_ref() {
        let resolved = root_dir.join(bg_path);
        // rsmpeg / libavformat wants an absolute path; canonicalize so we
        // don't depend on cwd matching root_dir.
        let absolute = std::fs::canonicalize(&resolved).unwrap_or(resolved.clone());
        match decode_rgba_frames(&absolute) {
            Ok((vw, vh, native_fps, frames)) => {
                if vw != width || vh != height {
                    eprintln!(
                        "warning: video_bg {} is {}x{} but comp is {}x{} — using as-is",
                        absolute.display(), vw, vh, width, height
                    );
                }
                (frames, native_fps)
            }
            Err(e) => {
                eprintln!(
                    "warning: video_bg {} decode failed: {e}",
                    absolute.display()
                );
                (Vec::new(), 30.0)
            }
        }
    } else {
        (Vec::new(), 30.0)
    };

    SceneRuntime {
        doc,
        video_bg_frames,
        video_bg_native_fps,
        inline_videos,
        body_filter_chain,
        element_filter_chains,
    }
}

/// Sample the scene's video_bg at a given local-scene-frame, mapping
/// time-correctly from the comp's fps to the clip's native fps. When the
/// clip's duration is shorter than the scene, the *last clip frame is
/// held* (no loop) — looping a stock or generated clip produces a
/// jarring jump that's almost never what the author wanted. Returns
/// `None` when the scene has no video_bg.
pub(super) fn sample_video_bg(rt: &SceneRuntime, local_frame: u32, comp_fps: u32) -> Option<&[u8]> {
    if rt.video_bg_frames.is_empty() {
        return None;
    }
    let t_secs = local_frame as f32 / comp_fps.max(1) as f32;
    let clip_idx = (t_secs * rt.video_bg_native_fps).floor() as usize;
    let last = rt.video_bg_frames.len() - 1;
    let idx = clip_idx.min(last);
    Some(&rt.video_bg_frames[idx])
}

/// Alpha-composite `fg` over `bg` (standard "over" operator, straight alpha).
/// Both buffers must be the same dimensions; output has alpha=255.
pub(super) fn compose_over(bg: &[u8], fg: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(bg.len());
    for (b, f) in bg.chunks_exact(4).zip(fg.chunks_exact(4)) {
        let a = f[3] as u32;
        let inv = 255 - a;
        out.push(((f[0] as u32 * a + b[0] as u32 * inv) / 255) as u8);
        out.push(((f[1] as u32 * a + b[1] as u32 * inv) / 255) as u8);
        out.push(((f[2] as u32 * a + b[2] as u32 * inv) / 255) as u8);
        out.push(255);
    }
    out
}

/// Render a scene to RGBA — picks the video_bg + transparent HTML compose
/// path when applicable, falls back to the plain opaque HTML render.
pub(super) fn render_scene_frame(
    runtime: &mut SceneRuntime,
    renderer: &mut Renderer,
    local_frame: u32,
    local_t_secs: f32,
    comp_fps: u32,
    wgpu_pair: Option<&(Arc<wgpu::Device>, Arc<wgpu::Queue>)>,
) -> Vec<u8> {
    // Refresh inline <video> frames before style resolution so the new
    // RasterImageData drives any object-fit math + layout cascades.
    if !runtime.inline_videos.is_empty() {
        update_inline_video_frames(
            runtime.doc.as_mut(),
            &runtime.inline_videos,
            local_t_secs,
        );
    }
    // Drive Stylo's CSS animation engine. `resolve(now)` walks active
    // @keyframes + transition sets and recomputes styles for time
    // `now`.
    runtime.doc.as_mut().resolve(local_t_secs as f64);
    let mut rgba = if let Some(bg) = sample_video_bg(runtime, local_frame, comp_fps) {
        let bg_owned = bg.to_vec();
        let fg = renderer.render_transparent(runtime.doc.as_mut());
        compose_over(&bg_owned, &fg)
    } else {
        renderer.render(runtime.doc.as_mut())
    };
    let (out_w, out_h) = renderer.dimensions();
    // Per-element CSS filter pass — runs BEFORE body filter so that a
    // body-level grade composites on top of element-specific effects
    // (matches CSS stacking-context semantics: each filtered element
    // resolves before parent-level filters).
    if !runtime.element_filter_chains.is_empty() {
        apply_element_filters(runtime, &mut rgba, out_w, out_h, wgpu_pair);
    }
    // Apply body-level CSS filter as a whole-frame post-process.
    // CSS-spec-correct because body's bbox equals the viewport.
    if !runtime.body_filter_chain.is_empty() {
        crate::css_filter::apply_chain_cpu(
            &mut rgba,
            out_w,
            out_h,
            &runtime.body_filter_chain,
            out_w as f32,
            out_h as f32,
        );
    }
    rgba
}

/// Walk the document for every element tagged with a `data-wavelet-fxid`
/// attribute, resolve its absolute layout bbox via Blitz, and apply
/// the recorded filter chain to that region of the rendered RGBA.
///
/// Multiple elements can share the same fxid (CSS rule that matched
/// several nodes). Each match contributes its own bbox-scoped apply.
/// The chain runs in document order, so deeper-nested filtered elements
/// composite on top of their ancestors' filtered output — matching
/// CSS stacking-context evaluation order.
pub(super) fn apply_element_filters(
    runtime: &SceneRuntime,
    rgba: &mut [u8],
    width: u32,
    height: u32,
    wgpu_pair: Option<&(Arc<wgpu::Device>, Arc<wgpu::Queue>)>,
) {
    let doc = runtime.doc.as_ref();
    // Collect (fxid_string, node_id) pairs in document order via the
    // tree iterator. We then look up each fxid's chain in the
    // element_filter_chains list. We look up `data-wavelet-fxid` by raw
    // string match against the attribute list — html5ever's
    // `local_name!` macro pre-interns only standard HTML attrs, not
    // custom data- attributes.
    let mut targets: Vec<(String, usize)> = Vec::new();
    for (node_id, node) in doc.tree().iter() {
        let Some(el) = node.data.downcast_element() else { continue };
        let fxid = el
            .attrs()
            .iter()
            .find(|a| a.name.local.as_ref() == "data-wavelet-fxid")
            .map(|a| a.value.to_string());
        let Some(fxid) = fxid else { continue };
        targets.push((fxid, node_id));
    }
    if targets.is_empty() {
        return;
    }
    for (fxid, node_id) in targets {
        // Look up the chain for this fxid.
        let Some((_, chain)) = runtime
            .element_filter_chains
            .iter()
            .find(|(id, _)| id == &fxid)
        else {
            continue;
        };
        // Resolve bbox via Blitz's absolute_position() + final_layout.size.
        let Some(node_ref) = doc.get_node(node_id) else { continue };
        let pos = node_ref.absolute_position(0.0, 0.0);
        let size = node_ref.final_layout.size;
        if size.width <= 0.0 || size.height <= 0.0 {
            continue;
        }
        // Prefer GPU when wgpu is available — orders of magnitude
        // faster for blur-heavy chains. Fall back to CPU on headless.
        // Chain selection heuristic: if the chain contains a blur with
        // radius >= 4, GPU wins (CPU box-blur is O(radius²)). For
        // pure per-pixel ops, CPU is comparable but consistent with
        // the rest of the path; use GPU anyway for uniformity.
        if let Some((dev, q)) = wgpu_pair {
            crate::shader::filter_pass::apply_chain_gpu_bbox(
                dev,
                q,
                rgba,
                width,
                height,
                pos.x as i32,
                pos.y as i32,
                size.width as u32,
                size.height as u32,
                chain,
                width as f32,
                height as f32,
            );
        } else {
            crate::css_filter::apply_chain_cpu_bbox(
                rgba,
                width,
                height,
                pos.x as i32,
                pos.y as i32,
                size.width as u32,
                size.height as u32,
                chain,
                width as f32,
                height as f32,
            );
        }
    }
}

