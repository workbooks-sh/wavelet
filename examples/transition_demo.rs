//! End-to-end shader-transition smoke test.
//!
//! Compiles a wavelet_fx crossfade, builds the wavelet transition pipeline,
//! runs three frames (progress = 0, 0.5, 1.0) with two synthetic input
//! frames (solid red, solid blue), and writes the output PNGs to /tmp.
//! Expectations:
//!   - progress=0   → output is red (frame A)
//!   - progress=0.5 → output is purple-gray (50/50 blend)
//!   - progress=1   → output is blue (frame B)
//!
//! `cargo run --release --example transition_demo`

use wavelet::shader::{create_wgpu, fx_source, TransitionPipeline};

const W: u32 = 320;
const H: u32 = 240;

fn solid(rgba: [u8; 4]) -> Vec<u8> {
    rgba.iter()
        .cycle()
        .copied()
        .take((W * H * 4) as usize)
        .collect()
}

fn save_png(path: &str, rgba: &[u8]) {
    let file = std::fs::File::create(path).expect("create png");
    let mut encoder = png::Encoder::new(file, W, H);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().expect("png header");
    writer.write_image_data(rgba).expect("png data");
}

fn main() {
    let (device, queue) = create_wgpu().expect("no GPU adapter");
    println!("wgpu device ready");

    // Hardcoded crossfade in wavelet_fx's DSL. `src(0)` is the outgoing frame,
    // `src(1)` is the incoming, `prop("progress")` is the transition
    // window's 0..1 normalized time.
    let shady_src = r#"
        src(0).blend(src(1), prop("progress")).out
    "#;
    let source = fx_source(shady_src).expect("compile wavelet_fx");
    println!("wavelet_fx compiled");

    let mut pipeline =
        TransitionPipeline::new(device, queue, W, H, &source).expect("pipeline build");
    println!("transition pipeline built");

    let frame_a = solid([255, 0, 0, 255]); // red
    let frame_b = solid([0, 0, 255, 255]); // blue

    for (label, progress) in &[("start", 0.0_f32), ("mid", 0.5), ("end", 1.0)] {
        let out = pipeline.render(&frame_a, &frame_b, 0.0, *progress);
        let path = format!("/tmp/wavelet-transition-{}.png", label);
        save_png(&path, &out);
        let mid_pixel = (out[(H / 2 * W + W / 2) as usize * 4],
                         out[(H / 2 * W + W / 2) as usize * 4 + 1],
                         out[(H / 2 * W + W / 2) as usize * 4 + 2]);
        println!("progress={progress:.2} → center px = {mid_pixel:?}  saved {path}");
    }
}
