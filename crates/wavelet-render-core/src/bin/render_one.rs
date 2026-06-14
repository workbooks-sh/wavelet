//! WASI command: render one frame of a built-in CSS-animated composition to
//! a PNG written via WASI fs. This is the in-nexus proof harness.
//!
//! Run natively:   `cargo run --bin render_one -- <frame> <out.png>`
//! Run in wasmtime:
//!   cargo build --bin render_one --target wasm32-wasip1 --release
//!   wasmtime --dir=. target/.../render_one.wasm <frame> /out.png
//!
//! Identical code path native vs wasm — the only WASI-specific surface is
//! `std::fs::write` (preopened dir) and `std::env::args`.

const SCENE: &str = r#"<!doctype html><html><head><style>
  html,body{margin:0;padding:0;width:100%;height:100%;background:#0b1020;overflow:hidden}
  #box{position:absolute;top:240px;width:280px;height:280px;border-radius:36px;
       background:linear-gradient(135deg,#3fe081,#149157);
       animation:slide 2s linear forwards}
  @keyframes slide{from{left:80px}to{left:1500px}}
  #bar{position:absolute;left:0;bottom:0;height:24px;background:#3fe081;
       animation:grow 2s linear forwards}
  @keyframes grow{from{width:0}to{width:1280px}}
  h1{position:absolute;left:80px;top:90px;font-size:92px;color:#fff;font-weight:700;
     letter-spacing:-0.02em;animation:fade 1.2s ease-out forwards}
  p{position:absolute;left:84px;top:215px;font-size:32px;color:#3fe081}
  @keyframes fade{from{opacity:0;transform:translateY(40px)}to{opacity:1;transform:translateY(0)}}
</style></head><body>
  <h1>Assembled in the nexus</h1>
  <p>Blitz + Vello CPU + bundled font, in wasm32-wasip1</p>
  <div id="box"></div>
  <div id="bar"></div>
</body></html>"#;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let frame: u32 = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(45);
    let out = args.get(2).cloned().unwrap_or_else(|| "frame.png".into());
    let png = wavelet_render_core::render_frame(SCENE, frame, 30, 1280, 720);
    std::fs::write(&out, &png).expect("write png");
    eprintln!("rendered frame {frame} -> {out} ({} bytes)", png.len());
}
