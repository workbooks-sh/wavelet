//! CLI argument types extracted from the wavelet binary. Each
//! clap-derive enum / struct lives in its own file to keep file size
//! under control. mod.rs re-exports everything so callers can keep
//! the existing `use crate::cli_args::Foo` shape.

#![allow(missing_docs)]

pub mod cmd;
pub mod plan_mode_arg;
pub mod agent_op;
pub mod workflow_op;
pub mod pipelines_op;
pub mod c2pa_op;
pub mod director_op;
pub mod shader_op;
pub mod screenplay_op;
pub mod clip_op;
pub mod velocity_op;
pub mod storyboard_op;
pub mod continuity_op;
pub mod transitions_op;
pub mod music_op;
pub mod brief_op;
pub mod dialogue_op;
pub mod captions_op;
pub mod image_op;
pub mod shot_op;
pub mod lint_op;

pub use cmd::Cmd;
pub use plan_mode_arg::PlanModeArg;
pub use agent_op::AgentOp;
pub use workflow_op::WorkflowOp;
pub use pipelines_op::PipelinesOp;
pub use c2pa_op::C2paOp;
pub use director_op::DirectorOp;
pub use shader_op::ShaderOp;
pub use screenplay_op::ScreenplayOp;
pub use clip_op::ClipOp;
pub use velocity_op::VelocityOp;
pub use storyboard_op::StoryboardOp;
pub use continuity_op::ContinuityOp;
pub use transitions_op::TransitionsOp;
pub use music_op::MusicOp;
pub use brief_op::BriefOp;
pub use dialogue_op::DialogueOp;
pub use captions_op::CaptionsOp;
pub use image_op::ImageOp;
pub use shot_op::ShotOp;
pub use lint_op::LintOp;
