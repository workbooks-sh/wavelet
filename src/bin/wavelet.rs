//! `wavelet` — the canonical CLI for the wavelet motion-graphics renderer.
//!
//! ```text
//! wavelet render <commercial.html> [-o out.mp4]
//! wavelet verify <commercial.html|*.mp4> [--deep]
//! wavelet query  <commercial.html> ...      (Phase 1+; currently a stub)
//! wavelet shader ...                  (Phase 7+; currently a stub)
//! ```

use clap::{Parser, Subcommand, ValueEnum};
use wavelet::query::{
    banding, bbox_of, color_at, color_in, contrast, diff_videos, events_from_composition,
    in_safe_area, no_overlap, on_beat, text_visible, transform_inherits, visibility_of,
    DiffMetric, DiffOptions, FrameDiff, FramePixels, FrameSnapshot, OverlapPair, Rect,
    ScoredEvent, VisibilityVerdict,
};
use wavelet::cli_args::{AgentOp, BriefOp, C2paOp, CaptionsOp, CharacterOp, ClipOp, Cmd, ContinuityOp, DialogueOp, DirectorOp, ImageOp, MusicOp, PipelinesOp, PlanModeArg, ScreenplayOp, ShaderOp, ShotOp, StoryboardOp, TransitionsOp, VelocityOp, WorkflowOp};
use wavelet::handlers::util::{QueryArgs, QueryEntry, QueryOutput, QuerySummary};
use wavelet::handlers::util::image_arg_to_url;
use wavelet::render_offline::Composition;
use std::path::PathBuf;
use std::process::ExitCode;
use wavelet::handlers::util::pick_model_auto;
use wavelet::handlers::util::resolve_model;
use wavelet::handlers::util::parse_target;
use wavelet::handlers::util::resolve_local_path;
use wavelet::handlers::shot_dispatch::run_shot;
use wavelet::handlers::query_dispatch::run_query;
use wavelet::handlers::storyboard_dispatch::run_storyboard;
use wavelet::handlers::image_dispatch::run_image;
use wavelet::handlers::query_shader::run_query_shader;
use wavelet::handlers::agent_dispatch::run_agent;
use wavelet::handlers::lint::run as run_lint;

mod handlers;

#[derive(Parser)]
#[command(
    name = "wavelet",
    version,
    about = "Wavelet motion-graphics renderer — render, verify, query, and shade compositions.",
    long_about = None,
)]
struct Cli {
    /// Force the CPU Vello backend even if a GPU adapter is available.
    /// Default is to use GPU when present; CPU is the automatic fallback
    /// for headless / no-adapter environments.
    #[arg(long, global = true)]
    cpu: bool,

    #[command(subcommand)]
    cmd: Cmd,
}
fn main() -> ExitCode {
    let cli = Cli::parse();
    if cli.cpu {
        wavelet::render::force_cpu();
    }
    match cli.cmd {
        Cmd::Render {
            comp,
            out,
            sign_c2pa,
            title,
            author,
            cache_root,
            signing_cert,
            signing_key,
            aspects,
            frame_budget_secs,
            no_audio,
        } => handlers::render::run(
            comp,
            out,
            handlers::render::C2paOpts {
                sign: sign_c2pa,
                title,
                author,
                cache_root,
                signing_cert,
                signing_key,
            },
            aspects,
            frame_budget_secs,
            no_audio,
        ),
        Cmd::Verify { comp, deep } => handlers::verify::run(comp, deep),
        Cmd::Query {
            comp,
            at,
            bbox,
            visible,
            in_safe_area: safe_sel,
            inset,
            transform_inherits: xform_sel,
            no_overlap: no_overlap_arg,
            color_at: color_at_arg,
            color_in: color_in_arg,
            max_de,
            contrast: contrast_arg,
            contrast_threshold,
            banding: banding_arg,
            on_beat: on_beat_arg,
            tolerance_ms,
            text_visible: text_visible_arg,
            text_in,
            text_tolerance,
            snapshot,
            json: _json,
            repl,
        } => {
            if repl {
                return run_repl_mode(comp);
            }
            run_query(QueryArgs {
            comp,
            at,
            bbox,
            visible,
            safe_sel,
            inset,
            xform_sel,
            no_overlap: no_overlap_arg,
            color_at: color_at_arg,
            color_in: color_in_arg,
            max_de,
            contrast: contrast_arg,
            contrast_threshold,
            banding: banding_arg,
            on_beat: on_beat_arg,
            tolerance_ms,
            text_visible: text_visible_arg,
            text_in,
            text_tolerance,
            snapshot,
        })
        }
        Cmd::Diff {
            a,
            b,
            metric,
            threshold,
            clip,
            max_diff_ratio,
            report,
        } => handlers::diff::run(a, b, metric, threshold, clip, max_diff_ratio, report),
        Cmd::Shader { op: ShaderOp::Validate { .. } } => {
            eprintln!("wavelet shader: not yet implemented — tracked at wb-so23 (Phase 7a).");
            ExitCode::from(3)
        }
        Cmd::Screenplay { op } => match op {
            ScreenplayOp::Parse { path, workdir, legacy_json, pretty, out } => {
                handlers::screenplay::run(path, workdir, legacy_json, pretty, out)
            }
            ScreenplayOp::Reassemble { workdir, out } => {
                handlers::screenplay::reassemble(workdir, out)
            }
            ScreenplayOp::Validate { path, duration, pretty } => {
                handlers::screenplay::validate(path, duration, pretty)
            }
            ScreenplayOp::Characters { path, json, pretty } => {
                handlers::screenplay::characters(path, json, pretty)
            }
        },
        Cmd::Velocity { op } => handlers::velocity::run(op),
        Cmd::Storyboard { op } => run_storyboard(op),
        Cmd::Continuity { op } => handlers::continuity::run(op),
        Cmd::Transitions { op } => handlers::transitions::run(op),
        Cmd::Shot { op } => run_shot(op),
        Cmd::Dialogue { op } => handlers::dialogue::run(op),
        Cmd::Captions { op } => handlers::captions::run(op),
        Cmd::Image { op } => run_image(op),
        Cmd::Music { op } => handlers::music::run(op),
        Cmd::Brief { op } => handlers::brief::run(op),
        Cmd::Director { op } => handlers::director::run(op),
        Cmd::C2pa { op } => handlers::c2pa::run(op),
        Cmd::Pipelines { op } => handlers::pipelines::run(op),
        Cmd::Workflow { op } => handlers::workflow::run(op),
        Cmd::Clip { op } => handlers::clip::run(op),
        Cmd::Lipsync {
            video,
            audio,
            backend,
            sync_mode,
            temperature,
            active_speaker,
            dry_run,
            max_cost,
            cache,
            out,
            pretty,
        } => handlers::lipsync::run(
            video, audio, backend, sync_mode, temperature, active_speaker, dry_run, max_cost,
            cache, out, pretty,
        ),
        Cmd::QueryShader {
            shader,
            frame,
            params,
        } => run_query_shader(&shader, &frame, &params),
        Cmd::Agent { op } => run_agent(op),
        Cmd::Lint(op) => run_lint(op),
        Cmd::Character { op } => handlers::character::run(op),
    }
}



















#[cfg(test)]
mod cli_upscale_tests {
    use super::*;

    #[test]
    fn auto_mode_picks_supir_for_images() {
        assert_eq!(pick_model_auto("foo.png"), Some(UpscaleModel::Supir));
        assert_eq!(pick_model_auto("https://x/y.JPG"), Some(UpscaleModel::Supir));
        assert_eq!(pick_model_auto("a.webp?token=x"), Some(UpscaleModel::Supir));
    }

    #[test]
    fn auto_mode_picks_seedvr2_for_video() {
        assert_eq!(pick_model_auto("clip.mp4"), Some(UpscaleModel::Seedvr2));
        assert_eq!(pick_model_auto("https://x/y.MOV"), Some(UpscaleModel::Seedvr2));
        assert_eq!(pick_model_auto("a.webm?q=1"), Some(UpscaleModel::Seedvr2));
    }

    #[test]
    fn auto_mode_returns_none_for_unknown_ext() {
        assert_eq!(pick_model_auto("foo.bin"), None);
        assert_eq!(pick_model_auto("foo"), None);
    }

    #[test]
    fn resolve_model_explicit_wins() {
        assert_eq!(resolve_model("supir", "anything.mp4"), Ok(UpscaleModel::Supir));
        assert_eq!(resolve_model("seedvr2-7b", "anything.png"), Ok(UpscaleModel::Seedvr2));
    }

    #[test]
    fn resolve_model_auto_uses_extension() {
        assert_eq!(resolve_model("auto", "foo.png"), Ok(UpscaleModel::Supir));
        assert_eq!(resolve_model("auto", "foo.mp4"), Ok(UpscaleModel::Seedvr2));
        assert!(resolve_model("auto", "foo").is_err());
    }

    #[test]
    fn resolve_model_rejects_unknown() {
        assert!(resolve_model("magic-upscale", "foo.png").is_err());
    }

    #[test]
    fn parse_target_handles_scales() {
        assert_eq!(parse_target("2x"), Ok(UpscaleTarget::Scale(2.0)));
        assert_eq!(parse_target("4x"), Ok(UpscaleTarget::Scale(4.0)));
    }

    #[test]
    fn parse_target_handles_named_resolutions() {
        assert_eq!(parse_target("1080p"), Ok(UpscaleTarget::Resolution(1920, 1080)));
        assert_eq!(parse_target("4k"), Ok(UpscaleTarget::Resolution(3840, 2160)));
    }

    #[test]
    fn parse_target_handles_explicit_wxh() {
        assert_eq!(parse_target("2560x1440"), Ok(UpscaleTarget::Resolution(2560, 1440)));
    }

    #[test]
    fn parse_target_rejects_garbage() {
        assert!(parse_target("biggest").is_err());
        assert!(parse_target("zx").is_err());
    }
}




/// Literal instruction the wb-j1ef.2 spike found effective. Do not
/// re-engineer this — the phrasing about LEFT/RIGHT halves and "output
/// only the merged scene without the side-by-side layout" is load-
/// bearing.

/// Cap on `--strict-identity` re-rolls.




















fn run_repl_mode(comp_path: PathBuf) -> ExitCode {
    let (comp, root_dir) = match Composition::from_json_path(&comp_path) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("error loading {}: {e}", comp_path.display());
            return ExitCode::from(3);
        }
    };
    let processed = wavelet::query::run_repl(&comp, &root_dir);
    eprintln!("wavelet query --repl: processed {processed} command(s)");
    ExitCode::SUCCESS
}











#[cfg(test)]
mod image_arg_tests {
    use super::image_arg_to_url;
    use std::io::Write;

    #[test]
    fn https_url_passes_through() {
        let url = "https://example.com/cat.jpg";
        assert_eq!(image_arg_to_url(url).unwrap(), url);
    }

    #[test]
    fn http_url_passes_through() {
        let url = "http://example.com/cat.jpg";
        assert_eq!(image_arg_to_url(url).unwrap(), url);
    }

    #[test]
    fn data_url_passes_through() {
        let url = "data:image/png;base64,iVBORw0KGgo=";
        assert_eq!(image_arg_to_url(url).unwrap(), url);
    }

    #[test]
    fn local_png_becomes_data_url() {
        let dir = std::env::temp_dir().join("wavelet-image-arg-png");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("tiny.png");
        let png_magic: &[u8] = b"\x89PNG\r\n\x1a\n\x00\x00\x00\rIHDR\x00\x00\x00\x01\x00\x00\x00\x01";
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(png_magic).unwrap();
        drop(f);
        let url = image_arg_to_url(path.to_str().unwrap()).unwrap();
        assert!(
            url.starts_with("data:image/png;base64,"),
            "expected png data URL, got {}",
            &url[..url.len().min(64)]
        );
    }

    #[test]
    fn local_jpeg_becomes_data_url() {
        let dir = std::env::temp_dir().join("wavelet-image-arg-jpg");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("tiny.jpg");
        let jpeg_magic: &[u8] = b"\xff\xd8\xff\xe0\x00\x10JFIF\x00";
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(jpeg_magic).unwrap();
        drop(f);
        let url = image_arg_to_url(path.to_str().unwrap()).unwrap();
        assert!(
            url.starts_with("data:image/jpeg;base64,"),
            "expected jpeg data URL, got {}",
            &url[..url.len().min(64)]
        );
    }

    #[test]
    fn missing_file_errors_clearly() {
        let err = image_arg_to_url("/nonexistent/path/to/image-xyz123.png").unwrap_err();
        assert!(
            err.contains("read image file"),
            "error should mention the file: {err}"
        );
    }

    #[test]
    fn empty_arg_errors_clearly() {
        let err = image_arg_to_url("").unwrap_err();
        assert!(err.contains("empty"), "error should mention empty: {err}");
    }
}

#[cfg(test)]
mod insert_into_scene_tests {
    use super::*;

    #[test]
    fn instruction_matches_spike_recipe() {
        let expected = "merge these two reference images: take the product from the LEFT half and place it naturally into the scene from the RIGHT half, matching the right-half's lighting direction, color temperature, and shadow geometry; output only the merged scene without the side-by-side layout, leave the right-half scene composition unchanged";
        assert_eq!(INSERT_INTO_SCENE_INSTRUCTION, expected);
    }

    #[test]
    fn instruction_keeps_load_bearing_phrases() {
        let i = INSERT_INTO_SCENE_INSTRUCTION;
        assert!(i.contains("LEFT half"));
        assert!(i.contains("RIGHT half"));
        assert!(i.contains("output only the merged scene without the side-by-side layout"));
        assert!(i.contains("leave the right-half scene composition unchanged"));
    }

    #[test]
    fn retry_cap_matches_spec() {
        assert_eq!(INSERT_INTO_SCENE_MAX_RETRIES, 3);
    }

    #[test]
    fn resolve_local_path_rejects_https() {
        let err = resolve_local_path("https://example.com/x.png").unwrap_err();
        assert!(err.contains("URL inputs not supported"));
    }

    #[test]
    fn resolve_local_path_rejects_missing_file() {
        let err = resolve_local_path("/nonexistent/insert-into-scene-x.png").unwrap_err();
        assert!(err.contains("does not exist"));
    }

    #[test]
    fn resolve_local_path_accepts_real_file() {
        let dir = std::env::temp_dir().join("wavelet-insert-into-scene-resolve");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("a.png");
        std::fs::write(&p, b"x").unwrap();
        let out = resolve_local_path(p.to_str().unwrap()).unwrap();
        assert_eq!(out, p);
    }
}

#[cfg(test)]
mod agent_cli_flag_tests {
    use super::*;
    use clap::Parser;

    fn parse(args: &[&str]) -> AgentOp {
        let cli = Cli::try_parse_from(args).expect("clap parse");
        match cli.cmd {
            Cmd::Agent { op } => op,
            _ => panic!("expected Agent subcommand"),
        }
    }

    #[test]
    fn chat_defaults_have_plan_mode_off_no_workdir_and_1800s_cap() {
        let op = parse(&["wavelet", "agent", "chat"]);
        let AgentOp::Chat {
            plan_mode,
            plan_workdir,
            max_wall_seconds,
            ..
        } = op
        else {
            panic!("expected Chat");
        };
        assert_eq!(plan_mode, PlanModeArg::Off);
        assert!(plan_workdir.is_none());
        assert_eq!(max_wall_seconds, 1800);
    }

    #[test]
    fn chat_accepts_plan_flags() {
        let op = parse(&[
            "wavelet",
            "agent",
            "chat",
            "--plan-mode",
            "on",
            "--plan-workdir",
            "./foo",
            "--max-wall-seconds",
            "60",
        ]);
        let AgentOp::Chat {
            plan_mode,
            plan_workdir,
            max_wall_seconds,
            ..
        } = op
        else {
            panic!("expected Chat");
        };
        assert_eq!(plan_mode, PlanModeArg::On);
        assert_eq!(plan_workdir, Some(PathBuf::from("./foo")));
        assert_eq!(max_wall_seconds, 60);
    }

    #[test]
    fn chat_shadow_mode_parses() {
        let op = parse(&["wavelet", "agent", "chat", "--plan-mode", "shadow"]);
        let AgentOp::Chat { plan_mode, .. } = op else {
            panic!("expected Chat");
        };
        assert_eq!(plan_mode, PlanModeArg::Shadow);
    }

    #[test]
    fn serve_accepts_plan_flags() {
        let op = parse(&[
            "wavelet",
            "agent",
            "serve",
            "--plan-mode",
            "shadow",
            "--plan-workdir",
            "/tmp/p",
            "--max-wall-seconds",
            "42",
        ]);
        let AgentOp::Serve {
            plan_mode,
            plan_workdir,
            max_wall_seconds,
            ..
        } = op
        else {
            panic!("expected Serve");
        };
        assert_eq!(plan_mode, PlanModeArg::Shadow);
        assert_eq!(plan_workdir, Some(PathBuf::from("/tmp/p")));
        assert_eq!(max_wall_seconds, 42);
    }

    #[test]
    fn plan_mode_arg_maps_to_agent_plan_mode() {
        assert_eq!(
            wavelet::agent::PlanMode::from(PlanModeArg::Off),
            wavelet::agent::PlanMode::Off
        );
        assert_eq!(
            wavelet::agent::PlanMode::from(PlanModeArg::Shadow),
            wavelet::agent::PlanMode::Shadow
        );
        assert_eq!(
            wavelet::agent::PlanMode::from(PlanModeArg::On),
            wavelet::agent::PlanMode::On
        );
    }

    #[test]
    fn invalid_plan_mode_value_is_rejected() {
        let err = Cli::try_parse_from(["wavelet", "agent", "chat", "--plan-mode", "bogus"]);
        assert!(err.is_err());
    }
}

use wavelet::handlers::util::{UpscaleTarget, UpscaleModel};
