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
pub mod js;
use blitz_dom::node::NodeData;
use blitz_dom::{build_single_font_ctx, BaseDocument, DocumentConfig, LocalName};
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
            // The real HTML parser provider so `set_inner_html` (the JS DOM layer) works — without
            // it the doc defaults to DummyHtmlParserProvider and inner-HTML mutations are no-ops.
            html_parser_provider: Some(Arc::new(blitz_html::HtmlProvider)),
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

// ── still-image + animated-GIF encode lane (in-guest, pure-Rust `image`) ──────
//
// The render-core paints to RGBA8; the `image` crate (already a dep for `<img>`
// decode) re-encodes that to the still formats and to an animated GIF over the
// frame *sequence* — NO ffmpeg, NO native exec. Everything below links under
// wasm32-wasip1 and runs under wasmtime.

use image::codecs::gif::{GifEncoder, Repeat};
use image::{ImageFormat, RgbaImage};
use std::io::Cursor;

/// The still-image output formats render_media can emit. GIF is handled by the
/// animated-sequence path, not here.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StillFormat {
    Png,
    Jpeg,
    Webp,
}

impl StillFormat {
    /// Parse a `--format` token. `gif` is intentionally NOT a still format.
    pub fn parse(s: &str) -> Option<StillFormat> {
        match s.to_ascii_lowercase().as_str() {
            "png" => Some(StillFormat::Png),
            "jpeg" | "jpg" => Some(StillFormat::Jpeg),
            "webp" => Some(StillFormat::Webp),
            _ => None,
        }
    }

    /// Conventional file extension (no dot).
    pub fn ext(self) -> &'static str {
        match self {
            StillFormat::Png => "png",
            StillFormat::Jpeg => "jpg",
            StillFormat::Webp => "webp",
        }
    }
}

/// Wrap an RGBA8 buffer in an `image::RgbaImage` (validates the length).
fn rgba_image(rgba: &[u8], width: u32, height: u32) -> RgbaImage {
    RgbaImage::from_raw(width, height, rgba.to_vec())
        .expect("rgba buffer length != width*height*4")
}

/// Encode an RGBA8 buffer to JPEG bytes. JPEG has no alpha channel, so the
/// buffer is flattened to RGB8 (alpha dropped) before encoding.
pub fn rgba_to_jpeg(rgba: &[u8], width: u32, height: u32) -> Vec<u8> {
    let dynimg = image::DynamicImage::ImageRgba8(rgba_image(rgba, width, height));
    let rgb = dynimg.to_rgb8(); // drop alpha — JPEG is opaque
    let mut out = Cursor::new(Vec::new());
    image::DynamicImage::ImageRgb8(rgb)
        .write_to(&mut out, ImageFormat::Jpeg)
        .expect("jpeg encode");
    out.into_inner()
}

/// Encode an RGBA8 buffer to WebP bytes (lossless, alpha preserved).
pub fn rgba_to_webp(rgba: &[u8], width: u32, height: u32) -> Vec<u8> {
    let dynimg = image::DynamicImage::ImageRgba8(rgba_image(rgba, width, height));
    let mut out = Cursor::new(Vec::new());
    dynimg
        .write_to(&mut out, ImageFormat::WebP)
        .expect("webp encode");
    out.into_inner()
}

/// Encode an RGBA8 buffer to the chosen still format.
pub fn rgba_to_still(rgba: &[u8], width: u32, height: u32, fmt: StillFormat) -> Vec<u8> {
    match fmt {
        StillFormat::Png => rgba_to_png(rgba, width, height),
        StillFormat::Jpeg => rgba_to_jpeg(rgba, width, height),
        StillFormat::Webp => rgba_to_webp(rgba, width, height),
    }
}

/// Render a single composition frame from a file path to the chosen still
/// format. Relative `<img>` assets resolve against the composition's directory.
pub fn render_still_from_path(
    composition_path: &Path,
    frame: u32,
    fps: u32,
    width: u32,
    height: u32,
    fmt: StillFormat,
) -> std::io::Result<Vec<u8>> {
    let (html, base) = read_composition(composition_path)?;
    let rgba = render_frame_rgba_with_base(&html, frame, fps, width, height, base);
    Ok(rgba_to_still(&rgba, width, height, fmt))
}

/// Render an ANIMATED GIF of the WHOLE composition from a file path.
///
/// Paints frames `0..N` (N = `round(fps * duration_secs)`), each as one GIF
/// frame, with the per-frame delay derived from `fps` (1/fps seconds), and
/// loops forever. The GIF is built entirely in-guest via `image`'s
/// `GifEncoder` — NO ffmpeg. Returns the GIF bytes.
///
/// GIF is a 256-colour palette format; `image`'s encoder quantizes each frame's
/// RGBA8 to that palette (alpha is binary). This is the lossy cost of GIF — for
/// full-colour motion use the mp4 encode lane. Frame delay is centiseconds
/// internally (GIF's unit); `image` derives that from the `Delay` we pass.
pub fn render_animated_gif_from_path(
    composition_path: &Path,
    fps: u32,
    duration_secs: f64,
    width: u32,
    height: u32,
) -> std::io::Result<Vec<u8>> {
    use image::{Delay, Frame};

    let (html, base) = read_composition(composition_path)?;
    let fps = fps.max(1);
    let n = (fps as f64 * duration_secs).round().max(1.0) as u32;

    // Per-frame delay = 1/fps second, expressed as a rational in milliseconds so
    // the GIF clock matches the render fps (image rounds to GIF's centisecond grid).
    let delay = Delay::from_numer_denom_ms(1000, fps);

    let mut out = Cursor::new(Vec::new());
    {
        let mut encoder = GifEncoder::new(&mut out);
        // Loop forever — the whole composition plays on repeat.
        encoder
            .set_repeat(Repeat::Infinite)
            .expect("gif set_repeat");

        for frame in 0..n {
            let rgba = render_frame_rgba_with_base(&html, frame, fps, width, height, base.clone());
            let img = rgba_image(&rgba, width, height);
            let gframe = Frame::from_parts(img, 0, 0, delay);
            encoder.encode_frame(gframe).expect("gif encode_frame");
        }
        // encoder dropped here — flushes the trailer into `out`.
    }
    Ok(out.into_inner())
}

// ── presentation (.pptx) export lane — IN-GUEST, pure-Rust OOXML ──────────────
//
// A multi-scene composition becomes a PowerPoint deck: ONE slide per scene,
// each slide a full-bleed picture of that scene rendered by the render-core.
// The .pptx is a ZIP of OOXML parts ([Content_Types].xml, _rels/.rels,
// ppt/presentation.xml + rels, one ppt/slides/slideN.xml + rels per scene, a
// minimal slideLayout + slideMaster, and ppt/media/imageN.png). Built with the
// pure-Rust `zip` crate (flate2/miniz_oxide backend) — NO native exec, links
// under wasm32-wasip1, runs under wasmtime.
//
// "Scene" convention (string-level, no DOM dependency):
//   1. elements carrying a `data-scene` attribute (any tag) — preferred;
//   2. else top-level `<section>` elements;
//   3. else the whole composition is a single scene.
// Each scene is rendered by re-rendering the WHOLE composition with an injected
// stylesheet that hides every scene element except the i-th and makes the
// visible one full-bleed, then painting the frame. This reuses the exact Blitz
// /Stylo/Vello pipeline (styles, fonts, assets, CSS timeline) verbatim.

use std::io::Write as _;

/// EMU (English Metric Units) per inch — OOXML's absolute unit. 914400 EMU = 1".
const EMU_PER_INCH: i64 = 914_400;

/// How scenes are delimited inside a composition.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SceneKind {
    /// `data-scene`-attributed elements.
    DataAttr,
    /// `<section>` elements.
    Section,
    /// The whole document is one scene.
    Whole,
}

/// Detect the scene kind + count of scenes in a composition's HTML. `data-scene`
/// wins; else `<section>`; else the whole doc (count 1). Counting is a tag/attr
/// scan that ignores `<script>`/`<style>`/comment contents so CSS or JS that
/// merely mentions the token isn't miscounted.
fn detect_scenes(html: &str) -> (SceneKind, usize) {
    let body = strip_non_markup(html);
    let data_n = count_data_scene_opening_tags(&body);
    if data_n > 0 {
        return (SceneKind::DataAttr, data_n);
    }
    let section_n = count_opening_tags(&body, "section");
    if section_n > 0 {
        return (SceneKind::Section, section_n);
    }
    (SceneKind::Whole, 1)
}

/// Remove `<!-- -->` comments and the *contents* of `<script>`/`<style>` so a
/// scene-token mention inside CSS/JS doesn't get miscounted as a scene element.
/// Keeps the surrounding markup intact. Operates on the lowercased copy used for
/// counting (we only need positions/structure, not original-case text).
fn strip_non_markup(html: &str) -> String {
    let lower = html.to_ascii_lowercase();
    let mut out = String::with_capacity(lower.len());
    let mut rest: &str = &lower;
    'outer: loop {
        // Comment?
        if let Some(stripped) = rest.strip_prefix("<!--") {
            match stripped.find("-->") {
                Some(end) => {
                    rest = &stripped[end + 3..];
                    continue 'outer;
                }
                None => break,
            }
        }
        // <script> / <style> — skip their inner content.
        for tag in ["script", "style"] {
            let open = format!("<{tag}");
            if rest.starts_with(&open) {
                match rest.find('>') {
                    Some(gt) => {
                        out.push_str(&rest[..=gt]); // opening tag
                        let after = &rest[gt + 1..];
                        let close = format!("</{tag}");
                        match after.find(&close) {
                            Some(rel) => {
                                let close_part = &after[rel..];
                                match close_part.find('>') {
                                    Some(cgt) => {
                                        out.push_str(&close_part[..=cgt]); // closing tag
                                        rest = &close_part[cgt + 1..];
                                    }
                                    None => {
                                        rest = "";
                                    }
                                }
                            }
                            None => {
                                rest = "";
                            }
                        }
                    }
                    None => {
                        rest = "";
                    }
                }
                continue 'outer;
            }
        }
        // Default: copy one char.
        match rest.chars().next() {
            Some(c) => {
                out.push(c);
                rest = &rest[c.len_utf8()..];
            }
            None => break,
        }
    }
    out
}

/// Count opening tags `<name ...>` / `<name>` (not closing `</name>`), case-
/// insensitive, requiring a word boundary after the name.
fn count_opening_tags(html: &str, name: &str) -> usize {
    let lower = html.to_ascii_lowercase();
    let needle = format!("<{name}");
    let bytes = lower.as_bytes();
    let mut count = 0;
    let mut from = 0;
    while let Some(rel) = lower[from..].find(&needle) {
        let pos = from + rel;
        let after = pos + needle.len();
        // boundary: next char must be whitespace, '>' or '/'
        let ok = bytes
            .get(after)
            .map(|&c| c == b' ' || c == b'>' || c == b'/' || c == b'\t' || c == b'\n' || c == b'\r')
            .unwrap_or(false);
        if ok {
            count += 1;
        }
        from = after;
    }
    count
}

/// Count opening tags (any name) that carry a `data-scene` attribute. Scans for
/// `data-scene` occurrences that sit inside an opening tag.
fn count_data_scene_opening_tags(html: &str) -> usize {
    let lower = html.to_ascii_lowercase();
    let mut count = 0;
    let mut from = 0;
    while let Some(rel) = lower[from..].find("data-scene") {
        let pos = from + rel;
        // walk back to the nearest '<' and forward to '>' to confirm we're
        // inside an opening (non-closing) tag.
        if let Some(lt) = lower[..pos].rfind('<') {
            let is_closing = lower.as_bytes().get(lt + 1) == Some(&b'/');
            let gt = lower[pos..].find('>').map(|r| pos + r);
            if !is_closing && gt.is_some() {
                count += 1;
            }
        }
        from = pos + "data-scene".len();
    }
    count
}

/// CSS that isolates the scene marked `data-wavelet-scene="index"` and makes it
/// the only visible, full-bleed content. Every scene element carries a
/// `data-wavelet-scene` ordinal (injected by [`annotate_scenes`]) so isolation
/// works regardless of whether scenes are the same tag. For `SceneKind::Whole`
/// no isolation is needed (the whole doc IS the scene).
fn scene_isolation_css(kind: SceneKind, index: usize, _total: usize) -> String {
    if kind == SceneKind::Whole {
        return String::new();
    }
    // Hide every scene element; reveal + full-bleed the target. `!important`
    // so author rules can't re-hide it. Position it to cover the viewport.
    format!(
        "<style id=\"__wavelet_scene_iso\">\n\
         [data-wavelet-scene]{{display:none !important;}}\n\
         [data-wavelet-scene=\"{index}\"]{{display:block !important;position:absolute !important;\
         left:0 !important;top:0 !important;right:0 !important;bottom:0 !important;\
         width:100% !important;height:100% !important;margin:0 !important;}}\n\
         </style>"
    )
}

/// Annotate each scene element with `data-wavelet-scene="<ordinal>"` (0-based) so
/// CSS isolation can target any scene by ordinal regardless of tag. Rewrites the
/// opening tag of every scene element in document order. Returns the rewritten
/// HTML. For `SceneKind::Whole` the HTML is returned unchanged.
fn annotate_scenes(html: &str, kind: SceneKind) -> String {
    match kind {
        SceneKind::Whole => html.to_string(),
        SceneKind::DataAttr => annotate_by(html, &|lower, lt| {
            // Opening tag whose attributes (up to '>') contain `data-scene`.
            tag_attrs_contain(lower, lt, "data-scene")
        }),
        SceneKind::Section => annotate_by(html, &|lower, lt| {
            // Opening <section ...> tag.
            tag_name_is(lower, lt, "section")
        }),
    }
}

/// True if the opening tag starting at `lt` (a '<') has tag-name `name`.
fn tag_name_is(lower: &str, lt: usize, name: &str) -> bool {
    let after = &lower[lt + 1..];
    if after.starts_with('/') {
        return false; // closing tag
    }
    let open = name;
    if !after.starts_with(open) {
        return false;
    }
    // boundary char after the name
    after[open.len()..]
        .chars()
        .next()
        .map(|c| c == ' ' || c == '>' || c == '/' || c == '\t' || c == '\n' || c == '\r')
        .unwrap_or(false)
}

/// True if the opening tag starting at `lt` contains attribute substring `attr`
/// before its closing '>'. Skips closing tags.
fn tag_attrs_contain(lower: &str, lt: usize, attr: &str) -> bool {
    let after = &lower[lt + 1..];
    if after.starts_with('/') {
        return false;
    }
    match after.find('>') {
        Some(gt) => after[..gt].contains(attr),
        None => false,
    }
}

/// Walk the HTML, and for each opening tag where `pred(lower, lt)` is true,
/// inject ` data-wavelet-scene="<ordinal>"` right after the tag name. Ordinals
/// increment in document order. Operates on original-case `html` (matches the
/// lowercased copy by byte offset — ASCII-lowercasing preserves byte lengths).
fn annotate_by(html: &str, pred: &dyn Fn(&str, usize) -> bool) -> String {
    let lower = html.to_ascii_lowercase();
    let mut out = String::with_capacity(html.len() + 64);
    let mut ordinal = 0usize;
    let mut i = 0usize;
    let bytes = lower.as_bytes();
    while i < html.len() {
        if bytes[i] == b'<' && pred(&lower, i) {
            // Find end of the tag name (after '<'): up to first whitespace/ '>' / '/'.
            let name_start = i + 1;
            let mut j = name_start;
            while j < html.len() {
                let c = bytes[j];
                if c == b' ' || c == b'>' || c == b'/' || c == b'\t' || c == b'\n' || c == b'\r' {
                    break;
                }
                j += 1;
            }
            out.push_str(&html[i..j]); // '<' + tag name
            out.push_str(&format!(" data-wavelet-scene=\"{ordinal}\""));
            ordinal += 1;
            i = j;
        } else {
            // copy one UTF-8 char
            let ch = html[i..].chars().next().unwrap();
            out.push(ch);
            i += ch.len_utf8();
        }
    }
    out
}

/// Build the per-scene HTML by annotating scene elements then injecting the
/// isolation `<style>` before `</head>`. If there's no `</head>`, inject right
/// after `<body>`/`<html>` or prepend. The original composition styles, fonts,
/// assets and timeline are all preserved — only visibility changes.
fn compose_scene_html(html: &str, kind: SceneKind, index: usize, total: usize) -> String {
    let css = scene_isolation_css(kind, index, total);
    if css.is_empty() {
        return html.to_string();
    }
    let html = annotate_scenes(html, kind);
    let lower = html.to_ascii_lowercase();
    if let Some(pos) = lower.find("</head>") {
        let mut out = String::with_capacity(html.len() + css.len());
        out.push_str(&html[..pos]);
        out.push_str(&css);
        out.push_str(&html[pos..]);
        out
    } else if let Some(pos) = lower.find("<body") {
        // insert after the <body ...> opening tag's '>'
        if let Some(rel) = lower[pos..].find('>') {
            let at = pos + rel + 1;
            let mut out = String::with_capacity(html.len() + css.len());
            out.push_str(&html[..at]);
            out.push_str(&css);
            out.push_str(&html[at..]);
            out
        } else {
            format!("{css}{html}")
        }
    } else {
        format!("{css}{html}")
    }
}

/// Render one scene of a composition to RGBA8 by isolating it and painting the
/// frame at `t = frame/fps`. `base_url` resolves relative `<img>` assets.
fn render_scene_rgba(
    html: &str,
    kind: SceneKind,
    index: usize,
    total: usize,
    frame: u32,
    fps: u32,
    width: u32,
    height: u32,
    base_url: Option<String>,
) -> Vec<u8> {
    let scene_html = compose_scene_html(html, kind, index, total);
    render_frame_rgba_with_base(&scene_html, frame, fps, width, height, base_url)
}

/// Number of scenes in a composition file (reads the file). Useful for callers
/// that want to size a deck before rendering.
pub fn count_scenes_in_path(composition_path: &Path) -> std::io::Result<usize> {
    let html = std::fs::read_to_string(composition_path)?;
    Ok(detect_scenes(&html).1)
}

/// Render every scene of a composition file to PNG bytes, one per slide.
///
/// Returns `Vec<png_bytes>` in scene order. `frame`/`fps` seek the CSS timeline
/// (a still snapshot per slide). Relative `<img>` assets resolve against the
/// composition's directory.
pub fn render_scene_pngs_from_path(
    composition_path: &Path,
    frame: u32,
    fps: u32,
    width: u32,
    height: u32,
) -> std::io::Result<Vec<Vec<u8>>> {
    let (html, base) = read_composition(composition_path)?;
    let (kind, total) = detect_scenes(&html);
    let mut pngs = Vec::with_capacity(total);
    for i in 0..total {
        let rgba = render_scene_rgba(&html, kind, i, total, frame, fps, width, height, base.clone());
        pngs.push(rgba_to_png(&rgba, width, height));
    }
    Ok(pngs)
}

/// Assemble a valid OOXML `.pptx` deck from per-scene PNG bytes — one full-bleed
/// picture slide per PNG. Slide size is derived from the pixel `width`/`height`
/// at 96 DPI (the OOXML default), so the picture fills the slide exactly.
///
/// The archive contains: `[Content_Types].xml`, `_rels/.rels`,
/// `ppt/presentation.xml` (+ rels), `ppt/slideMasters/slideMaster1.xml` (+rels),
/// `ppt/slideLayouts/slideLayout1.xml` (+rels), `ppt/theme/theme1.xml`, one
/// `ppt/slides/slideN.xml` (+rels) per scene, and `ppt/media/imageN.png`.
/// Built with the pure-Rust `zip` crate — opens in PowerPoint / Keynote /
/// LibreOffice Impress / Google Slides.
pub fn assemble_pptx(slide_pngs: &[Vec<u8>], width: u32, height: u32) -> std::io::Result<Vec<u8>> {
    use zip::write::SimpleFileOptions;
    use zip::{CompressionMethod, ZipWriter};

    let n = slide_pngs.len().max(1);
    // Slide dimensions in EMU from pixels at 96 DPI.
    let cx = (width as i64) * EMU_PER_INCH / 96;
    let cy = (height as i64) * EMU_PER_INCH / 96;

    let cursor = Cursor::new(Vec::new());
    let mut zip = ZipWriter::new(cursor);
    // Deterministic: fixed mtime, deflate. (Determinism keeps the export
    // reproducible — same scenes => same bytes.)
    let deflate = SimpleFileOptions::default()
        .compression_method(CompressionMethod::Deflated)
        .last_modified_time(zip::DateTime::default());
    // PNG is already compressed; store the media to avoid double work.
    let stored = SimpleFileOptions::default()
        .compression_method(CompressionMethod::Stored)
        .last_modified_time(zip::DateTime::default());

    let put = |zip: &mut ZipWriter<Cursor<Vec<u8>>>,
                   name: &str,
                   data: &[u8],
                   opts: SimpleFileOptions|
     -> std::io::Result<()> {
        zip.start_file(name, opts)
            .map_err(|e| std::io::Error::other(e.to_string()))?;
        zip.write_all(data)?;
        Ok(())
    };

    // [Content_Types].xml
    put(&mut zip, "[Content_Types].xml", content_types_xml(n).as_bytes(), deflate)?;
    // _rels/.rels
    put(&mut zip, "_rels/.rels", root_rels_xml().as_bytes(), deflate)?;
    // theme
    put(&mut zip, "ppt/theme/theme1.xml", theme1_xml().as_bytes(), deflate)?;
    // presentation + rels
    put(&mut zip, "ppt/presentation.xml", presentation_xml(n, cx, cy).as_bytes(), deflate)?;
    put(&mut zip, "ppt/_rels/presentation.xml.rels", presentation_rels_xml(n).as_bytes(), deflate)?;
    // master + rels
    put(&mut zip, "ppt/slideMasters/slideMaster1.xml", slide_master_xml().as_bytes(), deflate)?;
    put(
        &mut zip,
        "ppt/slideMasters/_rels/slideMaster1.xml.rels",
        slide_master_rels_xml().as_bytes(),
        deflate,
    )?;
    // layout + rels
    put(&mut zip, "ppt/slideLayouts/slideLayout1.xml", slide_layout_xml().as_bytes(), deflate)?;
    put(
        &mut zip,
        "ppt/slideLayouts/_rels/slideLayout1.xml.rels",
        slide_layout_rels_xml().as_bytes(),
        deflate,
    )?;

    // slides + per-slide rels + media
    for i in 0..n {
        let idx = i + 1;
        let png = slide_pngs.get(i).map(|v| v.as_slice()).unwrap_or(&[]);
        put(
            &mut zip,
            &format!("ppt/slides/slide{idx}.xml"),
            slide_xml(idx, cx, cy).as_bytes(),
            deflate,
        )?;
        put(
            &mut zip,
            &format!("ppt/slides/_rels/slide{idx}.xml.rels"),
            slide_rels_xml(idx).as_bytes(),
            deflate,
        )?;
        put(&mut zip, &format!("ppt/media/image{idx}.png"), png, stored)?;
    }

    let cursor = zip.finish().map_err(|e| std::io::Error::other(e.to_string()))?;
    Ok(cursor.into_inner())
}

/// End-to-end: render a multi-scene composition file to a `.pptx` byte buffer
/// (one full-bleed picture slide per scene). `frame`/`fps` seek the CSS timeline
/// for the still snapshot painted onto each slide.
pub fn render_pptx_from_path(
    composition_path: &Path,
    frame: u32,
    fps: u32,
    width: u32,
    height: u32,
) -> std::io::Result<Vec<u8>> {
    let pngs = render_scene_pngs_from_path(composition_path, frame, fps, width, height)?;
    assemble_pptx(&pngs, width, height)
}

// ── OOXML part templates ──────────────────────────────────────────────────────

fn content_types_xml(n: usize) -> String {
    let mut slides = String::new();
    for i in 1..=n {
        slides.push_str(&format!(
            "<Override PartName=\"/ppt/slides/slide{i}.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.presentationml.slide+xml\"/>"
        ));
    }
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\">\
<Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/>\
<Default Extension=\"xml\" ContentType=\"application/xml\"/>\
<Default Extension=\"png\" ContentType=\"image/png\"/>\
<Override PartName=\"/ppt/presentation.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.presentationml.presentation.main+xml\"/>\
<Override PartName=\"/ppt/slideMasters/slideMaster1.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.presentationml.slideMaster+xml\"/>\
<Override PartName=\"/ppt/slideLayouts/slideLayout1.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.presentationml.slideLayout+xml\"/>\
<Override PartName=\"/ppt/theme/theme1.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.theme+xml\"/>\
{slides}\
</Types>"
    )
}

fn root_rels_xml() -> String {
    "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\
<Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument\" Target=\"ppt/presentation.xml\"/>\
</Relationships>"
        .to_string()
}

fn presentation_xml(n: usize, cx: i64, cy: i64) -> String {
    // Master is rId1; slides start at rId2.
    let mut sldid = String::new();
    for i in 0..n {
        let rid = i + 2;
        let id = 256 + i; // sldId values must be >= 256
        sldid.push_str(&format!("<p:sldId id=\"{id}\" r:id=\"rId{rid}\"/>"));
    }
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
<p:presentation xmlns:a=\"http://schemas.openxmlformats.org/drawingml/2006/main\" \
xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\" \
xmlns:p=\"http://schemas.openxmlformats.org/presentationml/2006/main\">\
<p:sldMasterIdLst><p:sldMasterId id=\"2147483648\" r:id=\"rId1\"/></p:sldMasterIdLst>\
<p:sldIdLst>{sldid}</p:sldIdLst>\
<p:sldSz cx=\"{cx}\" cy=\"{cy}\"/>\
<p:notesSz cx=\"{cy}\" cy=\"{cx}\"/>\
</p:presentation>"
    )
}

fn presentation_rels_xml(n: usize) -> String {
    let mut rels = String::new();
    // rId1 = master
    rels.push_str(
        "<Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideMaster\" Target=\"slideMasters/slideMaster1.xml\"/>",
    );
    // rId2.. = slides
    for i in 0..n {
        let rid = i + 2;
        let idx = i + 1;
        rels.push_str(&format!(
            "<Relationship Id=\"rId{rid}\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/slide\" Target=\"slides/slide{idx}.xml\"/>"
        ));
    }
    // theme rel after slides
    let theme_rid = n + 2;
    rels.push_str(&format!(
        "<Relationship Id=\"rId{theme_rid}\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/theme\" Target=\"theme/theme1.xml\"/>"
    ));
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">{rels}</Relationships>"
    )
}

fn slide_xml(idx: usize, cx: i64, cy: i64) -> String {
    // A single full-bleed picture filling the slide (off=0, ext=slide size).
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
<p:sld xmlns:a=\"http://schemas.openxmlformats.org/drawingml/2006/main\" \
xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\" \
xmlns:p=\"http://schemas.openxmlformats.org/presentationml/2006/main\">\
<p:cSld><p:spTree>\
<p:nvGrpSpPr><p:cNvPr id=\"1\" name=\"\"/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr>\
<p:grpSpPr><a:xfrm><a:off x=\"0\" y=\"0\"/><a:ext cx=\"{cx}\" cy=\"{cy}\"/>\
<a:chOff x=\"0\" y=\"0\"/><a:chExt cx=\"{cx}\" cy=\"{cy}\"/></a:xfrm></p:grpSpPr>\
<p:pic>\
<p:nvPicPr><p:cNvPr id=\"{pic_id}\" name=\"Scene {idx}\"/>\
<p:cNvPicPr><a:picLocks noChangeAspect=\"1\"/></p:cNvPicPr><p:nvPr/></p:nvPicPr>\
<p:blipFill><a:blip r:embed=\"rId1\"/><a:stretch><a:fillRect/></a:stretch></p:blipFill>\
<p:spPr><a:xfrm><a:off x=\"0\" y=\"0\"/><a:ext cx=\"{cx}\" cy=\"{cy}\"/></a:xfrm>\
<a:prstGeom prst=\"rect\"><a:avLst/></a:prstGeom></p:spPr>\
</p:pic>\
</p:spTree></p:cSld><p:clrMapOvr><a:overrideClrMapping/></p:clrMapOvr></p:sld>",
        pic_id = idx + 1,
    )
}

fn slide_rels_xml(idx: usize) -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\
<Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/image\" Target=\"../media/image{idx}.png\"/>\
<Relationship Id=\"rId2\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideLayout\" Target=\"../slideLayouts/slideLayout1.xml\"/>\
</Relationships>"
    )
}

fn slide_master_xml() -> String {
    "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
<p:sldMaster xmlns:a=\"http://schemas.openxmlformats.org/drawingml/2006/main\" \
xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\" \
xmlns:p=\"http://schemas.openxmlformats.org/presentationml/2006/main\">\
<p:cSld><p:bg><p:bgPr><a:solidFill><a:srgbClr val=\"FFFFFF\"/></a:solidFill>\
<a:effectLst/></p:bgPr></p:bg>\
<p:spTree>\
<p:nvGrpSpPr><p:cNvPr id=\"1\" name=\"\"/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr>\
<p:grpSpPr/></p:spTree></p:cSld>\
<p:clrMap bg1=\"lt1\" tx1=\"dk1\" bg2=\"lt2\" tx2=\"dk2\" accent1=\"accent1\" accent2=\"accent2\" \
accent3=\"accent3\" accent4=\"accent4\" accent5=\"accent5\" accent6=\"accent6\" hlink=\"hlink\" folHlink=\"folHlink\"/>\
<p:sldLayoutIdLst><p:sldLayoutId id=\"2147483649\" r:id=\"rId1\"/></p:sldLayoutIdLst>\
</p:sldMaster>"
        .to_string()
}

fn slide_master_rels_xml() -> String {
    "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\
<Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideLayout\" Target=\"../slideLayouts/slideLayout1.xml\"/>\
<Relationship Id=\"rId2\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/theme\" Target=\"../theme/theme1.xml\"/>\
</Relationships>"
        .to_string()
}

fn slide_layout_xml() -> String {
    "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
<p:sldLayout xmlns:a=\"http://schemas.openxmlformats.org/drawingml/2006/main\" \
xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\" \
xmlns:p=\"http://schemas.openxmlformats.org/presentationml/2006/main\" type=\"blank\" preserve=\"1\">\
<p:cSld name=\"Blank\"><p:spTree>\
<p:nvGrpSpPr><p:cNvPr id=\"1\" name=\"\"/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr>\
<p:grpSpPr/></p:spTree></p:cSld>\
<p:clrMapOvr><a:overrideClrMapping bg1=\"lt1\" tx1=\"dk1\" bg2=\"lt2\" tx2=\"dk2\" accent1=\"accent1\" \
accent2=\"accent2\" accent3=\"accent3\" accent4=\"accent4\" accent5=\"accent5\" accent6=\"accent6\" \
hlink=\"hlink\" folHlink=\"folHlink\"/></p:clrMapOvr>\
</p:sldLayout>"
        .to_string()
}

fn slide_layout_rels_xml() -> String {
    "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\
<Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideMaster\" Target=\"../slideMasters/slideMaster1.xml\"/>\
</Relationships>"
        .to_string()
}

fn theme1_xml() -> String {
    // A minimal but schema-valid Office theme (one clrScheme/fontScheme/fmtScheme).
    "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
<a:theme xmlns:a=\"http://schemas.openxmlformats.org/drawingml/2006/main\" name=\"Office Theme\">\
<a:themeElements>\
<a:clrScheme name=\"Office\">\
<a:dk1><a:sysClr val=\"windowText\" lastClr=\"000000\"/></a:dk1>\
<a:lt1><a:sysClr val=\"window\" lastClr=\"FFFFFF\"/></a:lt1>\
<a:dk2><a:srgbClr val=\"44546A\"/></a:dk2>\
<a:lt2><a:srgbClr val=\"E7E6E6\"/></a:lt2>\
<a:accent1><a:srgbClr val=\"4472C4\"/></a:accent1>\
<a:accent2><a:srgbClr val=\"ED7D31\"/></a:accent2>\
<a:accent3><a:srgbClr val=\"A5A5A5\"/></a:accent3>\
<a:accent4><a:srgbClr val=\"FFC000\"/></a:accent4>\
<a:accent5><a:srgbClr val=\"5B9BD5\"/></a:accent5>\
<a:accent6><a:srgbClr val=\"70AD47\"/></a:accent6>\
<a:hlink><a:srgbClr val=\"0563C1\"/></a:hlink>\
<a:folHlink><a:srgbClr val=\"954F72\"/></a:folHlink>\
</a:clrScheme>\
<a:fontScheme name=\"Office\">\
<a:majorFont><a:latin typeface=\"Calibri Light\"/><a:ea typeface=\"\"/><a:cs typeface=\"\"/></a:majorFont>\
<a:minorFont><a:latin typeface=\"Calibri\"/><a:ea typeface=\"\"/><a:cs typeface=\"\"/></a:minorFont>\
</a:fontScheme>\
<a:fmtScheme name=\"Office\">\
<a:fillStyleLst>\
<a:solidFill><a:schemeClr val=\"phClr\"/></a:solidFill>\
<a:solidFill><a:schemeClr val=\"phClr\"/></a:solidFill>\
<a:solidFill><a:schemeClr val=\"phClr\"/></a:solidFill>\
</a:fillStyleLst>\
<a:lnStyleLst>\
<a:ln w=\"6350\" cap=\"flat\" cmpd=\"sng\" algn=\"ctr\"><a:solidFill><a:schemeClr val=\"phClr\"/></a:solidFill><a:prstDash val=\"solid\"/></a:ln>\
<a:ln w=\"12700\" cap=\"flat\" cmpd=\"sng\" algn=\"ctr\"><a:solidFill><a:schemeClr val=\"phClr\"/></a:solidFill><a:prstDash val=\"solid\"/></a:ln>\
<a:ln w=\"19050\" cap=\"flat\" cmpd=\"sng\" algn=\"ctr\"><a:solidFill><a:schemeClr val=\"phClr\"/></a:solidFill><a:prstDash val=\"solid\"/></a:ln>\
</a:lnStyleLst>\
<a:effectStyleLst>\
<a:effectStyle><a:effectLst/></a:effectStyle>\
<a:effectStyle><a:effectLst/></a:effectStyle>\
<a:effectStyle><a:effectLst/></a:effectStyle>\
</a:effectStyleLst>\
<a:bgFillStyleLst>\
<a:solidFill><a:schemeClr val=\"phClr\"/></a:solidFill>\
<a:solidFill><a:schemeClr val=\"phClr\"/></a:solidFill>\
<a:solidFill><a:schemeClr val=\"phClr\"/></a:solidFill>\
</a:bgFillStyleLst>\
</a:fmtScheme>\
</a:themeElements>\
</a:theme>"
        .to_string()
}

// ── film lane — workbook EDIT-STATE timeline → demo video frame sequence ───────
//
// PHASE 4: turn an ordered list of workbook edit STATES into a demo film, entirely
// in-guest. A timeline is a JSON array of STEPS, each:
//     { "snapshot_html": "<full workbook HTML at this edit step>",  // inline, OR
//       "snapshot_file": "snap1.html",   // a file ref resolved in the sandbox CWD
//       "label":         "what changed at this step",
//       "hold_secs":     2.0 }           // how long this scene holds on screen
// (top-level may be the bare array OR `{ "steps": [...] }`).
//
// Each step is painted FULL-BLEED (the snapshot HTML rendered as-is, made to fill
// the viewport) with the `label` composited as an on-screen CAPTION (an overlay
// injected into the snapshot before paint, so it rides on top of the workbook).
// Between steps a short CROSSFADE provides a tasteful cut (CPU alpha-blend of the
// outgoing step's last frame with the incoming step's first frame). The result is
// a deterministic `frame_%05d.png` sequence the existing ffmpeg lane encodes to
// mp4 — NO native exec, NO GPU.

use serde::Deserialize;

/// One edit-step of a film timeline.
#[derive(Debug, Clone, Deserialize)]
pub struct FilmStep {
    /// The workbook HTML at this edit step (inline). Mutually-exclusive-ish with
    /// `snapshot_file`; if both are set, inline wins.
    #[serde(default)]
    pub snapshot_html: Option<String>,
    /// A path to the snapshot HTML, resolved relative to the timeline's directory
    /// (the host stages these into the sandbox CWD). Used when `snapshot_html` is
    /// absent.
    #[serde(default)]
    pub snapshot_file: Option<String>,
    /// On-screen caption describing what changed at this step.
    #[serde(default)]
    pub label: String,
    /// How long this scene holds on screen, in seconds (the still snapshot is
    /// painted once and repeated for the hold). Defaults to 2.0 when absent/<=0.
    #[serde(default)]
    pub hold_secs: Option<f64>,
}

impl FilmStep {
    fn hold(&self) -> f64 {
        match self.hold_secs {
            Some(s) if s > 0.0 => s,
            _ => 2.0,
        }
    }
}

/// A film timeline: either a bare array of steps or `{ "steps": [...] }`.
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
enum TimelineDoc {
    Bare(Vec<FilmStep>),
    Wrapped { steps: Vec<FilmStep> },
}

impl TimelineDoc {
    fn into_steps(self) -> Vec<FilmStep> {
        match self {
            TimelineDoc::Bare(v) => v,
            TimelineDoc::Wrapped { steps } => steps,
        }
    }
}

/// Parse a timeline.json string into the ordered step list.
pub fn parse_timeline(json: &str) -> Result<Vec<FilmStep>, String> {
    let doc: TimelineDoc = serde_json::from_str(json).map_err(|e| format!("timeline parse: {e}"))?;
    let steps = doc.into_steps();
    if steps.is_empty() {
        return Err("timeline has no steps".to_string());
    }
    Ok(steps)
}

/// Read + parse a timeline.json from a file path (WASI `std::fs`).
pub fn read_timeline(path: &Path) -> std::io::Result<Vec<FilmStep>> {
    let json = std::fs::read_to_string(path)?;
    parse_timeline(&json).map_err(std::io::Error::other)
}

/// HTML-escape a caption string so it can't break out of the injected overlay
/// markup (the label is author/agent-supplied text, not markup).
fn escape_html_text(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
    out
}

/// Inject a full-bleed normalization + a caption overlay into a snapshot's HTML.
///
/// The snapshot is rendered AS-IS but: (a) `html,body` are forced to fill the
/// viewport with no margin so the workbook is full-bleed; (b) a fixed caption bar
/// is appended carrying `label`. The overlay uses a high z-index + `!important`
/// so it sits above the workbook regardless of the workbook's own stacking.
/// Empty `label` → no caption (just the full-bleed normalization).
fn compose_film_scene_html(snapshot: &str, label: &str) -> String {
    let caption = if label.trim().is_empty() {
        String::new()
    } else {
        let safe = escape_html_text(label);
        // A modest lower-left caption pill — small, low-chrome (matches the brand
        // guidance to keep overlays small/modern, not heavy drop-shadow chrome).
        format!(
            "<div id=\"__wavelet_caption\">{safe}</div>"
        )
    };
    let overlay = format!(
        "<style id=\"__wavelet_film_iso\">\
         html,body{{margin:0 !important;padding:0 !important;\
         width:100% !important;height:100% !important;overflow:hidden !important;}}\
         #__wavelet_caption{{position:fixed !important;left:36px !important;bottom:36px !important;\
         z-index:2147483647 !important;max-width:78% !important;\
         font-family:'Geist',-apple-system,BlinkMacSystemFont,'Segoe UI',sans-serif !important;\
         font-size:26px !important;font-weight:600 !important;line-height:1.25 !important;\
         color:#fff !important;padding:12px 18px !important;\
         background:rgba(11,16,32,0.72) !important;\
         border-left:3px solid #3fe081 !important;border-radius:6px !important;\
         letter-spacing:0.01em !important;}}\
         </style>{caption}"
    );

    let lower = snapshot.to_ascii_lowercase();
    // Prefer injecting just before </body> so the caption is the last (top) node;
    // else before </html>; else append.
    if let Some(pos) = lower.rfind("</body>") {
        let mut out = String::with_capacity(snapshot.len() + overlay.len());
        out.push_str(&snapshot[..pos]);
        out.push_str(&overlay);
        out.push_str(&snapshot[pos..]);
        out
    } else if let Some(pos) = lower.rfind("</html>") {
        let mut out = String::with_capacity(snapshot.len() + overlay.len());
        out.push_str(&snapshot[..pos]);
        out.push_str(&overlay);
        out.push_str(&snapshot[pos..]);
        out
    } else {
        format!("{snapshot}{overlay}")
    }
}

/// Alpha-blend two equal-length RGBA8 buffers: `out = a*(1-t) + b*t`, per channel.
/// `t` in [0,1]. Used for the crossfade cut between steps.
fn blend_rgba(a: &[u8], b: &[u8], t: f64) -> Vec<u8> {
    debug_assert_eq!(a.len(), b.len());
    let t = t.clamp(0.0, 1.0);
    let inv = 1.0 - t;
    a.iter()
        .zip(b.iter())
        .map(|(&x, &y)| (x as f64 * inv + y as f64 * t).round().clamp(0.0, 255.0) as u8)
        .collect()
}

/// Resolve a step's snapshot HTML + the asset base URL to use when painting it.
///
/// `snapshot_html` inline → no asset base (relative `<img>` won't resolve; inline
/// snapshots are expected self-contained / data: URIs). `snapshot_file` → read the
/// file and use ITS directory as the asset base, so a snapshot's relative assets
/// resolve exactly like a standalone composition. `timeline_dir` is the directory
/// the timeline.json lives in (file refs resolve against it).
fn resolve_step_html(
    step: &FilmStep,
    timeline_dir: &Path,
) -> std::io::Result<(String, Option<String>)> {
    if let Some(html) = &step.snapshot_html {
        if !html.is_empty() {
            return Ok((html.clone(), None));
        }
    }
    if let Some(file) = &step.snapshot_file {
        let p = if Path::new(file).is_absolute() {
            std::path::PathBuf::from(file)
        } else {
            timeline_dir.join(file)
        };
        let html = std::fs::read_to_string(&p)?;
        let base = dir_base_url(&p);
        return Ok((html, base));
    }
    Err(std::io::Error::other(
        "film step has neither snapshot_html nor snapshot_file",
    ))
}

/// Render a film from a timeline file to a deterministic PNG frame sequence in
/// `out_dir` (`frame_00000.png` ..). Returns the number of frames written.
///
/// For each step: paint the snapshot full-bleed with its `label` caption ONCE,
/// then write that still for `round(fps * hold_secs)` frames (the "hold"). Between
/// consecutive steps, write `round(fps * crossfade_secs)` blended frames that fade
/// the previous step's frame into the next step's frame — a tasteful cut. Set
/// `crossfade_secs = 0.0` for hard cuts.
///
/// All in-guest: Blitz/Stylo/Vello-CPU paint, CPU alpha-blend for the fade, PNG
/// encode. NO ffmpeg, NO native exec, NO GPU. Deterministic.
pub fn render_film_to_dir(
    timeline_path: &Path,
    out_dir: &Path,
    fps: u32,
    width: u32,
    height: u32,
    crossfade_secs: f64,
) -> std::io::Result<u32> {
    let steps = read_timeline(timeline_path)?;
    let timeline_dir = timeline_path.parent().unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(out_dir)?;
    let fps = fps.max(1);
    let crossfade_secs = crossfade_secs.max(0.0);

    // Paint each step's still once (the hold is just repetition of the still).
    let mut stills: Vec<Vec<u8>> = Vec::with_capacity(steps.len());
    for step in &steps {
        let (html, base) = resolve_step_html(step, timeline_dir)?;
        let scene_html = compose_film_scene_html(&html, &step.label);
        // Seek the CSS clock to t=0 for the still — a snapshot is a STATE, not an
        // animation; frame 0 is its canonical look.
        let rgba = render_frame_rgba_with_base(&scene_html, 0, fps, width, height, base);
        stills.push(rgba);
    }

    let cross_n = (fps as f64 * crossfade_secs).round() as u32;
    let mut frame_idx: u32 = 0;

    let write_frame = |out_dir: &Path, idx: u32, rgba: &[u8]| -> std::io::Result<()> {
        let png = rgba_to_png(rgba, width, height);
        std::fs::write(out_dir.join(format!("frame_{idx:05}.png")), &png)
    };

    for (i, step) in steps.iter().enumerate() {
        // Crossfade INTO this step from the previous step's still (skip for the
        // first step — nothing to fade from).
        if i > 0 && cross_n > 0 {
            let prev = &stills[i - 1];
            let cur = &stills[i];
            for k in 1..=cross_n {
                let t = k as f64 / (cross_n as f64 + 1.0);
                let blended = blend_rgba(prev, cur, t);
                write_frame(out_dir, frame_idx, &blended)?;
                frame_idx += 1;
            }
        }
        // Hold this step's still.
        let hold_n = (fps as f64 * step.hold()).round().max(1.0) as u32;
        for _ in 0..hold_n {
            write_frame(out_dir, frame_idx, &stills[i])?;
            frame_idx += 1;
        }
    }

    Ok(frame_idx)
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

    // ── presentation (.pptx) export ───────────────────────────────────────────

    const DECK: &str = r#"<!doctype html><html><head><style>
      html,body{margin:0;padding:0;width:100%;height:100%}
      section{position:absolute;inset:0}
      .a{background:#3fe081}.b{background:#149157}.c{background:#0b1020}
      h1{color:#fff;font-family:sans-serif;font-size:64px;margin:40px}
    </style></head><body>
      <section class="a"><h1>One</h1></section>
      <section class="b"><h1>Two</h1></section>
      <section class="c"><h1>Three</h1></section>
    </body></html>"#;

    #[test]
    fn detect_section_scenes() {
        let (kind, n) = detect_scenes(DECK);
        assert_eq!(kind, SceneKind::Section);
        assert_eq!(n, 3, "expected 3 <section> scenes");
    }

    #[test]
    fn detect_data_scene_scenes_mixed_tags() {
        let html = r#"<body><div data-scene>a</div><article data-scene="x">b</article></body>"#;
        let (kind, n) = detect_scenes(html);
        assert_eq!(kind, SceneKind::DataAttr);
        assert_eq!(n, 2);
    }

    #[test]
    fn scene_token_in_style_or_script_not_counted() {
        // `data-scene` mentioned only inside CSS/JS must NOT count as a scene.
        let html = r#"<head><style>.x[data-scene]{color:red}</style>
            <script>var s="<section>"</script></head><body><p>hi</p></body>"#;
        let (kind, n) = detect_scenes(html);
        assert_eq!(kind, SceneKind::Whole);
        assert_eq!(n, 1);
    }

    #[test]
    fn annotate_assigns_ordinals_in_order() {
        let out = annotate_scenes(DECK, SceneKind::Section);
        assert!(out.contains("<section data-wavelet-scene=\"0\" class=\"a\""));
        assert!(out.contains("<section data-wavelet-scene=\"1\" class=\"b\""));
        assert!(out.contains("<section data-wavelet-scene=\"2\" class=\"c\""));
    }

    #[test]
    fn scenes_render_distinctly() {
        // Each isolated section has a different solid background, so the three
        // rendered scenes must differ from one another.
        let (w, h) = (200u32, 120u32);
        let (kind, total) = detect_scenes(DECK);
        let s0 = render_scene_rgba(DECK, kind, 0, total, 0, 30, w, h, None);
        let s1 = render_scene_rgba(DECK, kind, 1, total, 0, 30, w, h, None);
        let s2 = render_scene_rgba(DECK, kind, 2, total, 0, 30, w, h, None);
        assert_eq!(s0.len(), (w * h * 4) as usize);
        assert!(s0 != s1, "scene 0 == scene 1 — isolation failed");
        assert!(s1 != s2, "scene 1 == scene 2 — isolation failed");
        // Scene 0 background is bright green (#3fe081): sample a pixel away from text.
        let idx = ((100 * w + 180) * 4) as usize;
        let (r, g, b) = (s0[idx], s0[idx + 1], s0[idx + 2]);
        assert!(
            g > 150 && r < 150 && b < 180,
            "scene 0 not green ({r},{g},{b}) — wrong scene painted"
        );
    }

    #[test]
    fn assemble_pptx_is_a_valid_zip_with_expected_parts() {
        // Two trivial 2x2 PNGs as slides.
        let png = rgba_to_png(&[0u8; 2 * 2 * 4], 2, 2);
        let pptx = assemble_pptx(&[png.clone(), png.clone()], 1280, 720).expect("assemble");
        // Re-open with the zip crate and verify the required OOXML parts exist.
        let reader =
            zip::ZipArchive::new(Cursor::new(pptx)).expect("output is not a valid zip");
        let names: Vec<String> = (0..reader.len())
            .map(|i| reader.name_for_index(i).unwrap().to_string())
            .collect();
        for required in [
            "[Content_Types].xml",
            "_rels/.rels",
            "ppt/presentation.xml",
            "ppt/_rels/presentation.xml.rels",
            "ppt/slideMasters/slideMaster1.xml",
            "ppt/slideLayouts/slideLayout1.xml",
            "ppt/theme/theme1.xml",
            "ppt/slides/slide1.xml",
            "ppt/slides/slide2.xml",
            "ppt/slides/_rels/slide1.xml.rels",
            "ppt/slides/_rels/slide2.xml.rels",
            "ppt/media/image1.png",
            "ppt/media/image2.png",
        ] {
            assert!(names.iter().any(|n| n == required), "missing part: {required}");
        }
    }

    #[test]
    fn pptx_is_deterministic() {
        let png = rgba_to_png(&[7u8; 4 * 4 * 4], 4, 4);
        let a = assemble_pptx(&[png.clone()], 640, 360).unwrap();
        let b = assemble_pptx(&[png], 640, 360).unwrap();
        assert_eq!(a, b, "pptx assembly is not byte-deterministic");
    }

    // ── film lane (edit-state timeline → frame sequence) ───────────────────────

    #[test]
    fn parse_bare_and_wrapped_timeline() {
        let bare = r#"[{"snapshot_html":"<p>a</p>","label":"one","hold_secs":1.5},
                       {"snapshot_html":"<p>b</p>","label":"two"}]"#;
        let s = parse_timeline(bare).expect("parse bare");
        assert_eq!(s.len(), 2);
        assert_eq!(s[0].label, "one");
        assert_eq!(s[0].hold(), 1.5);
        assert_eq!(s[1].hold(), 2.0, "missing hold_secs defaults to 2.0");

        let wrapped = r#"{"steps":[{"snapshot_file":"snap.html","label":"x"}]}"#;
        let w = parse_timeline(wrapped).expect("parse wrapped");
        assert_eq!(w.len(), 1);
        assert_eq!(w[0].snapshot_file.as_deref(), Some("snap.html"));
    }

    #[test]
    fn empty_timeline_is_an_error() {
        assert!(parse_timeline("[]").is_err());
        assert!(parse_timeline(r#"{"steps":[]}"#).is_err());
    }

    #[test]
    fn caption_is_injected_and_escaped() {
        let html = "<!doctype html><html><body><h1>Workbook</h1></body></html>";
        let out = compose_film_scene_html(html, "<b>edit</b> & stuff");
        assert!(out.contains("__wavelet_caption"));
        assert!(out.contains("&lt;b&gt;edit&lt;/b&gt; &amp; stuff"), "label not escaped");
        assert!(out.contains("<div id=\"__wavelet_caption\">"), "caption div missing");
        // injected before </body>
        let cap_pos = out.find("<div id=\"__wavelet_caption\">").unwrap();
        let body_close = out.find("</body>").unwrap();
        assert!(cap_pos < body_close);
        // empty label → no caption div (the id still appears in the style selector,
        // but the caption element itself must NOT be present).
        let none = compose_film_scene_html(html, "   ");
        assert!(!none.contains("<div id=\"__wavelet_caption\">"));
        assert!(none.contains("__wavelet_film_iso"), "full-bleed style still injected");
    }

    #[test]
    fn blend_is_a_linear_mix() {
        let a = vec![0u8, 100, 200, 255];
        let b = vec![100u8, 200, 0, 255];
        let mid = blend_rgba(&a, &b, 0.5);
        assert_eq!(mid, vec![50u8, 150, 100, 255]);
        assert_eq!(blend_rgba(&a, &b, 0.0), a);
        assert_eq!(blend_rgba(&a, &b, 1.0), b);
    }

    #[test]
    fn render_film_writes_expected_frame_count_with_crossfade() {
        use std::io::Write;
        let dir = std::env::temp_dir().join(format!(
            "wavelet_film_test_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let timeline = dir.join("timeline.json");
        let json = r#"[
            {"snapshot_html":"<!doctype html><html><body style='background:#3fe081'></body></html>","label":"step one","hold_secs":1.0},
            {"snapshot_html":"<!doctype html><html><body style='background:#0b1020'></body></html>","label":"step two","hold_secs":1.0}
        ]"#;
        std::fs::File::create(&timeline).unwrap().write_all(json.as_bytes()).unwrap();

        let frames_dir = dir.join("frames");
        let (w, h, fps) = (160u32, 90u32, 10u32);
        // hold 1.0s @ 10fps = 10 frames each step; crossfade 0.4s = 4 frames between.
        let n = render_film_to_dir(&timeline, &frames_dir, fps, w, h, 0.4).unwrap();
        assert_eq!(n, 10 + 4 + 10, "expected hold+crossfade+hold frame count");
        assert!(frames_dir.join("frame_00000.png").exists());
        assert!(frames_dir.join(format!("frame_{:05}.png", n - 1)).exists());

        // The two steps have different backgrounds → first hold frame and last
        // hold frame must differ (proves distinct scenes rendered).
        let f0 = std::fs::read(frames_dir.join("frame_00000.png")).unwrap();
        let flast = std::fs::read(frames_dir.join(format!("frame_{:05}.png", n - 1))).unwrap();
        assert!(f0 != flast, "first and last step frames identical — scenes not distinct");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn render_film_hard_cut_no_crossfade() {
        use std::io::Write;
        let dir = std::env::temp_dir().join(format!(
            "wavelet_film_hardcut_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let timeline = dir.join("t.json");
        let json = r#"[{"snapshot_html":"<body>a</body>","label":"a","hold_secs":0.5},
                       {"snapshot_html":"<body>b</body>","label":"b","hold_secs":0.5}]"#;
        std::fs::File::create(&timeline).unwrap().write_all(json.as_bytes()).unwrap();
        let frames_dir = dir.join("f");
        let n = render_film_to_dir(&timeline, &frames_dir, 10, 80, 45, 0.0).unwrap();
        assert_eq!(n, 5 + 5, "hard cut = holds only, no crossfade frames");
        std::fs::remove_dir_all(&dir).ok();
    }
}

/// Extract readable text from a parsed document for agent scraping. Walks the DOM in document order,
/// skips non-content elements (script/style/head/noscript/template/svg), and inserts newlines at
/// block boundaries. Whitespace is collapsed per line, blank lines dropped. This is the agent-facing
/// "rendered text" of a page — CSS-aware via Blitz, no JS.
pub fn rendered_text(doc: &BaseDocument) -> String {
    let mut out = String::new();
    let root = doc.root_node().id;
    walk_text(doc, root, &mut out);
    out.split('\n')
        .map(|l| l.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

/// Per-node layout boxes for the **geometry bridge**. For every element carrying a `data-wb-nid`
/// attribute (stamped by the JS-DOM side before serialization), emit one line
/// `<nid> <x> <y> <width> <height>` — viewport-relative CSS px from real Blitz/Taffy layout. The host
/// injects these back into the in-engine JS-DOM so `getBoundingClientRect`/`offset*` return true
/// numbers instead of zero. Line-based, not JSON: trivially parsed, no allocator churn.
pub fn measured_boxes(doc: &BaseDocument) -> String {
    let mut out = String::new();
    let root = doc.root_node().id;
    collect_boxes(doc, root, &mut out);
    out
}

fn collect_boxes(doc: &BaseDocument, id: usize, out: &mut String) {
    // Read nid + children, then drop the node borrow before the `&self` rect query / recursion.
    let (nid, kids) = match doc.get_node(id) {
        Some(node) => (
            node.attr(LocalName::from("data-wb-nid")).map(|s| s.to_string()),
            node.children.clone(),
        ),
        None => return,
    };

    if let Some(nid) = nid {
        if let Some(r) = doc.get_client_bounding_rect(id) {
            out.push_str(&format!("{} {:.2} {:.2} {:.2} {:.2}\n", nid, r.x, r.y, r.width, r.height));
        }
    }

    for c in kids {
        collect_boxes(doc, c, out);
    }
}

fn walk_text(doc: &BaseDocument, id: usize, out: &mut String) {
    let node = match doc.get_node(id) {
        Some(n) => n,
        None => return,
    };
    match &node.data {
        NodeData::Text(t) => out.push_str(&t.content),
        NodeData::Element(_) => {
            let name = node
                .element_data()
                .map(|e| e.name.local.as_ref().to_string())
                .unwrap_or_default();
            if matches!(name.as_str(), "script" | "style" | "head" | "noscript" | "template" | "svg") {
                return;
            }
            let block = matches!(
                name.as_str(),
                "p" | "div" | "section" | "article" | "li" | "tr" | "h1" | "h2" | "h3" | "h4" | "h5"
                    | "h6" | "header" | "footer" | "nav" | "main" | "aside" | "blockquote" | "pre"
                    | "figure" | "figcaption" | "hr" | "br" | "ul" | "ol" | "table" | "title"
            );
            for c in &node.children {
                walk_text(doc, *c, out);
            }
            if block {
                out.push('\n');
            }
        }
        _ => {
            for c in &node.children {
                walk_text(doc, *c, out);
            }
        }
    }
}

/// Render a page WITH its inline JavaScript run against the DOM (Boa) → rendered text. Layer 3.
pub fn render_js_text(html: &str, base_url: Option<String>) -> String {
    let mut doc = load_html_with_base(html, 1280, 2400, base_url);
    js::run_scripts(&mut doc);
    rendered_text(doc.as_ref())
}

/// Render a page WITH its inline JavaScript run against the DOM (Boa) → PNG. JS-aware screenshot.
pub fn render_js_png(html: &str, base_url: Option<String>, width: u32, height: u32) -> Vec<u8> {
    let mut doc = load_html_with_base(html, width, height, base_url);
    js::run_scripts(&mut doc);
    let rgba = render_doc_rgba(doc.as_mut(), width, height);
    rgba_to_png(&rgba, width, height)
}
