use std::path::Path;
use std::process::ExitCode;
use crate::handlers::util::{parse_region, emit_analysis};
use crate::query::contrast;

/// (auto-generated placeholder)
pub fn handle_image_contrast(
    image: &Path,
    region: &str,
    text_color: &str,
    threshold: f32,
    pretty: bool,
) -> ExitCode {
    use crate::image_analysis;
    let parsed_region = match parse_region(region) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("image contrast: {e}");
            return ExitCode::from(3);
        }
    };
    let parsed_color = match image_analysis::Rgb::parse_hex(text_color) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("image contrast: {e}");
            return ExitCode::from(3);
        }
    };
    emit_analysis(pretty, || {
        image_analysis::contrast::analyze(image, parsed_region, parsed_color, threshold)
    })
}
