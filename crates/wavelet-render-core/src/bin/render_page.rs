//! render_page — the browser-render spike. Read an arbitrary fetched HTML page (not a wavelet
//! composition), render it via Blitz (Stylo CSS + Taffy layout + Vello-CPU paint) entirely inside
//! wasmtime, and write a PNG screenshot. Proves in-wasm page rendering with NO Chromium, NO native
//! code, NO GPU. Reads from a WASI-preopened dir; argv-driven like the other bins.
//!
//!   render_page <html_file> [out.png] [width] [height]

fn arg<T: std::str::FromStr>(args: &[String], i: usize, default: T) -> T {
    args.get(i).and_then(|s| s.parse().ok()).unwrap_or(default)
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        eprintln!("usage: render_page <html_file> [out.png] [width] [height]");
        std::process::exit(2);
    }

    let html = match std::fs::read_to_string(&args[1]) {
        Ok(h) => h,
        Err(e) => {
            eprintln!("render_page: cannot read {}: {}", &args[1], e);
            std::process::exit(1);
        }
    };

    let out = args.get(2).cloned().unwrap_or_else(|| "page.png".into());
    let w: u32 = arg(&args, 3, 1280);
    let h: u32 = arg(&args, 4, 2400);

    // frame 0 of a 30fps timeline = t=0 (static page; no animation needed for a snapshot).
    let png = wavelet_render_core::render_frame(&html, 0, 30, w, h);

    if let Err(e) = std::fs::write(&out, &png) {
        eprintln!("render_page: cannot write {}: {}", out, e);
        std::process::exit(1);
    }

    eprintln!("render_page: {} -> {} ({}x{}, {} bytes)", &args[1], out, w, h, png.len());
}
