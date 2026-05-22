use std::path::PathBuf;
use std::process::ExitCode;
use crate::handlers::util::serialize_search;

/// (auto-generated placeholder)
#[allow(clippy::too_many_arguments)]
pub fn run_shot_search(
    query: String,
    backend: String,
    orientation: Option<String>,
    min_duration: Option<u32>,
    max_duration: Option<u32>,
    per_page: u32,
    page: u32,
    dry_run: bool,
    max_cost: f32,
    cache: PathBuf,
    pretty: bool,
) -> ExitCode {
    use crate::backends::pexels::PexelsAdapter;
    use crate::backends::pond5::Pond5Adapter;
    use crate::backends::stock::{Orientation, StockSearchBackend, StockSearchRequest};
    use crate::backends::{BackendError, RunMode};

    let orient = match orientation.as_deref() {
                    None => None,
                    Some("landscape") => Some(Orientation::Landscape),
                    Some("portrait") => Some(Orientation::Portrait),
                    Some("square") => Some(Orientation::Square),
                    Some(other) => {
                        eprintln!("invalid --orientation '{other}', want landscape|portrait|square");
                        return ExitCode::from(3);
                    }
                };
                let req = StockSearchRequest {
                    query,
                    orientation: orient,
                    min_duration_secs: min_duration,
                    max_duration_secs: max_duration,
                    per_page,
                    page,
                };
                let mode = if dry_run {
                    RunMode::DryRun
                } else {
                    RunMode::Live {
                        max_cost_usd: max_cost,
                    }
                };

                let outcome = match backend.as_str() {
                    "pexels" => {
                        let adapter = if dry_run {
                            // Dry-run doesn't need a real key.
                            PexelsAdapter::with_key("dry-run", &cache)
                        } else {
                            match PexelsAdapter::from_env(&cache) {
                                Ok(a) => a,
                                Err(e) => {
                                    eprintln!("backend pexels: {e}");
                                    if let BackendError::MissingCredential(name) = &e {
                                        eprintln!("set {name} or pass --dry-run to preview.");
                                    }
                                    return ExitCode::from(2);
                                }
                            }
                        };
                        serialize_search(adapter.search(&req, mode))
                    }
                    "pond5" => {
                        let adapter = Pond5Adapter::default();
                        serialize_search(adapter.search(&req, mode))
                    }
                    other => {
                        eprintln!("unknown --backend '{other}', want pexels|pond5");
                        return ExitCode::from(3);
                    }
                };

                match outcome {
                    Ok(json) => {
                        let formatted = if pretty {
                            serde_json::to_string_pretty(&json)
                        } else {
                            serde_json::to_string(&json)
                        };
                        println!("{}", formatted.unwrap_or_else(|e| format!(r#"{{"error":"{e}"}}"#)));
                        ExitCode::SUCCESS
                    }
                    Err(e) => {
                        eprintln!("shot search: {e}");
                        ExitCode::from(2)
                    }
                }
}
