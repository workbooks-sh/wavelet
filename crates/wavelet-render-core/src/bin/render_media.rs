//! WASI command: render a composition (loaded from a file path) to a STILL
//! image (PNG / JPEG / WebP — a single frame) or an ANIMATED GIF (the whole
//! composition). Pure in-guest — Blitz/Stylo/Vello-CPU paint + the `image`
//! crate encode. NO ffmpeg, NO native exec, NO GPU.
//!
//! Usage:
//!   render_media <composition.html> <out.ext> [--format png|jpeg|webp|gif]
//!                [--frame N] [--w W] [--h H] [--fps FPS] [--duration SECS]
//!
//! Defaults: format inferred from <out.ext> (else png); frame=24; w=1280;
//!           h=720; fps=30; duration=2.0 (GIF only).
//!
//! Still formats (png/jpeg/webp) render the SINGLE frame `--frame`. `gif`
//! renders frames 0..round(fps*duration) as an animated, infinitely-looping
//! GIF with per-frame delay = 1/fps.
//!
//! Native:
//!   cargo run --bin render_media -- examples/clip/clip.html out.webp --format webp --frame 24
//!   cargo run --bin render_media -- examples/clip/clip.html out.gif  --format gif  --fps 12 --duration 2
//! In wasmtime (preopen the crate dir for composition+assets, a writable out dir):
//!   cargo build --bin render_media --target wasm32-wasip1 --release
//!   wasmtime --dir=. target/wasm32-wasip1/release/render_media.wasm \
//!     examples/clip/clip.html /out/clip.gif --format gif --fps 12 --duration 2

use std::path::Path;
use wavelet_render_core::{
    render_animated_gif_from_path, render_still_from_path, StillFormat,
};

struct Opts {
    comp: String,
    out: String,
    format: Option<String>,
    frame: u32,
    w: u32,
    h: u32,
    fps: u32,
    duration: f64,
}

fn parse_opts(args: &[String]) -> Result<Opts, String> {
    let mut positional: Vec<String> = Vec::new();
    let mut format: Option<String> = None;
    let mut frame = 24u32;
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
            "--format" => format = Some(take("--format")?),
            "--frame" => frame = take("--frame")?.parse().map_err(|_| "bad --frame".to_string())?,
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
        return Err("need <composition.html> <out.ext>".to_string());
    }
    Ok(Opts {
        comp: positional[0].clone(),
        out: positional[1].clone(),
        format,
        frame,
        w,
        h,
        fps,
        duration,
    })
}

/// Resolve the effective format: explicit `--format` wins, else infer from the
/// output extension, else PNG.
fn resolve_format(opts: &Opts) -> Result<Choice, String> {
    let token = opts
        .format
        .clone()
        .or_else(|| {
            Path::new(&opts.out)
                .extension()
                .and_then(|e| e.to_str())
                .map(|s| s.to_string())
        })
        .unwrap_or_else(|| "png".into());

    let lc = token.to_ascii_lowercase();
    if lc == "gif" {
        return Ok(Choice::Gif);
    }
    StillFormat::parse(&lc)
        .map(Choice::Still)
        .ok_or_else(|| format!("unknown format {token:?} (want png|jpeg|webp|gif)"))
}

enum Choice {
    Still(StillFormat),
    Gif,
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let opts = match parse_opts(&args) {
        Ok(o) => o,
        Err(e) => {
            eprintln!("error: {e}");
            eprintln!(
                "usage: {} <composition.html> <out.ext> [--format png|jpeg|webp|gif] \
                 [--frame N] [--w W] [--h H] [--fps FPS] [--duration SECS]",
                args.first().map(String::as_str).unwrap_or("render_media")
            );
            std::process::exit(2);
        }
    };

    let choice = match resolve_format(&opts) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(2);
        }
    };

    let comp = Path::new(&opts.comp);
    let bytes = match choice {
        Choice::Still(fmt) => {
            let b = render_still_from_path(comp, opts.frame, opts.fps, opts.w, opts.h, fmt)
                .expect("render still from path");
            eprintln!(
                "rendered {} frame {} as {} ({}x{}@{}) -> {} ({} bytes)",
                opts.comp,
                opts.frame,
                fmt.ext(),
                opts.w,
                opts.h,
                opts.fps,
                opts.out,
                b.len()
            );
            b
        }
        Choice::Gif => {
            let b = render_animated_gif_from_path(comp, opts.fps, opts.duration, opts.w, opts.h)
                .expect("render animated gif from path");
            let n = (opts.fps.max(1) as f64 * opts.duration).round().max(1.0) as u32;
            eprintln!(
                "rendered {} as animated gif ({} frames, {}x{}@{}fps, {}s) -> {} ({} bytes)",
                opts.comp, n, opts.w, opts.h, opts.fps, opts.duration, opts.out, b.len()
            );
            b
        }
    };

    std::fs::write(&opts.out, &bytes).expect("write output");
}
