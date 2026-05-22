//! ImageOp — extracted from the wavelet CLI clap definitions.

use std::path::PathBuf;
use clap::{Parser, Subcommand, ValueEnum, Args};



#[derive(Subcommand)]
pub enum ImageOp {
    /// Remove the background from an image. Returns a PNG with alpha.
    #[command(name = "bg-remove")]
    BgRemove {
        /// URL or local path to the source image.
        image: String,
        /// Backend identifier. Currently `fal-birefnet`.
        #[arg(long, default_value = "fal-birefnet")]
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
        /// Optional destination path.
        #[arg(short, long)]
        out: Option<PathBuf>,
        /// Pretty-print the emitted JSON.
        #[arg(long)]
        pretty: bool,
    },
    /// Text-prompted segmentation — isolate the named subject from the
    /// image. Use this instead of `bg-remove` when the source photo has
    /// other people, watermarks, or cars that shouldn't be kept.
    Isolate {
        /// URL or local path to the source image. Must be a public URL
        /// or a local path readable by the CLI (data: URLs not supported
        /// for the source — pass `bg-remove` for that case).
        image: String,
        /// Text prompt naming what to keep (e.g. `"the car"`,
        /// `"the watch"`, `"the person on the left"`).
        #[arg(long)]
        prompt: String,
        /// Backend. Currently `fal-evf-sam`.
        #[arg(long, default_value = "fal-evf-sam")]
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
        /// Optional destination path.
        #[arg(short, long)]
        out: Option<PathBuf>,
        /// Pretty-print the emitted JSON.
        #[arg(long)]
        pretty: bool,
    },
    /// Negative-space scorer — rank grid cells by how clean they are
    /// for a text overlay (low edge density + low brightness variance).
    /// Pure-local Sobel + statistics; no backend call. Each cell ships
    /// a suggested text color and required scrim opacity for WCAG AA.
    #[command(name = "negative-space")]
    NegativeSpace {
        /// Path to the source PNG/JPG.
        image: PathBuf,
        /// Grid rows. Default 3.
        #[arg(long, default_value_t = 3)]
        rows: u32,
        /// Grid columns. Default 3.
        #[arg(long, default_value_t = 3)]
        cols: u32,
        /// Pretty-print the emitted JSON.
        #[arg(long)]
        pretty: bool,
    },
    /// Heuristic saliency map. Center-bias × edge-density per cell —
    /// not a trained model. Returns a `rows × cols` heatmap plus
    /// top-N attractor cells. Use the complement of these cells when
    /// placing text overlays.
    Saliency {
        /// Path to the source PNG/JPG.
        image: PathBuf,
        /// Grid rows. Default 9.
        #[arg(long, default_value_t = 9)]
        rows: u32,
        /// Grid columns. Default 9.
        #[arg(long, default_value_t = 9)]
        cols: u32,
        /// Number of attractor cells to return. Default 3.
        #[arg(long, default_value_t = 3)]
        top_n: usize,
        /// Pretty-print the emitted JSON.
        #[arg(long)]
        pretty: bool,
    },
    /// OCR the source image to detect baked-in text. Routes to a
    /// hosted provider (`roboflow-doctr`, default when `ROBOFLOW_API_KEY`
    /// is set) or the local stub (`rapidocr-local`, returns
    /// `Unimplemented` until a local ONNX OCR is wired).
    Ocr {
        /// URL or local path to the source PNG/JPG. Local paths are
        /// converted to `data:` URLs for the hosted backend; the local
        /// adapter accepts a path directly.
        image: String,
        /// Backend identifier. `roboflow-doctr` hits Roboflow's hosted
        /// doctr/ocr endpoint; `rapidocr-local` is a stub for an
        /// in-process ONNX OCR (not yet wired).
        #[arg(long)]
        backend: Option<String>,
        /// Emit the request spec without hitting the API.
        #[arg(long)]
        dry_run: bool,
        /// Maximum USD spend permitted on a live call.
        #[arg(long, default_value_t = 0.0)]
        max_cost: f32,
        /// Cache root.
        #[arg(long, default_value = ".wavelet-cache")]
        cache: PathBuf,
        /// Pretty-print the emitted JSON.
        #[arg(long)]
        pretty: bool,
    },
    /// WCAG contrast check for a candidate text region. Computes the
    /// mean luminance under the region, the WCAG ratio against the
    /// given text color, and (when below threshold) a scrim opacity
    /// that would lift it above the threshold.
    Contrast {
        /// Path to the source PNG/JPG.
        image: PathBuf,
        /// Region as `X,Y,W,H` in pixel coords.
        #[arg(long)]
        region: String,
        /// Text color as `#RGB` or `#RRGGBB`.
        #[arg(long)]
        text_color: String,
        /// WCAG threshold. Default 4.5 (AA for normal text).
        #[arg(long, default_value_t = 4.5)]
        threshold: f32,
        /// Pretty-print the emitted JSON.
        #[arg(long)]
        pretty: bool,
    },
    /// Build a render-time scrim plan as CSS custom properties.
    /// Combines negative-space + contrast: picks the cleanest grid cell
    /// for text, recommends a text color, picks the opposite color as
    /// the scrim, computes the opacity needed to clear `--threshold`,
    /// and emits a ready-to-paste `:root { … }` block of variables
    /// (`--scrim-color`, `--scrim-opacity`, `--text-color-recommended`,
    /// `--negative-space-{x,y,w,h}`). Scene HTML reads the vars instead
    /// of hard-coding scrim values per shot.
    Scrim {
        /// Path to the source PNG/JPG (typically the scene-still).
        image: PathBuf,
        /// Negative-space grid rows. Default 3.
        #[arg(long, default_value_t = 3)]
        rows: u32,
        /// Negative-space grid columns. Default 3.
        #[arg(long, default_value_t = 3)]
        cols: u32,
        /// WCAG threshold. Default 4.5 (AA for normal text).
        #[arg(long, default_value_t = 4.5)]
        threshold: f32,
        /// Optional output file. When set, writes the JSON report to
        /// the path; otherwise prints to stdout.
        #[arg(short, long)]
        out: Option<PathBuf>,
        /// Pretty-print the emitted JSON.
        #[arg(long)]
        pretty: bool,
    },
    /// Identity-similarity gate — verify that a generated still still
    /// shows the same subject as the master reference. Runs after
    /// every `scene-still` gen to catch drift (different car make,
    /// different watch face). Score is cosine similarity in
    /// embedding space; `--threshold` controls the pass/fail line.
    #[command(name = "identity-check")]
    IdentityCheck {
        /// URL or local path to the master reference image.
        #[arg(long)]
        reference: String,
        /// URL or local path to the candidate (generated) image.
        #[arg(long)]
        candidate: String,
        /// Pass threshold in `[0.0, 1.0]`. Default 0.85 — tuned for
        /// CLIP ViT-L/14 (products surviving the gen typically score
        /// 0.85+; drift falls to 0.60-0.75).
        #[arg(long, default_value_t = 0.85)]
        threshold: f32,
        /// Backend identifier. `roboflow-clip` (live, ViT-L/14, ~$0.001/check)
        /// or `fal-clip-similarity` (stub — returns Unimplemented in live mode
        /// until Fal publishes a CLIP endpoint).
        #[arg(long, default_value = "roboflow-clip")]
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
        /// Pretty-print the emitted JSON.
        #[arg(long)]
        pretty: bool,
    },
    /// Pure-local composite: overlay an isolated foreground (with
    /// alpha) on a background image, producing an RGBA PNG. No backend
    /// call. Used as Step 3 of Path B: isolated subject + env plate
    /// → composited frame, which then feeds into `wavelet shot img2vid`.
    Composite {
        /// Foreground image (the isolated subject with alpha channel).
        #[arg(long)]
        foreground: PathBuf,
        /// Background image (the environment plate).
        #[arg(long)]
        background: PathBuf,
        /// Output PNG path.
        #[arg(short, long)]
        out: PathBuf,
        /// Foreground scale relative to background height (0.0–1.0).
        /// Default 0.7 (subject fills 70% of frame height).
        #[arg(long, default_value_t = 0.7)]
        scale: f32,
        /// Vertical offset of the foreground's center as a fraction of
        /// bg height. Default 0.55 (slightly below center for cars).
        #[arg(long, default_value_t = 0.55)]
        y_offset: f32,
    },
    /// Pre-render verification gate — feed an image to a vision-language
    /// model and grade it against a list of yes/no criteria from the
    /// brief (subject identity, bystanders, baked-in watermarks). Used
    /// before paid render+mux so the agent doesn't ship a broken
    /// commercial. Backed by Fal `any-llm/vision` router (~$0.01/call).
    #[command(name = "verify-shot")]
    VerifyShot {
        /// URL or local path to the image to grade. Local paths are
        /// converted to `data:` URLs via the shared helper.
        image: String,
        /// One criterion. Repeat the flag for multiple. Phrase each as
        /// a positive claim: `--criteria "subject is a green Porsche"`.
        #[arg(long = "criteria")]
        criteria: Vec<String>,
        /// Backend identifier. Currently `fal-vision-verify`.
        #[arg(long, default_value = "fal-vision-verify")]
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
        /// Pretty-print the emitted JSON.
        #[arg(long)]
        pretty: bool,
    },
}
