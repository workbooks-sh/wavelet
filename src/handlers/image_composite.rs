use std::path::Path;
use std::process::ExitCode;

/// Composite a foreground image (with alpha channel) over a background.
/// Returns `ExitCode::SUCCESS` on success or `ExitCode::from(2)` on
/// failure (with the error printed to stderr).
pub fn handle_image_composite(
    foreground: &Path,
    background: &Path,
    out: &Path,
    scale: f32,
    y_offset: f32,
) -> ExitCode {
    match crate::backends::image::compose::composite_over(
        foreground,
        background,
        out,
        scale,
        y_offset,
    ) {
        Ok((w, h)) => {
            println!(
                "{}",
                serde_json::json!({
                    "ok": true,
                    "out": out.display().to_string(),
                    "width": w,
                    "height": h,
                })
            );
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("image composite: {e}");
            ExitCode::from(2)
        }
    }
}
