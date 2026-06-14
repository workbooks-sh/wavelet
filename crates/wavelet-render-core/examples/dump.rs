//! Render one frame of a CSS-animated composition to a PNG on disk.
//! Native: `cargo run --example dump -- <frame> <out.png>`
//! Proves the carve renders the same way it will inside the nexus.

const SCENE: &str = r#"<!doctype html><html><head><style>
  html,body{margin:0;padding:0;width:100%;height:100%;background:#0b1020;overflow:hidden}
  #box{position:absolute;top:240px;width:280px;height:280px;border-radius:36px;
       background:linear-gradient(135deg,#3fe081,#149157);
       animation:slide 2s linear forwards}
  @keyframes slide{from{left:80px}to{left:1500px}}
  h1{position:absolute;left:80px;top:90px;font-size:96px;color:#fff;
     font-family:sans-serif;font-weight:800;letter-spacing:-0.02em}
  p{position:absolute;left:84px;top:210px;font-size:34px;color:#3fe081;font-family:sans-serif}
</style></head><body>
  <h1>Assembled in the nexus</h1>
  <p>Blitz + Vello CPU, no GPU, no native exec</p>
  <div id="box"></div>
</body></html>"#;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let frame: u32 = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(45);
    let out = args.get(2).cloned().unwrap_or_else(|| "frame.png".into());
    let png = wavelet_render_core::render_frame(SCENE, frame, 30, 1280, 720);
    std::fs::write(&out, &png).expect("write png");
    eprintln!("wrote {} ({} bytes), frame={}", out, png.len(), frame);
}
