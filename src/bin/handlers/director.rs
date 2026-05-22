//! `wavelet director synthesize` — LLM-as-creative-director slot filler.

use std::process::ExitCode;

use std::path::PathBuf;

use wavelet::backends::fal::FalClient;
use wavelet::backends::image::VisionVerifyResult;
use wavelet::director::{
    mutate_prompt, resolve_model_flag, synthesize_shot_attributes, DirectorRequest,
    FalAnyLlmBackend, GraderRequest, ShotSkeleton,
};
use wavelet::storyboard::{Generation, Shot, Storyboard};

use super::super::DirectorOp;

/// Dispatch entrypoint.
pub fn run(op: DirectorOp) -> ExitCode {
    match op {
        DirectorOp::Synthesize {
            brief,
            storyboard,
            out,
            model,
            style_anchor,
            pretty,
            cache,
        } => run_synthesize(brief, storyboard, out, model, style_anchor, pretty, cache),
        DirectorOp::Grade {
            brief,
            prompt,
            findings,
            previous,
            model,
            out,
            pretty,
            cache,
        } => run_grade(brief, prompt, findings, previous, model, out, pretty, cache),
    }
}

#[allow(clippy::too_many_arguments)]
fn run_synthesize(
    brief: PathBuf,
    storyboard: PathBuf,
    out: PathBuf,
    model: String,
    style_anchor: Option<String>,
    pretty: bool,
    cache: PathBuf,
) -> ExitCode {
    let brief_text = match std::fs::read_to_string(&brief) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("read {}: {e}", brief.display());
            return ExitCode::from(2);
        }
    };
    let sb_src = match std::fs::read_to_string(&storyboard) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("read {}: {e}", storyboard.display());
            return ExitCode::from(2);
        }
    };
    let mut sb: Storyboard = match serde_json::from_str(&sb_src) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("parse {}: {e}", storyboard.display());
            return ExitCode::from(2);
        }
    };
    if sb.shots.is_empty() {
        eprintln!("storyboard has zero shots; nothing to synthesize");
        return ExitCode::from(2);
    }

    let shots: Vec<ShotSkeleton> = sb
        .shots
        .iter()
        .map(|s| ShotSkeleton {
            id: s.id.clone(),
            subject: s.subject.clone(),
            action: shot_action_phrase(s),
        })
        .collect();

    let client = match FalClient::from_env(&cache) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("director: FAL client: {e}");
            return ExitCode::from(2);
        }
    };
    let routed = resolve_model_flag(&model);
    let llm = FalAnyLlmBackend::new(client, routed);

    let req = DirectorRequest {
        brief: brief_text,
        shots,
        style_anchor,
    };

    let attrs = match synthesize_shot_attributes(req, &llm) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("director: {e}");
            return ExitCode::from(3);
        }
    };

    for (id, a) in &attrs {
        if let Some(shot) = sb.shots.iter_mut().find(|s| &s.id == id) {
            shot.attributes = Some(a.clone());
        }
    }

    let serialized = if pretty {
        serde_json::to_string_pretty(&sb)
    } else {
        serde_json::to_string(&sb)
    };
    let json = match serialized {
        Ok(s) => s,
        Err(e) => {
            eprintln!("serialize: {e}");
            return ExitCode::from(2);
        }
    };
    if let Err(e) = std::fs::write(&out, json) {
        eprintln!("write {}: {e}", out.display());
        return ExitCode::from(2);
    }
    eprintln!(
        "director: populated {} of {} shots → {} (model: {routed})",
        attrs.len(),
        sb.shots.len(),
        out.display()
    );
    ExitCode::SUCCESS
}

#[allow(clippy::too_many_arguments)]
fn run_grade(
    brief: PathBuf,
    prompt: String,
    findings_path: PathBuf,
    previous: Vec<String>,
    model: String,
    out: PathBuf,
    pretty: bool,
    cache: PathBuf,
) -> ExitCode {
    let brief_text = match std::fs::read_to_string(&brief) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("read {}: {e}", brief.display());
            return ExitCode::from(2);
        }
    };
    let findings_src = match std::fs::read_to_string(&findings_path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("read {}: {e}", findings_path.display());
            return ExitCode::from(2);
        }
    };
    let verify: VisionVerifyResult = match serde_json::from_str(&findings_src) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("parse {}: {e}", findings_path.display());
            return ExitCode::from(2);
        }
    };
    let client = match FalClient::from_env(&cache) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("director grade: FAL client: {e}");
            return ExitCode::from(2);
        }
    };
    let routed = resolve_model_flag(&model);
    let llm = FalAnyLlmBackend::new(client, routed);

    let req = GraderRequest {
        original_prompt: prompt,
        findings: verify.findings,
        brief: brief_text,
        previous_mutations: previous,
    };
    let result = match mutate_prompt(req, &llm) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("director grade: {e}");
            return ExitCode::from(3);
        }
    };

    let serialized = if pretty {
        serde_json::to_string_pretty(&result)
    } else {
        serde_json::to_string(&result)
    };
    let json = match serialized {
        Ok(s) => s,
        Err(e) => {
            eprintln!("serialize: {e}");
            return ExitCode::from(2);
        }
    };
    if let Err(e) = std::fs::write(&out, json) {
        eprintln!("write {}: {e}", out.display());
        return ExitCode::from(2);
    }
    eprintln!(
        "director grade: addressed {} finding(s) → {} (model: {routed})",
        result.addressed_findings.len(),
        out.display()
    );
    ExitCode::SUCCESS
}

fn shot_action_phrase(shot: &Shot) -> String {
    match &shot.generation {
        Generation::Img2Vid { motion_prompt, .. } if !motion_prompt.trim().is_empty() => {
            motion_prompt.clone()
        }
        Generation::Txt2Vid { prompt, .. } if !prompt.trim().is_empty() => prompt.clone(),
        Generation::Controlnet { prompt, .. } if !prompt.trim().is_empty() => prompt.clone(),
        Generation::StockSearch { query, .. } if !query.trim().is_empty() => query.clone(),
        Generation::Native { html } => format!("native render: {html}"),
        _ => "unspecified".into(),
    }
}
