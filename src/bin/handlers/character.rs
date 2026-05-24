//! `wavelet character …` handlers (wb-cx08).
//!
//! Today: only `define` is implemented. Emits a character-ref clip-HTML
//! at `<workdir>/refs/character/<name>.clip.html` that the storyboard
//! planner auto-discovers.

use std::path::PathBuf;
use std::process::ExitCode;

use wavelet::cli_args::{CharacterOp, CliCharacterType};
use wavelet::clipref::character::{emit_character, EmitOptions};

/// Entry point for the `wavelet character` subcommand.
pub fn run(op: CharacterOp) -> ExitCode {
    match op {
        CharacterOp::Define {
            name,
            reference,
            character_type,
            workdir,
        } => define(name, reference, character_type, workdir),
    }
}

fn define(
    name: String,
    reference: Vec<PathBuf>,
    character_type: CliCharacterType,
    workdir: Option<PathBuf>,
) -> ExitCode {
    if reference.is_empty() {
        eprintln!("error: at least one --reference must be passed");
        return ExitCode::from(2);
    }
    // Veo 3.1 reference-to-video accepts up to 4 reference images; the
    // adapter validates the upper bound at submit time. We mirror that
    // here as a warning rather than a hard error so authors who want to
    // pre-stage a 5th candidate can still define the bundle.
    if reference.len() > 4 {
        eprintln!(
            "warn: {} references passed; fal-veo3-ref accepts at most 4 — the extras will be trimmed at submit time",
            reference.len(),
        );
    }

    // References pass through verbatim: a `PathBuf` can hold either a
    // local path or an HTTPS URL string. The Fal Veo adapter handles
    // both shapes downstream.
    let refs: Vec<String> = reference
        .iter()
        .map(|p| p.to_string_lossy().to_string())
        .collect();

    let workdir = workdir.unwrap_or_else(|| PathBuf::from("."));
    let opts = EmitOptions { workdir: &workdir };
    let emission = match emit_character(&name, &refs, character_type.into(), &opts) {
        Ok(e) => e,
        Err(e) => {
            eprintln!("emit: {e}");
            return ExitCode::from(2);
        }
    };
    eprintln!("wrote {}", emission.path.display());
    println!("{}", emission.path.display());
    ExitCode::SUCCESS
}
