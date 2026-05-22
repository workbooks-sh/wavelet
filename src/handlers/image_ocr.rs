use std::path::PathBuf;
use std::process::ExitCode;
use crate::handlers::util::emit_analysis;
use crate::handlers::util::image_arg_to_url;

/// (auto-generated placeholder)
pub fn run_image_ocr(
    image: String,
    backend: Option<String>,
    dry_run: bool,
    max_cost: f32,
    cache: PathBuf,
    pretty: bool,
) -> ExitCode {
    use crate::backends::fal::RoboflowClient;
    use crate::backends::image::{OcrBackend, OcrRequest};
    use crate::backends::roboflow::RoboflowDoctrOcrAdapter;
    use crate::backends::{BackendError, RunMode};
    use crate::image_analysis;

    let backend = backend.unwrap_or_else(|| {
        if std::env::var("ROBOFLOW_API_KEY")
            .ok()
            .map(|v| !v.trim().is_empty())
            .unwrap_or(false)
        {
            "roboflow-doctr".to_string()
        } else {
            "rapidocr-local".to_string()
        }
    });

    match backend.as_str() {
        "rapidocr-local" => {
            let path = std::path::PathBuf::from(&image);
            emit_analysis(pretty, || image_analysis::ocr::analyze(&path))
        }
        "roboflow-doctr" => {
            let image_url = match image_arg_to_url(&image) {
                Ok(v) => v,
                Err(e) => {
                    eprintln!("image ocr: {e}");
                    return ExitCode::from(2);
                }
            };
            let req = OcrRequest::new(image_url);
            let mode = if dry_run {
                RunMode::DryRun
            } else {
                RunMode::Live { max_cost_usd: max_cost }
            };
            let client = if dry_run {
                RoboflowClient::with_key("dry-run", &cache)
            } else {
                match RoboflowClient::from_env(&cache) {
                    Ok(c) => c,
                    Err(e) => {
                        eprintln!("backend roboflow-doctr: {e}");
                        if let BackendError::MissingCredential(name) = &e {
                            eprintln!("set {name} or pass --dry-run to preview.");
                        }
                        return ExitCode::from(2);
                    }
                }
            };
            let adapter = RoboflowDoctrOcrAdapter::new(client);
            match adapter.recognize(&req, mode) {
                Ok(outcome) => {
                    let payload = serde_json::json!({
                        "mode": outcome.mode,
                        "provider": outcome.provider,
                        "request_hash": outcome.request_hash,
                        "cached": outcome.cached,
                        "cost_estimate_usd": outcome.cost_estimate_usd,
                        "result": outcome.response,
                    });
                    let formatted = if pretty {
                        serde_json::to_string_pretty(&payload)
                    } else {
                        serde_json::to_string(&payload)
                    };
                    println!("{}", formatted.unwrap_or_else(|e| format!(r#"{{"error":"{e}"}}"#)));
                    ExitCode::SUCCESS
                }
                Err(e) => {
                    eprintln!("image ocr: {e}");
                    ExitCode::from(2)
                }
            }
        }
        other => {
            eprintln!("unknown --backend '{other}', want roboflow-doctr or rapidocr-local");
            ExitCode::from(3)
        }
    }
}
