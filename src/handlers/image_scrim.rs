use std::path::Path;
use std::process::ExitCode;

/// (auto-generated placeholder)
pub fn handle_image_scrim(
    image: &Path,
    rows: u32,
    cols: u32,
    threshold: f32,
    out: Option<&Path>,
    pretty: bool,
) -> ExitCode {
    use crate::image_analysis;
    let report = match image_analysis::scrim::analyze(image, rows, cols, threshold) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("image scrim: {e}");
            return ExitCode::from(2);
        }
    };
    let json = if pretty {
        serde_json::to_string_pretty(&report)
    } else {
        serde_json::to_string(&report)
    };
    match (json, out) {
        (Ok(s), Some(p)) => {
            if let Err(e) = std::fs::write(p, &s) {
                eprintln!("write {}: {e}", p.display());
                return ExitCode::from(2);
            }
            eprintln!("wrote {}", p.display());
            ExitCode::SUCCESS
        }
        (Ok(s), None) => {
            println!("{s}");
            ExitCode::SUCCESS
        }
        (Err(e), _) => {
            eprintln!("serialize scrim: {e}");
            ExitCode::from(2)
        }
    }
}
