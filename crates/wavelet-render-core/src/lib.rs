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
//! - **Paint**: `anyrender_vello_cpu` (pure-CPU Vello, peniko/kurbo). No
//!   wgpu, no GPU adapter.
//! - **Output**: RGBA `Vec<u8>` or PNG bytes.
//!
//! ## What this carve EXCLUDES (stays host-side)
//!
//! - `rsmpeg` / system ffmpeg (native = bedrock under WASM): encode of
//!   frames → mp4 is a host broker or a Forge ffmpeg→wasi lane.
//! - `wgpu` / `anyrender_vello` (GPU).
//! - `src/backends/` (network services), ocr / c2pa / depth / agent /
//!   director / screenplay, video_bg decode, CSS-filter post-process.
//!
//! The nexus only needs to *paint frames*; everything else is host-side.

use anyrender::{ImageRenderer, PaintScene as _};
use anyrender_vello_cpu::VelloCpuImageRenderer;
use blitz_dom::{build_single_font_ctx, BaseDocument, DocumentConfig};
use blitz_html::HtmlDocument;
use blitz_paint::paint_scene;
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

/// Parse + resolve an HTML composition string into a Blitz document at the
/// given viewport. No net provider: compositions handed to the nexus must
/// be self-contained (inline CSS / data: URIs). Relative `file://` /
/// `http(s)://` asset loads are intentionally unsupported here — asset
/// fetch is a host capability, not an in-sandbox one.
pub fn load_html(html: &str, width: u32, height: u32) -> HtmlDocument {
    let mut document = HtmlDocument::from_html(
        html,
        DocumentConfig {
            viewport: Some(Viewport::new(width, height, 1.0, ColorScheme::Light)),
            font_ctx: Some(build_single_font_ctx(BUNDLED_FONT)),
            ..Default::default()
        },
    );
    // Drain any queued messages + resolve initial layout. No external
    // fetches are issued (no net provider), so a couple of passes settle
    // the document.
    for _ in 0..2 {
        document.as_mut().handle_messages();
        document.as_mut().resolve(0.0);
    }
    document
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

/// The core entrypoint requested by the spike: parse a composition, seek the
/// CSS-animation clock to `t = frame / fps`, paint, and return RGBA8 bytes
/// (`width * height * 4`).
///
/// `fps` is how the integer `frame` maps to seconds. The composition's CSS
/// `animation` declarations are what make consecutive frames differ.
pub fn render_frame_rgba(
    composition_html: &str,
    frame: u32,
    fps: u32,
    width: u32,
    height: u32,
) -> Vec<u8> {
    let mut document = load_html(composition_html, width, height);
    let t_secs = frame as f64 / fps.max(1) as f64;
    // Drive Stylo's CSS animation engine to time `t`.
    document.as_mut().resolve(t_secs);
    render_doc_rgba(document.as_mut(), width, height)
}

/// Same as [`render_frame_rgba`] but returns PNG-encoded bytes.
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
        // Not blank: some pixel is the green box color region.
        assert!(a.iter().any(|&p| p != 255), "frame 0 is all-white");
        // Animated: the two frames differ (box slid).
        assert!(a != b, "frame 0 and frame 45 identical — timeline not advancing");
    }
}
