//! WASI command: turn a workbook EDIT-STATE timeline into a demo-film frame
//! sequence — one full-bleed scene per edit step with the step's `label` as an
//! on-screen caption, and a tasteful crossfade cut between steps. Pure in-guest:
//! Blitz/Stylo/Vello-CPU paint each snapshot, CPU alpha-blend for the fade, PNG
//! out. NO ffmpeg, NO native exec, NO GPU. (The existing ffmpeg lane encodes the
//! PNG sequence to mp4.)
//!
//! Usage:
//!   render_film <timeline.json> <out_dir>
//!               [--w W] [--h H] [--fps FPS] [--crossfade SECS]
//! Defaults: w=1280 h=720 fps=30 crossfade=0.4
//!
//! timeline.json is an ordered list of steps (bare array OR `{"steps":[...]}`):
//!   { "snapshot_html": "<the workbook HTML at this edit step>",   // inline, OR
//!     "snapshot_file": "snap1.html",   // resolved relative to the timeline dir
//!     "label":         "what changed",
//!     "hold_secs":     2.0 }
//!
//! Native:
//!   cargo run --bin render_film -- timeline.json out/
//! In wasmtime (preopen the timeline+snapshot dir, a writable out dir):
//!   cargo build --bin render_film --target wasm32-wasip1 --release
//!   wasmtime --dir=.::/in --dir=/abs/out::/out \
//!     target/wasm32-wasip1/release/render_film.wasm \
//!     /in/timeline.json /out --fps 30 --crossfade 0.4

use std::path::Path;

struct Opts {
    timeline: String,
    out_dir: String,
    w: u32,
    h: u32,
    fps: u32,
    crossfade: f64,
}

fn parse_opts(args: &[String]) -> Result<Opts, String> {
    let mut positional: Vec<String> = Vec::new();
    let mut w = 1280u32;
    let mut h = 720u32;
    let mut fps = 30u32;
    let mut crossfade = 0.4f64;

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
            "--w" => w = take("--w")?.parse().map_err(|_| "bad --w".to_string())?,
            "--h" => h = take("--h")?.parse().map_err(|_| "bad --h".to_string())?,
            "--fps" => fps = take("--fps")?.parse().map_err(|_| "bad --fps".to_string())?,
            "--crossfade" => {
                crossfade = take("--crossfade")?
                    .parse()
                    .map_err(|_| "bad --crossfade".to_string())?
            }
            other if other.starts_with("--") => return Err(format!("unknown flag {other}")),
            _ => positional.push(a.clone()),
        }
        i += 1;
    }
    if positional.len() < 2 {
        return Err("need <timeline.json> <out_dir>".to_string());
    }
    Ok(Opts {
        timeline: positional[0].clone(),
        out_dir: positional[1].clone(),
        w,
        h,
        fps,
        crossfade,
    })
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let opts = match parse_opts(&args) {
        Ok(o) => o,
        Err(e) => {
            eprintln!("error: {e}");
            eprintln!(
                "usage: {} <timeline.json> <out_dir> [--w W] [--h H] [--fps FPS] [--crossfade SECS]",
                args.first().map(String::as_str).unwrap_or("render_film")
            );
            std::process::exit(2);
        }
    };

    let n = wavelet_render_core::render_film_to_dir(
        Path::new(&opts.timeline),
        Path::new(&opts.out_dir),
        opts.fps,
        opts.w,
        opts.h,
        opts.crossfade,
    )
    .expect("render film");

    eprintln!(
        "rendered film: {n} frames ({}x{}@{}fps, crossfade {}s) from {} -> {}/frame_00000.png .. frame_{:05}.png",
        opts.w,
        opts.h,
        opts.fps,
        opts.crossfade,
        opts.timeline,
        opts.out_dir,
        n.saturating_sub(1)
    );
}
