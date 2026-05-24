//! Cmd — extracted from the wavelet CLI clap definitions.

use std::path::PathBuf;
use clap::{Parser, Subcommand, ValueEnum, Args};
use super::{AgentOp, BriefOp, C2paOp, CaptionsOp, CharacterOp, ClipOp, ContinuityOp, DialogueOp, DirectorOp, ImageOp, LintOp, MusicOp, PipelinesOp, ScreenplayOp, ShaderOp, ShotOp, StoryboardOp, TransitionsOp, VelocityOp, WorkflowOp};



#[derive(Subcommand)]
pub enum Cmd {
    /// Render a `commercial.html` composition manifest to MP4 (+ sidecar
    /// WAV if audio cues are present). HTML-only — JSON inputs are
    /// rejected with exit 3.
    Render {
        /// Path to the `commercial.html` manifest. Must declare
        /// `<meta name="resolution" content="WxH">`,
        /// `<meta name="fps" content="N">`,
        /// `<meta name="duration" content="Ns">`, and reference each
        /// scene via `<section data-scene-href="scenes/01-foo.html">`.
        comp: PathBuf,
        /// Output MP4 path. Defaults to <comp-stem>.mp4 alongside the input.
        #[arg(short, long)]
        out: Option<PathBuf>,
        /// Embed a signed C2PA content-credentials manifest in the output MP4.
        /// EU AI Act Article 50 enforcement (Aug 2026) requires this for any
        /// commercial AI-generated deliverable. Default OFF in v0; will flip
        /// to default ON closer to the deadline. Uses the bundled test cert
        /// unless --signing-key is provided.
        #[arg(long)]
        sign_c2pa: bool,
        /// Optional title for the C2PA CreativeWork assertion. Defaults to
        /// the composition file stem.
        #[arg(long, requires = "sign_c2pa")]
        title: Option<String>,
        /// Optional author for the C2PA CreativeWork assertion.
        #[arg(long, requires = "sign_c2pa")]
        author: Option<String>,
        /// Path to the backend cache root used to enumerate ingredients.
        /// Defaults to `.wavelet-cache/` relative to the composition file.
        #[arg(long, requires = "sign_c2pa")]
        cache_root: Option<PathBuf>,
        /// PEM-encoded signing-cert chain (overrides the bundled test cert).
        /// Required for production signing per the EU AI Act deadline.
        #[arg(long, requires = "signing_key")]
        signing_cert: Option<PathBuf>,
        /// PEM-encoded private key matching `--signing-cert`.
        #[arg(long, requires = "signing_cert")]
        signing_key: Option<PathBuf>,
        /// Comma-separated aspect ratios. When set, the render loop
        /// produces one MP4 per aspect ratio (e.g.
        /// `--aspects 16:9,9:16,1:1` → `<stem>.16x9.mp4 / .9x16.mp4
        /// / .1x1.mp4`) with the comp resolution overridden per pass.
        /// Same scene-stills + audio are reused — for aspect-aware
        /// gen, re-run `wavelet image scene-still` with the target
        /// `--image-size` first.
        #[arg(long, value_delimiter = ',')]
        aspects: Vec<String>,
        /// Per-frame wall-clock budget in seconds. If any single frame
        /// takes longer than this, render aborts with FrameBudgetExceeded.
        /// Default 30s; catches pathological CSS / decode hangs without
        /// eating the eval-level timeout. Bump higher for legitimate
        /// long-running comps (heavy shader passes, high-res renders).
        #[arg(long, default_value_t = crate::render_offline::DEFAULT_FRAME_BUDGET_SECS)]
        frame_budget_secs: u64,
        /// Skip the audio mux pass. By default any `<audio>` reference
        /// in the HTML is encoded to AAC and muxed into the output MP4
        /// alongside the video. Use --no-audio when the caller wants to
        /// mux audio manually downstream — the sidecar WAV is still
        /// written so the muxer has something to consume.
        #[arg(long)]
        no_audio: bool,
    },
    /// Lint a composition. Without --deep, only structural checks run.
    Verify {
        /// Path to the JSON composition file.
        comp: PathBuf,
        /// Also render mid-frame of each scene + probe audio decode.
        #[arg(long)]
        deep: bool,
    },
    /// Query a composition at a given time. Scene-graph queries (Phase 1)
    /// answer from the resolved layout tree without touching pixels.
    Query {
        /// Path to the JSON composition file.
        comp: PathBuf,
        /// Time to query at. Forms accepted: `0.5s`, `frame:90`, `MM:SS`.
        /// Defaults to the midpoint of the composition.
        #[arg(long, default_value = "")]
        at: String,
        /// Get the bbox of an element. CSS-id selector (e.g. `#headline`).
        #[arg(long)]
        bbox: Option<String>,
        /// Get a structured visibility verdict for the element.
        #[arg(long)]
        visible: Option<String>,
        /// True when the element's bbox is inside the title-safe area.
        #[arg(long)]
        in_safe_area: Option<String>,
        /// Safe-area inset fraction (e.g. 0.1 for 10%). Defaults to 0.1.
        #[arg(long, default_value_t = 0.1, requires = "in_safe_area")]
        inset: f32,
        /// Check whether an element would inherit ancestor CSS transforms
        /// (catches the wb-b53k painter bug pattern).
        #[arg(long)]
        transform_inherits: Option<String>,
        /// Lint: detect any pair of text-bearing elements whose bboxes
        /// intersect (typical symptom of negative margins, absolute-position
        /// math errors, etc.). Excludes ancestor-descendant pairs.
        #[arg(long)]
        no_overlap: bool,
        /// Sample one pixel's color. Format: `x,y`.
        #[arg(long, value_name = "X,Y")]
        color_at: Option<String>,
        /// Check that an element's mean color is within ΔE of a target.
        /// Format: `<selector>=<hex>` (e.g. `#headline=#ffffff`).
        #[arg(long, value_name = "SEL=HEX")]
        color_in: Option<String>,
        /// CIEDE2000 max ΔE for `--color-in`. Default 5.
        #[arg(long, default_value_t = 5.0, requires = "color_in")]
        max_de: f32,
        /// WCAG contrast-ratio check on a selector vs its surround ring.
        #[arg(long)]
        contrast: Option<String>,
        /// Contrast threshold (default 4.5 = WCAG AA normal text).
        #[arg(long, default_value_t = 4.5, requires = "contrast")]
        contrast_threshold: f32,
        /// Banding-detection on a region. Format: `x,y,w,h`.
        #[arg(long, value_name = "X,Y,W,H")]
        banding: Option<String>,
        /// Detect onsets in an audio file and score every composition event
        /// (scene starts, motion start_at, audio cue starts) against the
        /// nearest onset. Argument is the audio file path.
        #[arg(long, value_name = "AUDIO")]
        on_beat: Option<PathBuf>,
        /// Tolerance in ms for `--on-beat` event-vs-onset alignment.
        /// Default 33ms (= one frame at 30fps).
        #[arg(long, default_value_t = 33, requires = "on_beat")]
        tolerance_ms: u32,
        /// OCR the rendered frame and check that `<text>` appears in the
        /// element's pixels. Use --in to crop to a selector for ~10x
        /// speedup. Tolerance is allowed Levenshtein edits.
        #[arg(long, value_name = "TEXT")]
        text_visible: Option<String>,
        /// Crop OCR to this selector's bbox.
        #[arg(long, value_name = "SELECTOR", requires = "text_visible")]
        text_in: Option<String>,
        /// Allowed Levenshtein edit distance for --text-visible. Default 2
        /// (tesseract on rendered text is usually ≤1 errors).
        #[arg(long, default_value_t = 2, requires = "text_visible")]
        text_tolerance: u32,
        /// Output the FrameSnapshot itself as JSON (debug aid for agents).
        #[arg(long)]
        snapshot: bool,
        /// Suppress text output; emit a single JSON object instead.
        #[arg(long, default_value_t = true)]
        json: bool,
        /// Enter NDJSON REPL mode — one command per stdin line, one response
        /// per stdout line, with FrameSnapshot/FramePixels caches keyed by
        /// frame index. Ignores all other --flags; commands come from stdin.
        #[arg(long, conflicts_with_all = ["bbox", "visible", "color_at", "text_visible"])]
        repl: bool,
    },
    /// Per-frame diff between two rendered MP4s. Pure-Rust pixelmatch or
    /// SSIM; reports per-frame entries + median/p95/worst-frame stats.
    Diff {
        /// First (baseline) MP4.
        a: PathBuf,
        /// Second (candidate) MP4.
        b: PathBuf,
        /// Metric: `pixelmatch` or `ssim`. Default ssim.
        #[arg(long, default_value = "ssim")]
        metric: String,
        /// Per-frame fail threshold. For pixelmatch this is the fraction
        /// of differing pixels; for ssim it's 1-mean_ssim. Default 0.05.
        #[arg(long, default_value_t = 0.05)]
        threshold: f32,
        /// Clip both frames to this region before diffing. Format: `x,y,w,h`.
        #[arg(long, value_name = "X,Y,W,H")]
        clip: Option<String>,
        /// Whole-video budget — fail if more than this fraction of frames
        /// fail. Default 0.0 (any failure fails the comparison).
        #[arg(long, default_value_t = 0.0)]
        max_diff_ratio: f32,
        /// Write the full per-frame JSON report to this path. Otherwise
        /// only the aggregate stats are printed.
        #[arg(long, value_name = "PATH")]
        report: Option<PathBuf>,
    },
    /// Shader pipeline tools (Phase 7+ — not yet implemented).
    Shader {
        #[command(subcommand)]
        op: ShaderOp,
    },
    /// Screenplay tools — parse a Fountain (.fountain) file into a
    /// structured AST that the storyboard/velocity proposers consume.
    Screenplay {
        #[command(subcommand)]
        op: ScreenplayOp,
    },
    /// Velocity-profile tools — propose a BPM curve from a screenplay,
    /// validate it against a music track's detected BPM, and render the
    /// curve as SVG. Per the screenplay-to-MP4 PRD §2.
    Velocity {
        #[command(subcommand)]
        op: VelocityOp,
    },
    /// Storyboard tools — plan a draft storyboard from a screenplay +
    /// velocity profile (no LLM; agent fills in via separate skill) and
    /// run structural verification gates. Per the PRD §5–6.
    Storyboard {
        #[command(subcommand)]
        op: StoryboardOp,
    },
    /// Per-cut continuity analysis: 180°-rule + motion-vector continuity
    /// + shot-type rhythm + scale-jump detection. Emits a structured
    /// report per cut.
    Continuity {
        #[command(subcommand)]
        op: ContinuityOp,
    },
    /// Transition tools — classify Fountain transitions against a
    /// velocity profile, filling in durations, whip-pan directions, and
    /// J/L-cut lead/trail times.
    Transitions {
        #[command(subcommand)]
        op: TransitionsOp,
    },
    /// Shot-generation backends — stock search, txt2vid (Veo), still
    /// (Flux Schnell), surgical fix (Flux Kontext Max), upscale (SUPIR).
    /// All adapters share `--dry-run` and `--max-cost` flags so the
    /// agent can preview spend before committing.
    Shot {
        #[command(subcommand)]
        op: ShotOp,
    },
    /// Dialogue / voice tools — TTS, voice conversion, transcription
    /// (per Phase 3+6 of the screenplay-to-MP4 epic).
    Dialogue {
        #[command(subcommand)]
        op: DialogueOp,
    },
    /// Word-level caption tools — alignment + HTML overlay generator
    /// for CapCut / Hormozi / minimal styles (wb-suvz).
    Captions {
        #[command(subcommand)]
        op: CaptionsOp,
    },
    /// Image processing — background removal, conditioning prep.
    Image {
        #[command(subcommand)]
        op: ImageOp,
    },
    /// Music generation — reference-conditioned. Default backend is
    /// `elevenlabs` (Merlin+Kobalt-licensed, commercial-safe). `udio`
    /// is wired as an alternative (partnership tier).
    Music {
        #[command(subcommand)]
        op: MusicOp,
    },
    /// Ad-creative brief tools — parse and validate the 9-line brief
    /// (PRODUCT / AUDIENCE / INSIGHT / PROMISE / PROOF / TONE / MUSIC /
    /// CALL / RUNTIME) the wavelet-director skill emits before screenplay
    /// generation.
    Brief {
        #[command(subcommand)]
        op: BriefOp,
    },
    /// LLM-as-creative-director — populates L-Storyboard shot
    /// attributes (subject / action / scene / camera / lens / lighting
    /// / style) for every shot via one LLM call. Replaces template
    /// prompt assembly. Per the May-2026 ComfyUI commercial-workflow
    /// audit, 9 of 15 production pipelines do this.
    Director {
        #[command(subcommand)]
        op: DirectorOp,
    },
    /// C2PA content-credentials operations: sign a finished MP4, verify a
    /// signed one. EU AI Act Article 50 enforcement begins Aug 2026.
    C2pa {
        #[command(subcommand)]
        op: C2paOp,
    },
    /// Declarative pipeline definitions — list / show / validate / run
    /// the `*.yaml` files under `packages/wavelet/pipeline_defs/`. Each
    /// YAML declares stages with required input/output artifacts, the
    /// tools each stage may call, success criteria, and orchestration
    /// caps (budget, retries, wall-time). The `run` verb is a stub in
    /// this release — it prints the resolved execution plan; the live
    /// runtime lands with `wavelet workflow run` (wb-oemp).
    Pipelines {
        #[command(subcommand)]
        op: PipelinesOp,
    },
    /// Cooperative pipeline runner — walks a pipeline against a working
    /// directory and reports the next stage to run. Stage completion is
    /// inferred from `required_artifacts_out` appearing on disk. Run
    /// repeatedly while the agent (or a human) fills in artifacts.
    Workflow {
        #[command(subcommand)]
        op: WorkflowOp,
    },
    /// Clip-ref inspection — list, show, and walk the lineage of
    /// `.clip.html` files emitted by wavelet producers (wb-n33n).
    Clip {
        #[command(subcommand)]
        op: ClipOp,
    },
    /// Lip-sync — graft an audio track onto a driving video so a
    /// speaker's mouth matches the new dialogue. Default backend is
    /// `sync-lipsync-2-pro` on Replicate (sync.so's studio tier).
    /// Hedra Character-3 is the SOTA phoneme-accurate alternative but
    /// isn't currently on Fal or Replicate as of May 2026.
    Lipsync {
        /// Driving video — local path or HTTPS URL.
        #[arg(long)]
        video: String,
        /// Replacement audio — local path or HTTPS URL.
        #[arg(long)]
        audio: String,
        /// Backend identifier. Default `sync-lipsync-2-pro`.
        #[arg(long, default_value = "sync-lipsync-2-pro")]
        backend: String,
        /// Optional sync mode string (provider-defined vocabulary).
        #[arg(long)]
        sync_mode: Option<String>,
        /// Optional generation temperature.
        #[arg(long)]
        temperature: Option<f32>,
        /// Hint that only one face in the frame is speaking.
        #[arg(long)]
        active_speaker: Option<bool>,
        /// Emit the request spec without dispatching the backend call.
        #[arg(long)]
        dry_run: bool,
        /// Max USD spend permitted.
        #[arg(long, default_value_t = 1.0)]
        max_cost: f32,
        /// Cache root.
        #[arg(long, default_value = ".wavelet-cache")]
        cache: PathBuf,
        /// Optional output path. Without it, the cached MP4 stays in `.wavelet-cache/`.
        #[arg(short, long)]
        out: Option<PathBuf>,
        /// Pretty-print the emitted JSON.
        #[arg(long)]
        pretty: bool,
    },
    /// Run one of the five starter shader assertions against a PNG
    /// frame and emit `{pass, score, metrics, shader, elapsed_us}` JSON
    /// on stdout. Exit code 0 on pass, 1 on fail. Spelled
    /// `query-shader` (kebab) because the existing `query` is a flat
    /// flag-driven verb, not a subcommand parent. The ticket's
    /// `wavelet query shader` reads as the same thing.
    /// TODO: resolve `--frame ctx:hero.mp4@t=0.5` URIs once the render
    /// integration lands; for v1 `--frame` is a PNG path.
    #[command(name = "query-shader")]
    QueryShader {
        /// Shader name. One of: contrast_in_region, motion_magnitude,
        /// golden_rmse, sobel_edge_density, color_band_mean.
        #[arg(long)]
        shader: String,
        /// Path to the PNG frame the assertion runs against.
        #[arg(long)]
        frame: PathBuf,
        /// Shader-specific params, as a JSON object. Schema is
        /// per-shader; see `agent/plan/validators/shader.rs` for the
        /// expected fields.
        #[arg(long, default_value = "{}")]
        params: String,
    },
    /// `wavelet agent` — Gemini-native agent loop. Two frontends share one
    /// orchestrator: `chat` runs an interactive REPL on stdin/stdout;
    /// `serve --port N` binds a JSON-RPC 2.0 WebSocket server.
    Agent {
        #[command(subcommand)]
        op: AgentOp,
    },
    /// `wavelet lint` — layout-walk lint rules. v1 ships `safe-zone`
    /// only. Walks every scene's resolved DOM and reports findings
    /// whose remediation hint is concrete enough for the agent to
    /// act on without a re-render.
    Lint(LintOp),
    /// Character reference bundles — define a named character with 1..N
    /// reference images. The storyboard planner auto-discovers these
    /// refs and routes matching CHARACTER cues through `fal-veo3-ref`.
    Character {
        #[command(subcommand)]
        op: CharacterOp,
    },
}
