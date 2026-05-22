//! Shared helpers used by CLI subcommand handlers — image argument
//! normalization, region parsing, structured-result emission.

use std::path::Path;
use serde::{Serialize, Deserialize};
use crate::query::{OverlapPair, ScoredEvent, VisibilityVerdict, Rect};
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Instant;
use crate::handlers::shot_insert_into_scene::run_shot_insert_into_scene;

/// Normalize an "image argument" into something Fal can consume.
///
/// - `http://` or `https://` URLs pass through unchanged.
/// - `data:` URLs pass through unchanged.
/// - Anything else is treated as a local file path: read the bytes,
///   detect the format via magic bytes, base64-encode, return a
///   `data:image/<format>;base64,<encoded>` URL.
///
/// This exists because subagents pass `--image <path>` and we don't
/// want them to bump into OS `ARG_MAX` (~1MB on macOS) by encoding to
/// base64 themselves. The adapters stay URL-agnostic; the CLI layer
/// does the path â data: URL conversion before any backend sees it.
pub fn image_arg_to_url(arg: &str) -> Result<String, String> {
    use crate::backends::util::{base64_encode, ext_to_mime, sniff_image_ext};

    if arg.is_empty() {
        return Err("image argument is empty".into());
    }
    if arg.starts_with("http://") || arg.starts_with("https://") || arg.starts_with("data:") {
        return Ok(arg.to_string());
    }

    let path = std::path::Path::new(arg);
    let bytes = std::fs::read(path)
        .map_err(|e| format!("read image file {}: {e}", path.display()))?;
    if bytes.is_empty() {
        return Err(format!("image file {} is empty", path.display()));
    }
    let ext = sniff_image_ext(&bytes);
    let mime = ext_to_mime(ext);
    let encoded = base64_encode(&bytes);
    Ok(format!("data:{mime};base64,{encoded}"))
}

/// Parse `X,Y,W,H` into a `BoundingRect`. Whitespace permitted.
pub fn parse_region(s: &str) -> Result<crate::image_analysis::BoundingRect, String> {
    let parts: Vec<&str> = s.split(',').map(|t| t.trim()).collect();
    if parts.len() != 4 {
        return Err(format!("region must be X,Y,W,H â got '{s}'"));
    }
    let x: u32 = parts[0].parse().map_err(|e| format!("x: {e}"))?;
    let y: u32 = parts[1].parse().map_err(|e| format!("y: {e}"))?;
    let w: u32 = parts[2].parse().map_err(|e| format!("w: {e}"))?;
    let h: u32 = parts[3].parse().map_err(|e| format!("h: {e}"))?;
    Ok(crate::image_analysis::BoundingRect::new(x, y, w, h))
}

/// Run an image-analysis closure and emit the `{ok, result, exec_ms}`
/// JSON envelope that all four analyses share.
pub fn emit_analysis<T, F>(pretty: bool, f: F) -> ExitCode
where
    T: serde::Serialize,
    F: FnOnce() -> Result<T, crate::image_analysis::AnalysisError>,
{
    let started = Instant::now();
    let outcome = f();
    let exec_ms = started.elapsed().as_millis() as u64;
    match outcome {
        Ok(result) => {
            let payload = serde_json::json!({
                "ok": true,
                "result": result,
                "exec_ms": exec_ms,
            });
            let formatted = if pretty {
                serde_json::to_string_pretty(&payload)
            } else {
                serde_json::to_string(&payload)
            };
            println!("{}", formatted.unwrap_or_else(|e| format!(r#"{{"ok":false,"error":"{e}"}}"#)));
            ExitCode::SUCCESS
        }
        Err(e) => {
            let payload = serde_json::json!({
                "ok": false,
                "error": e.to_string(),
                "exec_ms": exec_ms,
            });
            let formatted = if pretty {
                serde_json::to_string_pretty(&payload)
            } else {
                serde_json::to_string(&payload)
            };
            println!("{}", formatted.unwrap_or_else(|e| format!(r#"{{"ok":false,"error":"{e}"}}"#)));
            ExitCode::from(2)
        }
    }
}

/// Map a `"A"` / `"B"` / `"tie"` string into `PairVerdict`. Unknown
/// strings fall back to `Tie` so the bracket runner stays moving â the
/// upstream pairwise adapter already rejects truly unknown verdicts.
pub fn verdict_from_str(s: &str) -> crate::variants::PairVerdict {
    match s.trim().to_ascii_uppercase().as_str() {
        "A" => crate::variants::PairVerdict::A,
        "B" => crate::variants::PairVerdict::B,
        _ => crate::variants::PairVerdict::Tie,
    }
}

/// Fetch bytes from an `https://` URL, or read them from a local
/// `data:` URL or file path. Used by the face-refine handler to pull
/// the original image so it can run the in-memory crop / paste-back.
pub fn download_or_read(url: &str) -> Result<Vec<u8>, String> {
    if let Some(rest) = url.strip_prefix("data:") {
        let comma = rest.find(',').ok_or_else(|| "malformed data: URL".to_string())?;
        let payload = &rest[comma + 1..];
        return crate::backends::util::base64_decode(payload)
            .map_err(|e| format!("decode data: URL: {e}"));
    }
    if url.starts_with("http://") || url.starts_with("https://") {
        let resp = ureq::get(url).call().map_err(|e| e.to_string())?;
        let mut buf = Vec::with_capacity(64 * 1024);
        std::io::Read::read_to_end(&mut resp.into_reader(), &mut buf)
            .map_err(|e| format!("read body: {e}"))?;
        return Ok(buf);
    }
    std::fs::read(url).map_err(|e| format!("read {url}: {e}"))
}

/// (auto-generated placeholder)
pub fn pick_model_auto(input: &str) -> Option<UpscaleModel> {
    let lower = input.split('?').next().unwrap_or(input).to_lowercase();
    if lower.ends_with(".png")
        || lower.ends_with(".jpg")
        || lower.ends_with(".jpeg")
        || lower.ends_with(".webp")
    {
        Some(UpscaleModel::Supir)
    } else {
        None
    }
}

/// (auto-generated placeholder)
pub fn resolve_model(model: &str, input: &str) -> Result<UpscaleModel, String> {
    match model {
        "supir" => Ok(UpscaleModel::Supir),
        "auto" => pick_model_auto(input).ok_or_else(|| {
            format!("auto mode could not pick a model from input '{input}' (unknown extension)")
        }),
        other => Err(format!(
            "unknown --model '{other}' — want supir | auto"
        )),
    }
}

/// (auto-generated placeholder)
pub fn parse_target(spec: &str) -> Result<UpscaleTarget, String> {
    let s = spec.trim().to_lowercase();
    if let Some(num) = s.strip_suffix('x') {
        return num
            .parse::<f32>()
            .map(UpscaleTarget::Scale)
            .map_err(|_| format!("bad scale target '{spec}'"));
    }
    match s.as_str() {
        "1080p" | "fhd" => Ok(UpscaleTarget::Resolution(1920, 1080)),
        "4k" | "uhd" => Ok(UpscaleTarget::Resolution(3840, 2160)),
        "720p" => Ok(UpscaleTarget::Resolution(1280, 720)),
        _ => {
            if let Some((w, h)) = s.split_once('x') {
                if let (Ok(w), Ok(h)) = (w.parse::<u32>(), h.parse::<u32>()) {
                    return Ok(UpscaleTarget::Resolution(w, h));
                }
            }
            Err(format!("unrecognized --target '{spec}' (want 2x|4x|1080p|4k|WxH)"))
        }
    }
}

/// (auto-generated placeholder)
pub fn resolve_local_path(arg: &str) -> Result<PathBuf, String> {
    if arg.is_empty() {
        return Err("argument is empty".into());
    }
    if arg.starts_with("http://") || arg.starts_with("https://") || arg.starts_with("data:") {
        return Err(format!(
            "URL inputs not supported for the concat step â pass a local file path, got {arg}"
        ));
    }
    let p = PathBuf::from(arg);
    if !p.exists() {
        return Err(format!("file does not exist: {}", p.display()));
    }
    Ok(p)
}

/// Load a caller-supplied signing key pair, or fall back to the bundled test
/// cert when neither is supplied. Returns `Ok(None)` for the test-cert path.
pub fn load_signing_key(
    cert: Option<&std::path::Path>,
    key: Option<&std::path::Path>,
) -> Result<Option<crate::c2pa_credentials::SigningKey>, ExitCode> {
    match (cert, key) {
        (Some(c), Some(k)) => {
            let cert_pem = std::fs::read(c).map_err(|e| {
                eprintln!("c2pa: read {}: {e}", c.display());
                ExitCode::from(2)
            })?;
            let key_pem = std::fs::read(k).map_err(|e| {
                eprintln!("c2pa: read {}: {e}", k.display());
                ExitCode::from(2)
            })?;
            Ok(Some(crate::c2pa_credentials::SigningKey {
                cert_pem,
                key_pem,
                alg: c2pa::SigningAlg::Es256,
            }))
        }
        _ => Ok(None),
    }
}

/// (auto-generated placeholder)
pub fn format_winner_reason<R>(
    policy: crate::variants::SelectPolicy,
    winner: Option<u32>,
    records: &[crate::variants::VariantRecord<R>],
) -> String {
    use crate::variants::SelectPolicy;
    if records.iter().all(|r| !r.is_success()) {
        return "no variant succeeded".into();
    }
    match (policy, winner) {
        (SelectPolicy::User, _) => "user selection â caller picks from manifest".into(),
        (SelectPolicy::First, Some(i)) => format!("first-success â variant {i}"),
        (SelectPolicy::Cheapest, Some(i)) => format!("cheapest-elapsed â variant {i}"),
        (SelectPolicy::MaxVlm, Some(i)) => {
            let r = &records[i as usize];
            let p = r.vlm_pass_count.unwrap_or(0);
            let t = r.vlm_total.unwrap_or(0);
            if t == 0 {
                format!("max-vlm (no scores; fallback first-success) â variant {i}")
            } else {
                format!("max-vlm pass={p}/{t} â variant {i}")
            }
        }
        _ => String::new(),
    }
}

/// Map a `"A"` / `"B"` / `"tie"` string into `PairVerdict`. Unknown
pub fn format_pairwise_winner_reason<R>(
    winner: Option<u32>,
    bracket: &[crate::variants::BracketRound],
    records: &[crate::variants::VariantRecord<R>],
) -> String {
    if records.iter().all(|r| !r.is_success()) {
        return "no variant succeeded".into();
    }
    let Some(idx) = winner else {
        return "pairwise tournament produced no champion".into();
    };
    let rounds = bracket.len();
    let ties = bracket.iter().filter(|r| r.seed_tiebreak).count();
    if ties == 0 {
        format!("pairwise-tournament ({rounds} pairs) â variant {idx}")
    } else {
        format!(
            "pairwise-tournament ({rounds} pairs, {ties} seed-tiebreak) â variant {idx}"
        )
    }
}

#[allow(missing_docs)]
pub enum UpscaleModel {
    Supir,
}

#[allow(missing_docs)]
pub enum UpscaleTarget {
    Scale(f32),
    Resolution(u32, u32),
}

#[allow(missing_docs)]
/// Argument bundle for `run_shot_insert_into_scene`. Keeps the handler
/// signature manageable — twelve knobs all surface through the CLI.
pub struct InsertIntoSceneArgs {
    pub product: String,
    pub scene: String,
    pub threshold: f32,
    pub strict_identity: bool,
    pub seed: Option<u64>,
    pub backend: String,
    pub identity_backend: String,
    pub dry_run: bool,
    pub max_cost: f32,
    pub cache: PathBuf,
    pub out: Option<PathBuf>,
    pub pretty: bool,
}

#[allow(missing_docs)]
/// Argument bundle for the ShotOp::Still variant orchestrator.
pub struct ShotStillVariantArgs {
    pub req: crate::backends::image::Txt2ImgRequest,
    pub backend: String,
    pub variants: u32,
    pub select_raw: String,
    pub criteria: Vec<String>,
    pub max_variants_cost: Option<f32>,
    pub mode: crate::backends::RunMode,
    pub cache: PathBuf,
    pub out: Option<PathBuf>,
    pub pretty: bool,
    pub dry_run: bool,
}

#[allow(missing_docs)]
pub struct QueryArgs {
    pub comp: PathBuf,
    pub at: String,
    pub bbox: Option<String>,
    pub visible: Option<String>,
    pub safe_sel: Option<String>,
    pub inset: f32,
    pub xform_sel: Option<String>,
    pub no_overlap: bool,
    pub color_at: Option<String>,
    pub color_in: Option<String>,
    pub max_de: f32,
    pub contrast: Option<String>,
    pub contrast_threshold: f32,
    pub banding: Option<String>,
    pub on_beat: Option<PathBuf>,
    pub tolerance_ms: u32,
    pub text_visible: Option<String>,
    pub text_in: Option<String>,
    pub text_tolerance: u32,
    pub snapshot: bool,
}

#[allow(missing_docs)]
#[derive(Serialize)]
pub struct QueryOutput {
    pub comp: String,
    pub t_secs: f32,
    pub frame_index: u32,
    pub queries: Vec<QueryEntry>,
    pub summary: QuerySummary,
}

#[allow(missing_docs)]
#[derive(Serialize)]
pub struct QuerySummary {
    pub passed: usize,
    pub failed: usize,
    pub total_ms: u128,
}

#[allow(missing_docs)]
#[derive(Serialize)]
#[serde(tag = "q", rename_all = "snake_case")]
pub enum QueryEntry {
    Bbox {
        ok: bool,
        selector: String,
        bbox: Option<crate::query::Rect>,
        exec_ms: u128,
    },
    Visible {
        ok: bool,
        selector: String,
        verdict: VisibilityVerdict,
        exec_ms: u128,
    },
    InSafeArea {
        ok: bool,
        selector: String,
        bbox: Option<crate::query::Rect>,
        safe_area: crate::query::Rect,
        inset: f32,
        exec_ms: u128,
    },
    TransformInherits {
        ok: bool,
        selector: String,
        affected_ancestors: Vec<usize>,
        exec_ms: u128,
    },
    NoOverlap {
        ok: bool,
        overlaps: Vec<OverlapPair>,
        exec_ms: u128,
    },
    ColorAt {
        ok: bool,
        x: i32,
        y: i32,
        color: Option<[u8; 4]>,
        hex: Option<String>,
        exec_ms: u128,
    },
    ColorIn {
        ok: bool,
        selector: String,
        mean: Option<[u8; 4]>,
        delta_e: Option<f32>,
        target: String,
        max_de: f32,
        exec_ms: u128,
    },
    Contrast {
        ok: bool,
        selector: String,
        ratio: Option<f32>,
        threshold: f32,
        exec_ms: u128,
    },
    Banding {
        ok: bool,
        unique_colors: usize,
        sampled_rows: u32,
        diversity: f32,
        exec_ms: u128,
    },
    OnBeat {
        ok: bool,
        aligned: usize,
        total: usize,
        worst_delta_ms: u32,
        failed: Vec<String>,
        tolerance_ms: u32,
        onset_count: usize,
        events: Vec<ScoredEvent>,
        exec_ms: u128,
    },
    TextVisible {
        ok: bool,
        expected: String,
        detected: Option<String>,
        edit_distance: Option<u32>,
        tolerance: u32,
        selector: Option<String>,
        error: Option<String>,
        exec_ms: u128,
    },
    Snapshot {
        node_count: usize,
        exec_ms: u128,
    },
}

/// (auto-generated placeholder)
pub fn short_digest(s: &str) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut h = DefaultHasher::new();
    s.hash(&mut h);
    format!("{:016x}", h.finish())
}

/// Parse `Finding`s out of a verify-shot report. Accepts both the
/// top-level `VisionVerifyResult` JSON and the wrapper shape emitted by
/// `wavelet image verify-shot` (which nests the result under a `result`
/// key alongside `mode`, `provider`, etc).
pub fn parse_verify_findings(raw: &str) -> Result<Vec<crate::backends::image::Finding>, String> {
    let v: serde_json::Value = serde_json::from_str(raw).map_err(|e| e.to_string())?;
    let candidate = v
        .get("findings")
        .or_else(|| v.get("result").and_then(|r| r.get("findings")))
        .ok_or_else(|| "no `findings` array in report".to_string())?;
    serde_json::from_value(candidate.clone()).map_err(|e| e.to_string())
}

/// Parse a `"X,Y,W,H"` bbox string into a `[u32; 4]`.
pub fn parse_bbox(s: &str) -> Result<[u32; 4], String> {
    let parts: Vec<&str> = s.split(',').map(str::trim).collect();
    if parts.len() != 4 {
        return Err(format!("expected X,Y,W,H (4 fields), got {}", parts.len()));
    }
    let mut out = [0u32; 4];
    for (i, p) in parts.iter().enumerate() {
        out[i] = p
            .parse()
            .map_err(|e| format!("field {i} '{p}' is not a u32: {e}"))?;
    }
    Ok(out)
}

/// Resolve source image dimensions for a region hint. Reads the image
/// file when `input` is a local path; otherwise relies on
/// `--image-w` / `--image-h`.
pub fn resolve_image_dims(
    input: &str,
    image_w: Option<u32>,
    image_h: Option<u32>,
) -> Result<(u32, u32), String> {
    if let (Some(w), Some(h)) = (image_w, image_h) {
        return Ok((w, h));
    }
    if input.starts_with("http://") || input.starts_with("https://") || input.starts_with("data:") {
        return Err("remote/data URL input â pass --image-w and --image-h".into());
    }
    let dyn_img = image::open(input).map_err(|e| format!("read {input}: {e}"))?;
    Ok((dyn_img.width(), dyn_img.height()))
}

/// (auto-generated placeholder)
pub fn serialize_search(
    result: Result<
        crate::backends::BackendCallOutcome<crate::backends::stock::StockSearchResult>,
        crate::backends::BackendError,
    >,
) -> Result<serde_json::Value, crate::backends::BackendError> {
    result.map(|outcome| {
        serde_json::json!({
            "mode": outcome.mode,
            "provider": outcome.provider,
            "request_hash": outcome.request_hash,
            "cached": outcome.cached,
            "cost_estimate_usd": outcome.cost_estimate_usd,
            "result": outcome.response,
        })
    })
}

/// (auto-generated placeholder)
pub fn parse_rect(s: &str) -> Option<Rect> {
    let parts: Vec<&str> = s.split(',').collect();
    if parts.len() != 4 {
        return None;
    }
    Some(Rect {
        x: parts[0].trim().parse().ok()?,
        y: parts[1].trim().parse().ok()?,
        w: parts[2].trim().parse().ok()?,
        h: parts[3].trim().parse().ok()?,
    })
}

/// Parse a time string into seconds. Accepts `0.5s`, `frame:90`, `MM:SS`,
/// or a bare float (seconds). Empty string â caller picks a default.
pub fn parse_time(s: &str, fps: u32) -> Result<f32, String> {
    let s = s.trim();
    if s.is_empty() {
        return Err("empty time string".into());
    }
    if let Some(rest) = s.strip_prefix("frame:") {
        let f: u32 = rest
            .parse()
            .map_err(|e| format!("invalid frame index '{rest}': {e}"))?;
        return Ok(f as f32 / fps as f32);
    }
    if s.ends_with('s') {
        return s[..s.len() - 1]
            .parse::<f32>()
            .map_err(|e| format!("invalid seconds '{s}': {e}"));
    }
    if let Some((mm, ss)) = s.split_once(':') {
        let m: f32 = mm.parse().map_err(|e| format!("invalid minutes '{mm}': {e}"))?;
        let s2: f32 = ss.parse().map_err(|e| format!("invalid seconds '{ss}': {e}"))?;
        return Ok(m * 60.0 + s2);
    }
    s.parse::<f32>().map_err(|e| format!("invalid time '{s}': {e}"))
}

/// (auto-generated placeholder)
pub fn parse_xy(s: &str) -> Option<(i32, i32)> {
    let (a, b) = s.split_once(',')?;
    Some((a.trim().parse().ok()?, b.trim().parse().ok()?))
}

/// (auto-generated placeholder)
pub fn parse_resolution(s: &str) -> Option<[u32; 2]> {
    let (w, h) = s.split_once('x')?;
    Some([w.trim().parse().ok()?, h.trim().parse().ok()?])
}

/// Instruction prompt for shot insert-into-scene.
pub const INSERT_INTO_SCENE_INSTRUCTION: &str = "merge these two reference images: take the product from the LEFT half and place it naturally into the scene from the RIGHT half, matching the right-half's lighting direction, color temperature, and shadow geometry; output only the merged scene without the side-by-side layout, leave the right-half scene composition unchanged";

/// Max retry attempts for shot insert-into-scene.
pub const INSERT_INTO_SCENE_MAX_RETRIES: u32 = 3;
