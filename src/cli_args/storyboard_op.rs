//! StoryboardOp — extracted from the wavelet CLI clap definitions.

use std::path::PathBuf;
use clap::{Parser, Subcommand, ValueEnum, Args};



#[derive(Subcommand)]
pub enum StoryboardOp {
    /// Plan a draft storyboard from a screenplay + velocity profile.
    Plan {
        /// Path to the Fountain source file.
        screenplay: PathBuf,
        /// Path to the velocity profile JSON.
        #[arg(long)]
        velocity: PathBuf,
        /// Target FPS for the eventual render. Default 30.
        #[arg(long, default_value_t = 30)]
        fps: u32,
        /// Output resolution as `WxH`. Default `1920x1080`. Ignored
        /// when `--aspect` is set — the aspect derives a 720-pixel
        /// short edge (1280x720, 720x1280, 720x720, …).
        #[arg(long, default_value = "1920x1080")]
        resolution: String,
        /// Aspect ratio (`16:9`, `9:16`, `1:1`, `4:5`, `21:9`). When
        /// set, overrides `--resolution` with the aspect's default
        /// dimensions. Unset means "follow `--resolution` literally"
        /// for backwards compatibility.
        #[arg(long)]
        aspect: Option<String>,
        /// Pretty-print the emitted JSON.
        #[arg(long)]
        pretty: bool,
        /// Optional output path. Without it, JSON goes to stdout.
        #[arg(short, long)]
        out: Option<PathBuf>,
        /// Optional pre-rendered music track. When provided, scene
        /// boundaries snap to detected onsets within ±0.3s.
        #[arg(long)]
        onsets: Option<PathBuf>,
        /// Disable onset snapping even when `--onsets` is given. Use
        /// when you want raw heuristic durations for comparison.
        #[arg(long)]
        no_snap: bool,
        /// Scale every shot's start + duration so the storyboard total
        /// hits this target in seconds. Use when the brief's RUNTIME
        /// line doesn't naturally fall out of velocity-driven shot
        /// pacing. Typically set to the brief's RUNTIME value verbatim.
        #[arg(long)]
        match_runtime: Option<f32>,
        /// Project workdir — used to auto-load character refs from
        /// `<workdir>/refs/character/`. When a Dialogue scene's
        /// CHARACTER cue matches a loaded ref, the shot is routed
        /// through `fal-veo3-ref` instead of stock-search. Defaults to
        /// the directory containing the screenplay.
        #[arg(long)]
        workdir: Option<PathBuf>,
        /// Disable character-ref auto-loading even when `--workdir`
        /// resolves a populated `refs/character/` directory. Use when
        /// you want raw stock-search defaults (e.g. for diffs against
        /// pre-character-refs baselines).
        #[arg(long)]
        no_characters: bool,
    },
    /// Run structural verification gates over a storyboard. Reports
    /// errors + warnings; exit code 1 when any error is found.
    Verify {
        /// Path to the storyboard JSON.
        storyboard: PathBuf,
        /// Emit the report as JSON instead of human-readable text.
        #[arg(long)]
        json: bool,
    },
}
