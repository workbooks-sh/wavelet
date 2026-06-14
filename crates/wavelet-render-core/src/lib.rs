//! # wavelet-render-core — the carved render slice
//!
//! Proves the wavelet motion-graphics render path can paint composition
//! frames with **no native exec, no GPU, no rsmpeg** — the keystone for
//! "tenants assemble video inside an isolated workbook (the nexus / WASM
//! sandbox)".
//!
//! ## What this carve includes
//!
//! - **HTML + CSS + layout**: upstream Blitz (`blitz-dom` + `blitz-html` +
//!   `blitz-paint`). Stylo for CSS, Taffy for layout, Parley for text.
//! - **Timeline**: Stylo's CSS-animation clock, driven by
//!   `BaseDocument::resolve(t)`. A frame at time `t` differs from `t=0`
//!   when the scene declares `@keyframes` + `animation:` — this is what
//!   makes it *motion* graphics, not a static screenshot.
//! - **Assets**: a [`FileNetProvider`] resolves `<img src>` against the
//!   composition's directory via WASI `std::fs` (relative paths) and decodes
//!   `data:` URIs inline. No live network — asset fetch is filesystem-only,
//!   which is exactly what a WASI preopened tenant dir gives the nexus.
//! - **Paint**: `anyrender_vello_cpu` (pure-CPU Vello, peniko/kurbo). No
//!   wgpu, no GPU adapter.
//! - **Output**: RGBA `Vec<u8>` or PNG bytes; a whole frame *sequence* to a
//!   directory.
//!
//! ## What this carve EXCLUDES (stays host-side)
//!
//! - `rsmpeg` / system ffmpeg (native = bedrock under WASM): encode of
//!   frames → mp4 is a host broker or a Forge ffmpeg→wasi lane.
//! - `wgpu` / `anyrender_vello` (GPU).
//! - Live `http(s)` fetch (a host capability, not in-sandbox).

use std::path::Path;
use std::sync::Arc;

use anyrender::{ImageRenderer, PaintScene as _};
use anyrender_vello_cpu::VelloCpuImageRenderer;
use blitz_dom::{build_single_font_ctx, BaseDocument, DocumentConfig};
use blitz_html::HtmlDocument;
use blitz_paint::paint_scene;
use blitz_traits::net::{Bytes, NetHandler, NetProvider, Request};
use blitz_traits::shell::{ColorScheme, Viewport};
use kurbo::Rect;
use peniko::{Color, Fill};

/// Bundled fallback font. Under WASI there is no system-font discovery
/// (`system_fonts` is off), so text would render as nothing unless a font
/// is registered. We embed one permissively-licensed family (Geist, OFL)
/// and alias it to every generic family — exactly the WASM setup blitz-dom
/// documents for `build_single_font_ctx`. A production nexus can swap in a
/// host-fetched, content-addressed font here instead of the embedded blob.
static BUNDLED_FONT: &[u8] = include_bytes!("../assets/font.ttf");

/// A [`NetProvider`] that serves sub-resources (currently `<img>`) from the
/// filesystem and from inline `data:` URIs — and nothing else.
///
/// Blitz hands `fetch` a *fully resolved* [`Url`](url::Url): a relative
/// `<img src="hero.png">` is joined against the document `base_url` we set to
/// the composition's directory, so it arrives as `file:///abs/dir/hero.png`.
/// `data:` URIs arrive verbatim. Both are resolved **synchronously** here —
/// we call `handler.bytes(...)` inline, which posts the decoded bytes to the
/// document's message queue (drained by `handle_messages()` + `resolve()`).
///
/// This is the in-sandbox asset model: a WASI guest can only see what the
/// host preopened (the tenant dir), so "the filesystem" *is* the asset
/// sandbox. There is no live `http(s)` path on purpose.
pub struct FileNetProvider;

impl NetProvider for FileNetProvider {
    fn fetch(&self, _doc_id: usize, request: Request, handler: Box<dyn NetHandler>) {
        let url = request.url;
        match url.scheme() {
            "data" => {
                // Decode per WHATWG data: URL processing (handles base64 +
                // percent-encoding + mime). Mirrors upstream blitz providers.
                match data_url::DataUrl::process(url.as_str()) {
                    Ok(data_url) => match data_url.decode_to_vec() {
                        Ok((body, _frag)) => {
                            handler.bytes(url.to_string(), Bytes::from(body));
                        }
                        Err(e) => eprintln!("data: URI decode failed: {e:?}"),
                    },
                    Err(e) => eprintln!("data: URI parse failed: {e:?}"),
                }
            }
            // `file:` asset loading needs a real filesystem. It exists on
            // WASI (wasm32-wasip1/2, the nexus target) and native, but NOT on
            // wasm32-unknown-unknown, where `url`'s file-path conversions are
            // compiled out. Keep the lib building on every target; the file
            // branch just isn't reachable where there's no fs.
            #[cfg(any(not(target_arch = "wasm32"), target_os = "wasi"))]
            "file" => match url.to_file_path() {
                Ok(path) => match std::fs::read(&path) {
                    Ok(body) => handler.bytes(url.to_string(), Bytes::from(body)),
                    Err(e) => eprintln!("asset read failed {}: {e}", path.display()),
                },
                Err(()) => eprintln!("file: URL has no path: {url}"),
            },
            other => {
                // No live network in the nexus. Surface, don't fetch.
                eprintln!("FileNetProvider: refusing non-local scheme {other:?} ({url})");
            }
        }
    }
}

/// Build the `base_url` (a `file://` directory URL, trailing slash) that
/// relative `<img src>` paths resolve against, from the composition file's
/// parent directory. Returns `None` for a path with no parent.
#[cfg(all(target_arch = "wasm32", not(target_os = "wasi")))]
fn dir_base_url(_composition_path: &Path) -> Option<String> {
    // No filesystem on wasm32-unknown-unknown: relative file assets are N/A;
    // only data: URIs resolve. (The nexus runs on wasm32-wasip1, which has fs.)
    None
}

#[cfg(any(not(target_arch = "wasm32"), target_os = "wasi"))]
fn dir_base_url(composition_path: &Path) -> Option<String> {
    use std::path::PathBuf;
    let dir = composition_path.parent()?;
    // Absolute path → file:// URL. Make it absolute against CWD if needed so
    // WASI preopen resolution works regardless of how the path was passed.
    let abs: PathBuf = if dir.is_absolute() {
        dir.to_path_buf()
    } else {
        std::env::current_dir().ok()?.join(dir)
    };
    let mut url = url::Url::from_directory_path(&abs).ok()?;
    // from_directory_path already adds the trailing slash needed for join().
    url.set_query(None);
    Some(url.to_string())
}

/// Parse + resolve an HTML composition string into a Blitz document.
///
/// `base_url` (a `file://` dir URL) is what relative `<img src>` paths
/// resolve against; pass `None` for a self-contained composition (only
/// `data:` URIs). A [`FileNetProvider`] is always wired so assets load.
pub fn load_html_with_base(
    html: &str,
    width: u32,
    height: u32,
    base_url: Option<String>,
) -> HtmlDocument {
    let mut document = HtmlDocument::from_html(
        html,
        DocumentConfig {
            viewport: Some(Viewport::new(width, height, 1.0, ColorScheme::Light)),
            font_ctx: Some(build_single_font_ctx(BUNDLED_FONT)),
            net_provider: Some(Arc::new(FileNetProvider)),
            base_url,
            ..Default::default()
        },
    );
    // Drain queued messages + resolve. Asset fetches resolve synchronously
    // inside our provider, but the decoded bytes land in the message queue,
    // so we need extra passes to (a) issue the fetch, (b) ingest the decoded
    // image, (c) re-layout with the now-known intrinsic size.
    for _ in 0..4 {
        document.as_mut().handle_messages();
        document.as_mut().resolve(0.0);
    }
    document
}

/// Back-compat: parse a self-contained composition (no base dir → relative
/// `<img>` paths will fail to resolve; use [`load_html_with_base`] or the
/// `*_from_path` helpers for assets).
pub fn load_html(html: &str, width: u32, height: u32) -> HtmlDocument {
    load_html_with_base(html, width, height, None)
}

/// Paint a resolved document to RGBA8 on an opaque white background.
fn render_doc_rgba(doc: &mut BaseDocument, width: u32, height: u32) -> Vec<u8> {
    let mut renderer = VelloCpuImageRenderer::new(width, height);
    let bg = Rect::new(0.0, 0.0, width as f64, height as f64);
    let mut buf = Vec::with_capacity((width * height * 4) as usize);
    renderer.render_to_vec(
        |scene| {
            scene.fill(
                Fill::NonZero,
                Default::default(),
                Color::WHITE,
                Default::default(),
                &bg,
            );
            paint_scene(scene, doc, 1.0, width, height, 0, 0);
        },
        &mut buf,
    );
    buf
}

/// Core entrypoint: parse a composition string, seek the CSS-animation clock
/// to `t = frame / fps`, paint, return RGBA8 (`width * height * 4`).
///
/// `base_url` (a `file://` dir URL) resolves relative `<img>` assets.
pub fn render_frame_rgba_with_base(
    composition_html: &str,
    frame: u32,
    fps: u32,
    width: u32,
    height: u32,
    base_url: Option<String>,
) -> Vec<u8> {
    let mut document = load_html_with_base(composition_html, width, height, base_url);
    let t_secs = frame as f64 / fps.max(1) as f64;
    document.as_mut().resolve(t_secs);
    render_doc_rgba(document.as_mut(), width, height)
}

/// Self-contained variant (no asset base dir).
pub fn render_frame_rgba(
    composition_html: &str,
    frame: u32,
    fps: u32,
    width: u32,
    height: u32,
) -> Vec<u8> {
    render_frame_rgba_with_base(composition_html, frame, fps, width, height, None)
}

/// Self-contained variant, PNG-encoded.
pub fn render_frame(
    composition_html: &str,
    frame: u32,
    fps: u32,
    width: u32,
    height: u32,
) -> Vec<u8> {
    let rgba = render_frame_rgba(composition_html, frame, fps, width, height);
    rgba_to_png(&rgba, width, height)
}

/// Load a composition **from a file path** (WASI `std::fs`), with its
/// directory wired as the asset base. Returns `(html, base_url)`.
pub fn read_composition(composition_path: &Path) -> std::io::Result<(String, Option<String>)> {
    let html = std::fs::read_to_string(composition_path)?;
    let base = dir_base_url(composition_path);
    Ok((html, base))
}

/// Render a single frame of a composition loaded from a file path. Relative
/// `<img>` assets resolve against the composition's directory.
pub fn render_frame_from_path(
    composition_path: &Path,
    frame: u32,
    fps: u32,
    width: u32,
    height: u32,
) -> std::io::Result<Vec<u8>> {
    let (html, base) = read_composition(composition_path)?;
    let rgba = render_frame_rgba_with_base(&html, frame, fps, width, height, base);
    Ok(rgba_to_png(&rgba, width, height))
}

/// Render a deterministic **frame sequence** of a composition file to a
/// directory of PNGs: `frame_00000.png`, `frame_00001.png`, ...
///
/// Total frames = `round(fps * duration_secs)` (frame indices `0..N`). The
/// composition is parsed **once per frame** (the spec asks for determinism,
/// not speed; reusing a document would require re-seeding the animation
/// clock, and a fresh parse is the simplest provably-deterministic path).
/// Returns the number of frames written.
pub fn render_sequence_to_dir(
    composition_path: &Path,
    out_dir: &Path,
    fps: u32,
    duration_secs: f64,
    width: u32,
    height: u32,
) -> std::io::Result<u32> {
    let (html, base) = read_composition(composition_path)?;
    std::fs::create_dir_all(out_dir)?;
    let fps = fps.max(1);
    let n = (fps as f64 * duration_secs).round() as u32;
    for frame in 0..n {
        let rgba = render_frame_rgba_with_base(&html, frame, fps, width, height, base.clone());
        let png = rgba_to_png(&rgba, width, height);
        let path = out_dir.join(format!("frame_{frame:05}.png"));
        std::fs::write(&path, &png)?;
    }
    Ok(n)
}

/// Encode an RGBA8 buffer to PNG bytes.
pub fn rgba_to_png(rgba: &[u8], width: u32, height: u32) -> Vec<u8> {
    let mut out = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut out, width, height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().expect("png header");
        writer.write_image_data(rgba).expect("png data");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCENE: &str = r#"<!doctype html><html><head><style>
      html,body{margin:0;padding:0;width:100%;height:100%;background:#0b1020;overflow:hidden}
      #box{position:absolute;top:200px;width:240px;height:240px;background:#3fe081;
           animation:slide 2s linear forwards}
      @keyframes slide{from{left:80px}to{left:1400px}}
      h1{position:absolute;left:80px;top:80px;font-size:64px;color:#fff;font-family:sans-serif}
    </style></head><body><h1>Wavelet in the nexus</h1><div id="box"></div></body></html>"#;

    #[test]
    fn renders_nonblank_and_animates() {
        let (w, h) = (640u32, 360u32);
        let a = render_frame_rgba(SCENE, 0, 30, w, h);
        let b = render_frame_rgba(SCENE, 45, 30, w, h); // t=1.5s, box moved
        assert_eq!(a.len(), (w * h * 4) as usize);
        assert!(a.iter().any(|&p| p != 255), "frame 0 is all-white");
        assert!(a != b, "frame 0 and frame 45 identical — timeline not advancing");
    }

    /// A 1x1 red PNG as a data: URI must decode + paint (proves the
    /// FileNetProvider data: path independent of the filesystem).
    #[test]
    fn data_uri_image_paints() {
        // 1x1 opaque red PNG.
        let data_uri = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP4z8DwHwAFAAH/VscvDQAAAABJRU5ErkJggg==";
        let html = format!(
            r#"<!doctype html><html><head><style>
            html,body{{margin:0;background:#fff}}
            img{{position:absolute;left:0;top:0;width:200px;height:200px}}
            </style></head><body><img src="{data_uri}"></body></html>"#
        );
        let (w, h) = (256u32, 256u32);
        let rgba = render_frame_rgba(&html, 0, 30, w, h);
        // Sample a pixel well inside the 200x200 image box (the very corner is
        // antialiased / edge-blended). (100,100) is squarely on the image.
        let idx = ((100 * w + 100) * 4) as usize;
        let r = rgba[idx];
        let g = rgba[idx + 1];
        let b = rgba[idx + 2];
        assert!(
            r > 180 && g < 100 && b < 100,
            "expected red image pixel, got ({r},{g},{b}) — data: image did not paint"
        );
    }
}
