//! render_js — run a page's inline JS against the Blitz DOM, then extract rendered text.
//!   render_js <html_file> <base_url>
fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 3 { eprintln!("usage: render_js <html_file> <base_url>"); std::process::exit(2); }
    let html = std::fs::read_to_string(&args[1]).unwrap_or_default();
    let base = if args[2].is_empty() { None } else { Some(args[2].clone()) };
    print!("{}", wavelet_render_core::render_js_text(&html, base));
}
