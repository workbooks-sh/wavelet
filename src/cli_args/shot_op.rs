//! ShotOp — extracted from the wavelet CLI clap definitions.

use std::path::PathBuf;
use clap::{Parser, Subcommand, ValueEnum, Args};



#[derive(Subcommand)]
pub enum ShotOp {
    /// Text-to-video generation via Google Veo (`Txt2VidGen` cluster).
    /// Cached by request hash so identical re-requests skip the API and
    /// return the same clip.
    #[command(name = "txt2vid")]
    Txt2Vid {
        /// Generation prompt.
        prompt: String,
        /// Backend identifier. When unset, resolves from
        /// `wavelet.config.toml` (workdir → user-global → tool default
        /// `veo`). See `crate::config::cascade` and wb-e90g.
        #[arg(long)]
        backend: Option<String>,
        /// Desired clip duration in seconds. Models clamp to their
        /// max length — Veo typically returns ~5s.
        #[arg(long, default_value_t = 5.0)]
        duration: f32,
        /// Aspect ratio (`16:9`, `9:16`, `1:1`).
        #[arg(long, default_value = "16:9")]
        aspect: String,
        /// Negative prompt (things to avoid). Merged with the canonical
        /// default negatives (wb-ynn0) unless `--no-default-negatives`
        /// is set.
        #[arg(long)]
        negative: Option<String>,
        /// Skip the canonical default negative prompt that's normally
        /// appended to every gen call (wb-ynn0). Use for adversarial
        /// experiments only — the default reduces unusable outputs by
        /// ~30% per Artlist's documented number.
        #[arg(long)]
        no_default_negatives: bool,
        /// Random seed for reproducibility.
        #[arg(long)]
        seed: Option<u64>,
        /// Number of variants to roll (1-8). Default 1. Variant winner
        /// for clips is picked by `--select first` or `cheapest`;
        /// `max-vlm` falls back to `first` since clip-frame VLM grading
        /// is not wired in this pass.
        #[arg(long, default_value_t = 1)]
        variants: u32,
        /// Variant selection policy. `max-vlm` (clip → falls back to
        /// `first`), `first`, `cheapest`, `user`.
        #[arg(long, default_value = "max-vlm")]
        select: String,
        /// Aggregate USD ceiling across all N variants.
        #[arg(long)]
        max_variants_cost: Option<f32>,
        /// Emit the request spec without hitting the API.
        #[arg(long)]
        dry_run: bool,
        /// Maximum USD spend permitted. Default 0.0 — refuses paid
        /// requests until raised.
        #[arg(long, default_value_t = 0.0)]
        max_cost: f32,
        /// Cache root.
        #[arg(long, default_value = ".wavelet-cache")]
        cache: PathBuf,
        /// Optional destination path. When set, the cached MP4 is
        /// copied here.
        #[arg(short, long)]
        out: Option<PathBuf>,
        /// Pretty-print the emitted JSON.
        #[arg(long)]
        pretty: bool,
    },
    /// Text-to-image generation (`Txt2Img` cluster, Google nano-banana-3
    /// primary; Fal Flux Schnell available as alternate). Used for
    /// environment plates in Path B — the backdrop the isolated
    /// subject gets composited over.
    Still {
        /// Generation prompt. Be specific about composition + lighting.
        prompt: String,
        /// Backend. When unset, resolves from the cascade (image slot →
        /// tool default `google-nano-banana-3`).
        #[arg(long)]
        backend: Option<String>,
        /// Image-size hint (`landscape_16_9`, `square_hd`, `portrait_4_3`).
        #[arg(long, default_value = "landscape_16_9")]
        image_size: String,
        /// Random seed.
        #[arg(long)]
        seed: Option<u64>,
        /// Number of variants to roll (1-8). Default 1.
        #[arg(long, default_value_t = 1)]
        variants: u32,
        /// Variant selection policy. `max-vlm` (default), `first`,
        /// `cheapest`, `user`.
        #[arg(long, default_value = "max-vlm")]
        select: String,
        /// Comma-separated VLM verification criteria for `max-vlm`.
        #[arg(long, value_delimiter = ',')]
        criteria: Vec<String>,
        /// Aggregate USD ceiling across all N variants.
        #[arg(long)]
        max_variants_cost: Option<f32>,
        /// Emit the request spec without hitting the API.
        #[arg(long)]
        dry_run: bool,
        /// Maximum USD spend.
        #[arg(long, default_value_t = 0.0)]
        max_cost: f32,
        /// Cache root.
        #[arg(long, default_value = ".wavelet-cache")]
        cache: PathBuf,
        /// Optional destination path for the JPG.
        #[arg(short, long)]
        out: Option<PathBuf>,
        /// Pretty-print the emitted JSON.
        #[arg(long)]
        pretty: bool,
    },
    /// Surgical instruction-edit — preserve everything the model isn't
    /// asked to touch, rewrite just the offending element. Use when
    /// `verify-shot` flags one specific failure ("badge wrong color",
    /// "BUY NWO misspelling") instead of re-rolling the whole frame.
    /// Backed by Fal Flux Pro Kontext Max (~$0.04-0.08/call). The
    /// `--region` flag bakes a coarse location hint into the instruction
    /// text (Kontext Max is instruction-only — masks are accepted on
    /// the wire but currently ignored).
    Fix {
        /// URL or local path to the source image. Local paths are
        /// converted to `data:` URLs via the shared helper.
        #[arg(long)]
        input: String,
        /// The edit instruction. End with "leave everything else
        /// unchanged" — that phrase is load-bearing.
        #[arg(long)]
        instruction: String,
        /// Optional region as `X,Y,W,H` in pixel coords. When set, a
        /// location hint ("in the upper-right region of the image") is
        /// appended to the instruction. The source's dimensions are
        /// auto-detected when `--input` is a local path; pass
        /// `--image-w` / `--image-h` for remote URLs.
        #[arg(long, value_name = "X,Y,W,H")]
        region: Option<String>,
        /// Source image width (only needed when `--region` is given and
        /// `--input` is a remote URL).
        #[arg(long)]
        image_w: Option<u32>,
        /// Source image height (only needed when `--region` is given and
        /// `--input` is a remote URL).
        #[arg(long)]
        image_h: Option<u32>,
        /// Guidance scale (>= 1.0). Kontext default is provider-side.
        #[arg(long)]
        guidance: Option<f32>,
        /// Inference steps (<= 50).
        #[arg(long)]
        steps: Option<u32>,
        /// Random seed.
        #[arg(long)]
        seed: Option<u64>,
        /// Backend. Currently `fal-flux-kontext-max`.
        #[arg(long, default_value = "fal-flux-kontext-max")]
        backend: String,
        /// Emit the request spec without hitting the API.
        #[arg(long)]
        dry_run: bool,
        /// Maximum USD spend permitted.
        #[arg(long, default_value_t = 0.0)]
        max_cost: f32,
        /// Cache root.
        #[arg(long, default_value = ".wavelet-cache")]
        cache: PathBuf,
        /// Optional destination path for the edited image.
        #[arg(short, long)]
        out: Option<PathBuf>,
        /// Pretty-print the emitted JSON.
        #[arg(long)]
        pretty: bool,
    },
    /// Chain surgical edits from a `verify-shot` report. Reads the
    /// JSON, picks every `Fail` finding, converts each to a Kontext
    /// instruction, applies them sequentially (output of N feeds the
    /// input of N+1), and reports the per-step path + final image.
    /// Does NOT re-run verify at the end — that's a separate call so
    /// it can use a different backend/criteria set.
    #[command(name = "fix-from-verify")]
    FixFromVerify {
        /// URL or local path to the source image.
        #[arg(long)]
        input: String,
        /// Path to the JSON report produced by `wavelet image verify-shot`.
        #[arg(long = "verify-report")]
        verify_report: PathBuf,
        /// Backend. Currently `fal-flux-kontext-max`.
        #[arg(long, default_value = "fal-flux-kontext-max")]
        backend: String,
        /// Emit the planned edit chain without hitting the API.
        #[arg(long)]
        dry_run: bool,
        /// Maximum USD spend (covers all chained edits combined).
        #[arg(long, default_value_t = 0.0)]
        max_cost: f32,
        /// Cache root.
        #[arg(long, default_value = ".wavelet-cache")]
        cache: PathBuf,
        /// Optional destination for the final image.
        #[arg(short, long)]
        out: Option<PathBuf>,
        /// Pretty-print the emitted JSON.
        #[arg(long)]
        pretty: bool,
    },
    /// Stock-footage search across cluster providers (Pexels primary,
    /// Pond5 fallback).
    Search {
        /// Search query.
        query: String,
        /// Backend identifier (`pexels` or `pond5`). Default `pexels`.
        #[arg(long, default_value = "pexels")]
        backend: String,
        /// Filter by orientation.
        #[arg(long, value_name = "landscape|portrait|square")]
        orientation: Option<String>,
        /// Minimum clip duration in seconds.
        #[arg(long, value_name = "SECS")]
        min_duration: Option<u32>,
        /// Maximum clip duration in seconds.
        #[arg(long, value_name = "SECS")]
        max_duration: Option<u32>,
        /// Items per page (clamped to provider's max).
        #[arg(long, default_value_t = 15)]
        per_page: u32,
        /// 1-based page index.
        #[arg(long, default_value_t = 1)]
        page: u32,
        /// Emit the request spec without hitting the API.
        #[arg(long)]
        dry_run: bool,
        /// Maximum USD spend permitted. Default 0.0 — refuses any paid
        /// request. Pexels is free; this only matters for paid clusters.
        #[arg(long, default_value_t = 0.0)]
        max_cost: f32,
        /// Cache root. Manifests + (future) downloaded assets land here.
        #[arg(long, default_value = ".wavelet-cache")]
        cache: PathBuf,
        /// Pretty-print the emitted JSON.
        #[arg(long)]
        pretty: bool,
    },
    /// Final-pass upscale — the single largest perceived-quality lever
    /// in the production pipeline (ComfyUI workflow audit, 6/15
    /// surveyed workflows include one). Routes still images through SUPIR.
    Upscale {
        /// Source asset — local path or URL. `.png`/`.jpg`/`.webp`
        /// route to SUPIR.
        #[arg(value_name = "INPUT")]
        input: String,
        /// Adapter to use. `auto` (default) picks by input extension;
        /// the only surviving model is `supir`.
        #[arg(long, default_value = "auto", value_name = "supir|auto")]
        model: String,
        /// Target spec — `2x`, `4x`, `1080p`, `4k`. Default `2x`.
        #[arg(long, default_value = "2x")]
        target: String,
        /// Emit the request spec without hitting the API.
        #[arg(long)]
        dry_run: bool,
        /// Maximum USD spend.
        #[arg(long, default_value_t = 0.0)]
        max_cost: f32,
        /// Cache root.
        #[arg(long, default_value = ".wavelet-cache")]
        cache: PathBuf,
        /// Optional destination path for the upscaled output.
        #[arg(short, long)]
        out: Option<PathBuf>,
        /// Pretty-print the emitted JSON.
        #[arg(long)]
        pretty: bool,
    },
    /// Drop a product into a scene with lighting integration. Wraps
    /// the three-step Insert-Anything pattern the wb-j1ef.2 spike
    /// validated: local horizontal concat of `<product>` + `<scene>`,
    /// a single Fal Kontext Max call with the spike's load-bearing
    /// instruction, then a Roboflow-CLIP identity-similarity gate
    /// against the original product (Kontext tends to lightly repaint
    /// the subject).
    ///
    /// Cost band: ~$0.08 per call (Kontext only; concat + identity
    /// check are free / cheap). Latency band: ~12s wall.
    ///
    /// Use this verb when the goal is "place my product in this
    /// environment with believable light + shadow." When pixel-
    /// identical subject preservation matters, use `image composite`
    /// against a pre-removed background instead.
    #[command(name = "insert-into-scene")]
    InsertIntoScene {
        /// Product source — local path or URL. Becomes the LEFT half
        /// of the concat input. Alpha is composited onto white before
        /// concat so Kontext doesn't see a transparent hole.
        #[arg(long)]
        product: String,
        /// Scene source — local path or URL. Becomes the RIGHT half
        /// of the concat input. The instruction tells Kontext to
        /// leave this half's composition unchanged.
        #[arg(long)]
        scene: String,
        /// Threshold for the post-Kontext identity-similarity gate
        /// (cosine sim, 0..1). The spike noted subtle case-shape +
        /// dial-repaint drift on a wristwatch; `0.70` is the
        /// conservative default that catches that while passing
        /// faithful renders.
        #[arg(long, default_value_t = 0.70)]
        threshold: f32,
        /// Re-roll up to N times (cap 3) with bumped seeds when the
        /// identity gate fails. Without this flag, drift is reported
        /// as a warning and the result is returned anyway — the
        /// spike's stance: production-safe behavior is opt-in.
        #[arg(long)]
        strict_identity: bool,
        /// Optional random seed for the initial Kontext call. Re-rolls
        /// (with `--strict-identity`) bump from here.
        #[arg(long)]
        seed: Option<u64>,
        /// Backend for the merge step. Currently `fal-flux-kontext-max`.
        #[arg(long, default_value = "fal-flux-kontext-max")]
        backend: String,
        /// Backend for the identity-similarity gate.
        /// `fal-clip-similarity` (default) accepts both HTTPS and
        /// `data:` URLs, so local-file products work without a public
        /// upload. `roboflow-clip` is HTTPS-only.
        #[arg(long, default_value = "fal-clip-similarity")]
        identity_backend: String,
        /// Emit the planned request without hitting any API.
        #[arg(long)]
        dry_run: bool,
        /// Maximum USD spend (covers Kontext + every re-roll combined).
        #[arg(long, default_value_t = 0.0)]
        max_cost: f32,
        /// Cache root.
        #[arg(long, default_value = ".wavelet-cache")]
        cache: PathBuf,
        /// Output path for the merged PNG.
        #[arg(short, long)]
        out: Option<PathBuf>,
        /// Pretty-print the emitted JSON.
        #[arg(long)]
        pretty: bool,
    },
    /// Model-as-planner / agent-as-loop video edit. Decomposes a
    /// natural-language intent into a typed plan, executes it, then
    /// asks a reviewer model whether the result actually fulfills
    /// the intent. Retries with the reviewer's critique up to
    /// `--max-attempts` / `--max-cost`. See wb-ft0o for the design.
    Edit {
        /// Input — either an `.mp4` (rendered shot) or a scene `.html`
        /// (wavelet scene file). For mp4 inputs, the executor will try
        /// to locate a sibling scene HTML for the CSS-only path.
        input: PathBuf,
        /// Natural-language edit instruction. Required.
        #[arg(long)]
        intent: String,
        /// Output path. Default: `<input-stem>-edited.mp4` alongside
        /// the input.
        #[arg(long)]
        out: Option<PathBuf>,
        /// Report JSON output path. Default: `<input-stem>-edit-report.json`.
        #[arg(long)]
        report: Option<PathBuf>,
        /// Maximum plan→execute→review attempts.
        #[arg(long, default_value_t = 3)]
        max_attempts: u32,
        /// Aggregate USD budget across all attempts.
        #[arg(long, default_value_t = 0.50)]
        max_cost: f32,
        /// Reviewer score (0..1) at which a result is shipped.
        #[arg(long, default_value_t = 0.7)]
        pass_threshold: f32,
        /// Gemini model slug for the planner.
        #[arg(long, default_value = "gemini-3.1-pro-preview")]
        planner_model: String,
        /// Gemini model slug for the reviewer.
        #[arg(long, default_value = "gemini-3.5-flash")]
        reviewer_model: String,
        /// Plan only — emit the plan JSON to stdout and exit.
        #[arg(long)]
        dry_run: bool,
    },
}
