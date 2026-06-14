//! WASI command: render a deterministic FRAME SEQUENCE of a composition file
//! to a directory of PNGs (`frame_00000.png`, `frame_00001.png`, ...).
//!
//! Usage:
//!   render_seq <composition.html> <out_dir> [--w W] [--h H] [--fps FPS]
//!                                            [--duration SECS]
//! Defaults: w=1280 h=720 fps=30 duration=2.0
//!
//! Native:
//!   cargo run --bin render_seq -- examples/clip/clip.html out/
//! In wasmtime (preopen the crate dir for the composition + assets and a
//! writable out dir):
//!   cargo build --bin render_seq --target wasm32-wasip1 --release
//!   wasmtime --dir=. --dir=/abs/out::/out \
//!     target/wasm32-wasip1/release/render_seq.wasm \
//!     examples/clip/clip.html /out --fps 24 --duration 2
//!
//! Output layout (N = round(fps * duration)):
//!   <out_dir>/frame_00000.png
//!   <out_dir>/frame_00001.png
//!   ...
//!   <out_dir>/frame_000NN.png   (frame index N-1)
//!
//! Deterministic: each frame seeks Stylo's CSS-animation clock to
//! t = frame / fps and re-parses the composition, so the same args always
//! produce byte-identical PNGs.

use std::path::Path;

struct Opts {
    comp: String,
    out_dir: String,
    w: u32,
    h: u32,
    fps: u32,
    duration: f64,
}

fn parse_opts(args: &[String]) -> Result<Opts, String> {
    let mut positional: Vec<String> = Vec::new();
    let mut w = 1280u32;
    let mut h = 720u32;
    let mut fps = 30u32;
    let mut duration = 2.0f64;

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
            "--duration" => {
                duration = take("--duration")?
                    .parse()
                    .map_err(|_| "bad --duration".to_string())?
            }
            other if other.starts_with("--") => return Err(format!("unknown flag {other}")),
            _ => positional.push(a.clone()),
        }
        i += 1;
    }
    if positional.len() < 2 {
        return Err("need <composition.html> <out_dir>".to_string());
    }
    Ok(Opts {
        comp: positional[0].clone(),
        out_dir: positional[1].clone(),
        w,
        h,
        fps,
        duration,
    })
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let opts = match parse_opts(&args) {
        Ok(o) => o,
        Err(e) => {
            eprintln!("error: {e}");
            eprintln!(
                "usage: {} <composition.html> <out_dir> [--w W] [--h H] [--fps FPS] [--duration SECS]",
                args.first().map(String::as_str).unwrap_or("render_seq")
            );
            std::process::exit(2);
        }
    };

    let n = wavelet_render_core::render_sequence_to_dir(
        Path::new(&opts.comp),
        Path::new(&opts.out_dir),
        opts.fps,
        opts.duration,
        opts.w,
        opts.h,
    )
    .expect("render sequence");

    eprintln!(
        "rendered {n} frames of {} ({}x{}@{}fps, {}s) -> {}/frame_00000.png .. frame_{:05}.png",
        opts.comp,
        opts.w,
        opts.h,
        opts.fps,
        opts.duration,
        opts.out_dir,
        n.saturating_sub(1)
    );
}
