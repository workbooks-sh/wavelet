//! Plan executor — dispatches every `Step` to the matching tool.
//!
//! No model calls happen here; this is dumb routing + filesystem
//! coordination. Inputs are typed; outputs are paths.

use std::path::{Path, PathBuf};

use super::intent::{EditRequest, InputKind};
use super::plan::{Approach, Plan, Step};
use super::tools;
use super::EditError;

/// Output of one successful execution — the rendered MP4 path plus
/// a flat summary of what the executor actually did (fed to the
/// reviewer so it can compare claims vs. visible reality).
pub struct ExecOutput {
    /// Rendered MP4.
    pub output_path: PathBuf,
    /// Plain-text recap of every step that ran.
    pub plan_summary: String,
}

/// Execute a plan against an input. The work tree is rooted at the
/// input's parent directory; intermediate artifacts (edited HTML,
/// veo cache) land alongside.
pub fn execute_plan(
    req: &EditRequest,
    plan: &Plan,
    out_mp4: &Path,
    attempt_n: u32,
) -> Result<ExecOutput, EditError> {
    let mut summary = format!(
        "approach={:?}; intent_summary={}; steps={}\n",
        plan.approach,
        plan.intent_summary,
        plan.steps.len()
    );
    for (i, step) in plan.steps.iter().enumerate() {
        summary.push_str(&format!("  [{i}] {}\n", describe_step(step)));
    }

    match plan.approach {
        Approach::CssOnly => execute_css_only(req, plan, out_mp4, attempt_n, summary),
        Approach::VeoRegen => execute_veo_regen(req, plan, out_mp4, summary),
        Approach::OmniEdit => Err(tools::omni_edit::unavailable(
            &req.input,
            &plan.intent_summary,
            &std::env::var("GOOGLE_API_KEY").unwrap_or_default(),
        )),
        Approach::Composite => tools::composite::run_composite(&[], out_mp4).map(|p| ExecOutput {
            output_path: p,
            plan_summary: summary,
        }),
    }
}

fn execute_css_only(
    req: &EditRequest,
    plan: &Plan,
    out_mp4: &Path,
    attempt_n: u32,
    summary: String,
) -> Result<ExecOutput, EditError> {
    let scene_html = resolve_scene_html(req)?;
    let raw = std::fs::read_to_string(&scene_html)
        .map_err(|e| EditError::Transport(format!("read scene html: {e}")))?;
    let edited = tools::css_only::apply_css_steps(&raw, &plan.steps)?;
    // Per-attempt edit path so attempts don't clobber each other.
    let edit_path = scene_html.with_extension(format!("wavelet-edit-{attempt_n}.html"));
    std::fs::write(&edit_path, &edited)
        .map_err(|e| EditError::Transport(format!("write edited html: {e}")))?;
    let output_path = tools::css_only::render_css_edit(&edit_path, &edited, out_mp4)?;
    Ok(ExecOutput {
        output_path,
        plan_summary: summary,
    })
}

fn execute_veo_regen(
    req: &EditRequest,
    plan: &Plan,
    out_mp4: &Path,
    summary: String,
) -> Result<ExecOutput, EditError> {
    let regen_step = plan
        .steps
        .iter()
        .find_map(|s| match s {
            Step::VeoRegen {
                prompt,
                duration_secs,
                aspect,
                max_cost_usd,
            } => Some((prompt.clone(), *duration_secs, aspect.clone(), *max_cost_usd)),
            _ => None,
        })
        .ok_or_else(|| {
            EditError::PlanParse("VeoRegen approach with no VeoRegen step".into())
        })?;
    let cache_root = req
        .input
        .parent()
        .map(|p| p.join(".wavelet-cache"))
        .unwrap_or_else(|| PathBuf::from(".wavelet-cache"));
    let path = tools::veo_regen::run_veo_regen(
        &regen_step.0,
        regen_step.1,
        &regen_step.2,
        regen_step.3,
        &cache_root,
        out_mp4,
    )?;
    Ok(ExecOutput {
        output_path: path,
        plan_summary: summary,
    })
}

fn resolve_scene_html(req: &EditRequest) -> Result<PathBuf, EditError> {
    match req.kind {
        InputKind::SceneHtml => Ok(req.input.clone()),
        InputKind::Mp4 => {
            // Sniff for a sibling .html or `scenes/<stem>.html` near the mp4.
            let stem = req.input.file_stem().and_then(|s| s.to_str()).unwrap_or("");
            let parent = req.input.parent().unwrap_or_else(|| Path::new("."));
            let candidates = [
                parent.join(format!("{stem}.html")),
                parent.join("scenes").join(format!("{stem}.html")),
                parent.join("scenes").join("scene-01.html"),
            ];
            for c in &candidates {
                if c.exists() {
                    return Ok(c.clone());
                }
            }
            // Fallback: any single .html in the scenes/ dir.
            let scenes = parent.join("scenes");
            if scenes.is_dir() {
                if let Ok(entries) = std::fs::read_dir(&scenes) {
                    let htmls: Vec<_> = entries
                        .flatten()
                        .filter(|e| {
                            e.path()
                                .extension()
                                .and_then(|s| s.to_str())
                                .map(|s| s.eq_ignore_ascii_case("html"))
                                .unwrap_or(false)
                        })
                        .collect();
                    if htmls.len() == 1 {
                        return Ok(htmls[0].path());
                    }
                }
            }
            Err(EditError::Transport(format!(
                "CssOnly requires a scene HTML; could not locate one near {}",
                req.input.display()
            )))
        }
    }
}

fn describe_step(step: &Step) -> String {
    match step {
        Step::CssFilter { target_selector, css } => format!("CssFilter {target_selector} :: {css}"),
        Step::CssAnimation { target_selector, .. } => format!("CssAnimation {target_selector}"),
        Step::PlaybackRate { target_selector, value } => format!("PlaybackRate {target_selector} x{value}"),
        Step::DurationOverride { secs } => format!("DurationOverride {secs}s"),
        Step::ReRender { duration_secs } => format!("ReRender {duration_secs:?}"),
        Step::OmniEdit { instruction, .. } => format!("OmniEdit {instruction}"),
        Step::VeoRegen { prompt, duration_secs, aspect, .. } => {
            format!("VeoRegen [{aspect} {duration_secs}s] {prompt}")
        }
        Step::Splice { source, start_secs, end_secs } => {
            format!("Splice {} {}..{}", source.display(), start_secs, end_secs)
        }
    }
}
