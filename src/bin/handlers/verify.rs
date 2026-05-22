//! `wavelet verify` handler — structural lint of a comp.json.

use std::path::PathBuf;
use std::process::ExitCode;

use wavelet::render_offline::Composition;
use wavelet::verify::{verify, Level};

/// Run verify on a comp.json.
pub fn run(comp_path: PathBuf, deep: bool) -> ExitCode {
    let (comp, root_dir) = match Composition::from_json_path(&comp_path) {
        Ok(pair) => pair,
        Err(e) => {
            eprintln!("error loading {}: {e}", comp_path.display());
            return ExitCode::from(3);
        }
    };

    let findings = verify(&comp, &root_dir, deep);
    let mut errors = 0usize;
    let mut warnings = 0usize;
    for f in &findings {
        let prefix = match f.level {
            Level::Error => {
                errors += 1;
                "ERROR"
            }
            Level::Warning => {
                warnings += 1;
                "WARN "
            }
        };
        println!("{prefix}  [{}] {}", f.origin, f.message);
    }
    println!();
    if findings.is_empty() {
        println!("✓ clean: 0 errors, 0 warnings");
        ExitCode::SUCCESS
    } else if errors == 0 {
        println!("◐ {} warning(s), 0 errors", warnings);
        ExitCode::SUCCESS
    } else {
        println!("✗ {} error(s), {} warning(s)", errors, warnings);
        ExitCode::from(1)
    }
}
