//! WASI command: export a multi-scene composition (loaded from a file path) to a
//! PowerPoint `.pptx` deck — ONE full-bleed slide per scene. Pure in-guest:
//! Blitz/Stylo/Vello-CPU paint a still PNG per scene, then a pure-Rust ZIP
//! (OOXML) writer assembles the deck. NO ffmpeg, NO native exec, NO GPU.
//!
//! Usage:
//!   render_present <composition.html> <out.pptx>
//!                  [--frame N] [--w W] [--h H] [--fps FPS]
//!
//! Defaults: frame=0; w=1280; h=720; fps=30.
//!
//! Scene convention (see lib::detect_scenes):
//!   1. elements carrying a `data-scene` attribute (any tag) — preferred;
//!   2. else top-level `<section>` elements;
//!   3. else the whole composition is one scene.
//! Each scene is isolated (its siblings hidden, it made full-bleed) and painted
//! at `t = frame/fps`, then placed as a full-bleed picture on its own slide.
//!
//! Native:
//!   cargo run --bin render_present -- examples/deck/deck.html out.pptx
//! In wasmtime (preopen the crate dir for composition+assets, a writable out dir):
//!   cargo build --bin render_present --target wasm32-wasip1 --release
//!   wasmtime --dir=.::/in --dir=/out::/out \
//!     target/wasm32-wasip1/release/render_present.wasm \
//!     /in/examples/deck/deck.html /out/deck.pptx --w 1280 --h 720

use std::path::Path;
use wavelet_render_core::{count_scenes_in_path, render_pptx_from_path};

struct Opts {
    comp: String,
    out: String,
    frame: u32,
    w: u32,
    h: u32,
    fps: u32,
}

fn parse_opts(args: &[String]) -> Result<Opts, String> {
    let mut positional: Vec<String> = Vec::new();
    let mut frame = 0u32;
    let mut w = 1280u32;
    let mut h = 720u32;
    let mut fps = 30u32;

    let mut i = 1;
    while i < args.len() {
        let a = &args[i];
        let mut take = |name: &str| -> Result<String, String> {
            i += 1;
            args.get(i)
                .cloned()
                .ok_or_else(|| format!("{name} needs a value"))
        };
        match a.as_str() {
            "--frame" => frame = take("--frame")?.parse().map_err(|_| "bad --frame".to_string())?,
            "--w" => w = take("--w")?.parse().map_err(|_| "bad --w".to_string())?,
            "--h" => h = take("--h")?.parse().map_err(|_| "bad --h".to_string())?,
            "--fps" => fps = take("--fps")?.parse().map_err(|_| "bad --fps".to_string())?,
            other if other.starts_with("--") => return Err(format!("unknown flag {other}")),
            _ => positional.push(a.clone()),
        }
        i += 1;
    }
    if positional.len() < 2 {
        return Err("need <composition.html> <out.pptx>".to_string());
    }
    Ok(Opts {
        comp: positional[0].clone(),
        out: positional[1].clone(),
        frame,
        w,
        h,
        fps,
    })
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let opts = match parse_opts(&args) {
        Ok(o) => o,
        Err(e) => {
            eprintln!("error: {e}");
            eprintln!(
                "usage: {} <composition.html> <out.pptx> [--frame N] [--w W] [--h H] [--fps FPS]",
                args.first().map(String::as_str).unwrap_or("render_present")
            );
            std::process::exit(2);
        }
    };

    let comp = Path::new(&opts.comp);
    let n = count_scenes_in_path(comp).expect("count scenes");
    let pptx = render_pptx_from_path(comp, opts.frame, opts.fps, opts.w, opts.h)
        .expect("render pptx from path");
    std::fs::write(&opts.out, &pptx).expect("write pptx");
    eprintln!(
        "rendered {} as a {n}-slide pptx ({}x{}, frame {} @ {}fps) -> {} ({} bytes)",
        opts.comp, opts.w, opts.h, opts.frame, opts.fps, opts.out, pptx.len()
    );
}
