//! js_spike — prove a pure-Rust JS engine (Boa) evaluates JS inside wasmtime.
fn main() {
    let src = std::env::args().nth(1).unwrap_or_else(|| "1 + 2 * 3".into());
    let mut ctx = boa_engine::Context::default();
    match ctx.eval(boa_engine::Source::from_bytes(&src)) {
        Ok(v) => println!("JS OK: {} = {}", src, v.display()),
        Err(e) => println!("JS ERR: {e}"),
    }
}
