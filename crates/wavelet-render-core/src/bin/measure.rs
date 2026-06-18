//! measure — emit per-node layout boxes for the geometry bridge (the in-wasm answer to
//! `getBoundingClientRect` returning zero in a pure-JS DOM). Reads HTML whose elements carry
//! `data-wb-nid` attributes, runs real Blitz/Taffy layout, prints `<nid> <x> <y> <w> <h>` per line.
//!   measure <html_file> <base_url>
fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 3 {
        eprintln!("usage: measure <html_file> <base_url>");
        std::process::exit(2);
    }
    let html = std::fs::read_to_string(&args[1]).unwrap_or_default();
    let base = if args[2].is_empty() { None } else { Some(args[2].clone()) };
    let doc = wavelet_render_core::load_html_with_base(&html, 1280, 2400, base);
    print!("{}", wavelet_render_core::measured_boxes(doc.as_ref()));
}
