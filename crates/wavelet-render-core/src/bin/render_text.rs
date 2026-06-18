//! render_text — extract the rendered, readable text of a fetched page (CSS-aware via Blitz, no JS),
//! for agent scraping. Reads HTML, walks the Blitz DOM, prints text to stdout.
//!   render_text <html_file> <base_url>
fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 3 {
        eprintln!("usage: render_text <html_file> <base_url>");
        std::process::exit(2);
    }
    let html = std::fs::read_to_string(&args[1]).unwrap_or_default();
    let base = if args[2].is_empty() { None } else { Some(args[2].clone()) };
    let doc = wavelet_render_core::load_html_with_base(&html, 1280, 2400, base);
    print!("{}", wavelet_render_core::rendered_text(doc.as_ref()));
}
