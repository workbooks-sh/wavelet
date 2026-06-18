//! render_page — the browser-render path. Read an arbitrary fetched HTML page (not a wavelet
//! composition), render it via Blitz (Stylo CSS + Taffy layout + Vello-CPU paint) inside wasmtime,
//! and write a PNG screenshot. No Chromium, no native code, no GPU.
//!
//!   render_page <html_file> <out.png> <base_url> [width] [height]
//!
//! `base_url` is the page's own URL (e.g. https://news.ycombinator.com/) so relative `<link>`/`<img>`
//! references resolve to absolute URLs — without it, blitz-dom panics resolving a relative URL
//! against a non-base. External http(s) subresources are skipped by the net provider (no live net in
//! the sandbox); inline them host-side ("freeze") for a styled render.

fn arg<T: std::str::FromStr>(args: &[String], i: usize, default: T) -> T {
    args.get(i).and_then(|s| s.parse().ok()).unwrap_or(default)
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 4 {
        eprintln!("usage: render_page <html_file> <out.png> <base_url> [width] [height]");
        std::process::exit(2);
    }

    let html = match std::fs::read_to_string(&args[1]) {
        Ok(h) => h,
        Err(e) => {
            eprintln!("render_page: cannot read {}: {}", &args[1], e);
            std::process::exit(1);
        }
    };

    let out = &args[2];
    let base_url = args[3].clone();
    let w: u32 = arg(&args, 4, 1280);
    let h: u32 = arg(&args, 5, 2400);

    let base = if base_url.is_empty() { None } else { Some(base_url) };
    let rgba = wavelet_render_core::render_frame_rgba_with_base(&html, 0, 30, w, h, base);
    let png = wavelet_render_core::rgba_to_png(&rgba, w, h);

    if let Err(e) = std::fs::write(out, &png) {
        eprintln!("render_page: cannot write {}: {}", out, e);
        std::process::exit(1);
    }

    eprintln!("render_page: {} -> {} ({}x{}, {} bytes)", &args[1], out, w, h, png.len());
}
