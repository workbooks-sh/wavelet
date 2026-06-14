//! WASI command: render ONE frame of a composition **loaded from a file
//! path** to a PNG written via WASI fs. Relative `<img>` assets resolve
//! against the composition's directory.
//!
//! Usage:  render_one <composition.html> <frame> <out.png> [w] [h] [fps]
//!
//! Native:
//!   cargo run --bin render_one -- examples/clip/clip.html 24 frame.png
//! In wasmtime (preopen the crate dir so the composition + assets are
//! visible, and a writable out dir):
//!   cargo build --bin render_one --target wasm32-wasip1 --release
//!   wasmtime --dir=. target/wasm32-wasip1/release/render_one.wasm \
//!     examples/clip/clip.html 24 /out/frame.png
//!
//! Identical code path native vs wasm — the only WASI-specific surface is
//! `std::fs` (preopened dirs) and `std::env::args`.

use std::path::Path;

fn arg<T: std::str::FromStr>(args: &[String], i: usize, default: T) -> T {
    args.get(i).and_then(|s| s.parse().ok()).unwrap_or(default)
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        eprintln!(
            "usage: {} <composition.html> [frame] [out.png] [w] [h] [fps]",
            args.first().map(String::as_str).unwrap_or("render_one")
        );
        std::process::exit(2);
    }
    let comp = Path::new(&args[1]);
    let frame: u32 = arg(&args, 2, 24);
    let out = args.get(3).cloned().unwrap_or_else(|| "frame.png".into());
    let w: u32 = arg(&args, 4, 1280);
    let h: u32 = arg(&args, 5, 720);
    let fps: u32 = arg(&args, 6, 30);

    let png = wavelet_render_core::render_frame_from_path(comp, frame, fps, w, h)
        .expect("render frame from path");
    std::fs::write(&out, &png).expect("write png");
    eprintln!(
        "rendered {} frame {frame} ({w}x{h}@{fps}) -> {out} ({} bytes)",
        comp.display(),
        png.len()
    );
}
