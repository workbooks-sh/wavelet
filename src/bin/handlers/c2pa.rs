//! `wavelet c2pa { sign | verify }` handler — content-credentials signing.

use std::process::ExitCode;

use wavelet::c2pa_credentials::{sign_mp4, verify};
use wavelet::render_offline::Composition;

use wavelet::handlers::util::load_signing_key;
use wavelet::cli_args::C2paOp;

/// Dispatch entrypoint.
pub fn run(op: C2paOp) -> ExitCode {
    match op {
        C2paOp::Sign {
            input,
            out,
            comp,
            title,
            author,
            cache_root,
            signing_cert,
            signing_key,
        } => {
            let (composition, root_dir) = match Composition::from_json_path(&comp) {
                Ok(c) => c,
                Err(e) => {
                    eprintln!("error loading {}: {e}", comp.display());
                    return ExitCode::from(2);
                }
            };
            let cache = cache_root.unwrap_or_else(|| root_dir.join(".wavelet-cache"));
            let cache_opt = if cache.exists() { Some(cache.as_path()) } else { None };
            let key = match load_signing_key(signing_cert.as_deref(), signing_key.as_deref()) {
                Ok(k) => k,
                Err(code) => return code,
            };
            let auto_title = title.unwrap_or_else(|| {
                comp.file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("wavelet export")
                    .to_string()
            });
            match sign_mp4(
                &composition,
                cache_opt,
                Some(auto_title.as_str()),
                author.as_deref(),
                &input,
                &out,
                key,
            ) {
                Ok(report) => {
                    println!(
                        "signed {} → {} ({})",
                        input.display(),
                        out.display(),
                        report.summary
                    );
                    ExitCode::SUCCESS
                }
                Err(e) => {
                    eprintln!("c2pa sign failed: {e}");
                    ExitCode::from(2)
                }
            }
        }
        C2paOp::Verify { input, json } => match verify(&input) {
            Ok(report) => {
                if json {
                    println!("{}", report.raw_json);
                } else {
                    println!("{}", report.summary);
                    if !report.ingredients.is_empty() {
                        println!("ingredients:");
                        for i in &report.ingredients {
                            println!("  - {i}");
                        }
                    }
                    println!("assertions:");
                    for a in &report.assertion_labels {
                        println!("  - {a}");
                    }
                }
                if report.valid {
                    ExitCode::SUCCESS
                } else {
                    ExitCode::from(1)
                }
            }
            Err(e) => {
                eprintln!("c2pa verify failed: {e}");
                ExitCode::from(2)
            }
        },
    }
}
