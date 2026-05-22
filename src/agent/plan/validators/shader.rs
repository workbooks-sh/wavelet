//! `query_shader` validator (wb-mxrk.8) — runs one of the five starter
//! assertion shaders against a single frame and grades the outcome.
//!
//! Ordering discipline (per the methodology doc §7.1): `query_shader`
//! should appear after `query_scene_graph` and before `rubric_passes`
//! when authoring a task. Cheap structural → cheap pixel/composition →
//! expensive vision. The dispatcher does not enforce order — it runs
//! validators in declared order and short-circuits on the first failure;
//! the author chooses the sequence.
//!
//! v1 frame input: `frame: <png-path>` (resolved against `ctx.workdir`).
//! The full `comp + on_frame` URI path (`ctx:hero.mp4@t=0.5`) lands
//! when render integration matures; `on_frame` is accepted as a
//! log-only no-op for now. `correlate_with` is also accepted and
//! ignored — placeholder for the v2 cross-correlation hook.
//!
//! For `motion_magnitude`, the validator additionally requires
//! `prev_frame: <png-path>`. For `golden_rmse`, `golden: <png-path>`.

use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::{anyhow, Result};
use serde_json::{json, Value};

use super::super::validator::{Validator, ValidatorCtx, ValidatorOutcome};
use crate::shader::assert::{
    color_band_mean::{assert_color_band_mean, HslTarget},
    contrast_in_region::{assert_contrast, Region},
    golden_rmse::assert_golden_rmse,
    motion_magnitude::assert_motion,
    sobel_edge_density::assert_sobel_edge_density,
    AssertionOutcome, FrameSource,
};

/// All shader names this validator + the CLI + the agent tool resolve.
pub const KNOWN_SHADERS: &[&str] = &[
    "contrast_in_region",
    "motion_magnitude",
    "golden_rmse",
    "sobel_edge_density",
    "color_band_mean",
];

/// Decoded shape returned by `run_named_shader`. Mirrored byte-for-byte
/// across the validator detail, the CLI stdout JSON, and the agent
/// tool response.
#[derive(Debug, Clone)]
pub struct ShaderRunResult {
    pub shader: String,
    pub pass: bool,
    pub score: f32,
    pub metrics: Vec<f32>,
    pub reason_code: i32,
    pub reason: String,
    pub elapsed_us: u64,
}

impl ShaderRunResult {
    /// JSON envelope shared by every consumer.
    pub fn to_json(&self) -> Value {
        json!({
            "shader": self.shader,
            "pass": self.pass,
            "score": self.score,
            "metrics": self.metrics,
            "reason_code": self.reason_code,
            "reason": self.reason,
            "elapsed_us": self.elapsed_us,
        })
    }
}

/// Resolve `shader` (a name from `KNOWN_SHADERS`) against `frame_path`
/// + `params`, dispatch, and return the decoded run result.
///
/// The `score` field is shader-specific:
///   - contrast_in_region: contrast ratio (evidence[0]) — higher is better
///   - motion_magnitude:  mean motion magnitude (evidence[8]) — higher is better
///   - golden_rmse:       global RMSE on [0,1] (evidence[0]) — lower is better
///   - sobel_edge_density: edge density fraction — higher is better
///   - color_band_mean:   max |delta| across HSL — lower is better (0 = exact match)
pub fn run_named_shader(
    shader: &str,
    frame_path: &Path,
    params: &Value,
) -> Result<ShaderRunResult> {
    let start = Instant::now();
    let frame = FrameSource::PngPath(frame_path.to_path_buf());
    let outcome = match shader {
        "contrast_in_region" => {
            let region = read_region(params)?;
            let min_contrast = read_f32(params, "min_contrast", 4.5);
            assert_contrast(frame, region, min_contrast)?
        }
        "motion_magnitude" => {
            let prev_rel = params
                .get("prev_frame")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow!("motion_magnitude requires `prev_frame: <png-path>`"))?;
            let prev = FrameSource::PngPath(PathBuf::from(prev_rel));
            let min_mean = read_f32(params, "min_mean", 0.01);
            assert_motion(prev, frame, min_mean)?
        }
        "golden_rmse" => {
            let golden_rel = params
                .get("golden")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow!("golden_rmse requires `golden: <png-path>`"))?;
            let golden = FrameSource::PngPath(PathBuf::from(golden_rel));
            let max_diff = read_u32(params, "max_diff", 8);
            let max_pixels = read_u32(params, "max_pixels", 0);
            assert_golden_rmse(frame, golden, max_diff, max_pixels)?
        }
        "sobel_edge_density" => {
            let region = read_region(params)?;
            let threshold = read_f32(params, "threshold", 0.5);
            let min_density = read_f32(params, "min_density", 0.05);
            assert_sobel_edge_density(frame, region, threshold, min_density)?
        }
        "color_band_mean" => {
            let region = read_region(params)?;
            let target = HslTarget {
                h: read_f32(params, "h", 0.0),
                s: read_f32(params, "s", 0.0),
                l: read_f32(params, "l", 0.0),
                tolerance: read_f32(params, "tolerance", 0.1),
            };
            assert_color_band_mean(frame, region, target)?
        }
        other => {
            return Err(anyhow!(
                "unknown_shader: '{other}' (available: {})",
                KNOWN_SHADERS.join(", ")
            ));
        }
    };
    let elapsed_us = start.elapsed().as_micros() as u64;
    Ok(decode_outcome(shader, outcome, elapsed_us))
}

fn decode_outcome(shader: &str, outcome: AssertionOutcome, elapsed_us: u64) -> ShaderRunResult {
    let score = outcome.evidence.first().copied().unwrap_or(0.0);
    ShaderRunResult {
        shader: shader.to_string(),
        pass: outcome.passed,
        score,
        metrics: outcome.evidence,
        reason_code: outcome.reason_code,
        reason: outcome.reason,
        elapsed_us,
    }
}

fn region_from_floats(floats: Vec<f32>) -> Result<Region> {
    if floats.len() != 4 {
        return Err(anyhow!("`region` must have 4 elements (x, y, w, h)"));
    }
    Ok(Region {
        x: floats[0],
        y: floats[1],
        w: floats[2],
        h: floats[3],
    })
}

fn read_region(params: &Value) -> Result<Region> {
    let arr = params
        .get("region")
        .and_then(|v| v.as_array())
        .ok_or_else(|| anyhow!("missing `region: [x, y, w, h]`"))?;
    region_from_floats(arr.iter().map(|v| v.as_f64().unwrap_or(0.0) as f32).collect())
}

fn read_region_yaml(params: &serde_yaml::Value) -> Result<Region> {
    let seq = params
        .get("region")
        .and_then(|v| v.as_sequence())
        .ok_or_else(|| anyhow!("missing `region: [x, y, w, h]`"))?;
    region_from_floats(seq.iter().map(|v| v.as_f64().unwrap_or(0.0) as f32).collect())
}

fn read_f32(params: &Value, key: &str, default: f32) -> f32 {
    params
        .get(key)
        .and_then(|v| v.as_f64())
        .map(|v| v as f32)
        .unwrap_or(default)
}

fn read_u32(params: &Value, key: &str, default: u32) -> u32 {
    params
        .get(key)
        .and_then(|v| v.as_u64())
        .map(|v| v as u32)
        .unwrap_or(default)
}

fn yaml_to_json(y: &serde_yaml::Value) -> Value {
    serde_json::to_value(y).unwrap_or(Value::Null)
}

fn fail(detail: Value, start: Instant) -> ValidatorOutcome {
    ValidatorOutcome {
        ok: false,
        detail,
        cost_usd: 0.0,
        wall_ms: start.elapsed().as_millis(),
    }
}

/// `query_shader` — params:
/// `{shader, frame, params: {…}, on_frame?, correlate_with?, fail_action?}`.
///
/// `on_frame` and `correlate_with` are accepted but ignored in v1; the
/// validator runs against a single PNG. `fail_action` chooses the
/// `severity` field in the failure detail (`"warn"` default vs
/// `"abort"`). The outcome's `ok` field is `false` either way on fail
/// — the dispatcher's caller decides what to do with that signal.
pub struct QueryShader;

impl Validator for QueryShader {
    fn kind(&self) -> &'static str {
        "query_shader"
    }

    fn check(&self, params: &serde_yaml::Value, ctx: &ValidatorCtx) -> ValidatorOutcome {
        let start = Instant::now();
        let Some(shader) = params.get("shader").and_then(|v| v.as_str()) else {
            return fail(
                json!({ "error": "missing_param", "param": "shader" }),
                start,
            );
        };
        if !KNOWN_SHADERS.contains(&shader) {
            return fail(
                json!({
                    "error": "unknown_shader",
                    "shader": shader,
                    "available": KNOWN_SHADERS,
                }),
                start,
            );
        }
        let Some(frame_rel) = params.get("frame").and_then(|v| v.as_str()) else {
            return fail(
                json!({ "error": "missing_param", "param": "frame" }),
                start,
            );
        };
        let frame_path = ctx.workdir.join(frame_rel);

        let fail_action = params
            .get("fail_action")
            .and_then(|v| v.as_str())
            .unwrap_or("warn");
        let severity = if fail_action == "abort_render" {
            "abort"
        } else {
            "warn"
        };

        // Shader-specific params live under the optional `params:` map.
        // We resolve relative paths inside it against the workdir, then
        // serialize to JSON so `run_named_shader` can read it generically.
        let sub_params_yaml = params
            .get("params")
            .cloned()
            .unwrap_or(serde_yaml::Value::Null);
        let mut sub_params_json = yaml_to_json(&sub_params_yaml);

        // Region sanity (when applicable) caught early so failure detail
        // surfaces the validator's own shape error rather than the
        // shader's generic anyhow.
        if matches!(shader, "contrast_in_region" | "sobel_edge_density" | "color_band_mean") {
            if let Err(e) = read_region_yaml(&sub_params_yaml) {
                return fail(
                    json!({
                        "error": "bad_params",
                        "shader": shader,
                        "reason": e.to_string(),
                    }),
                    start,
                );
            }
        }

        // Resolve sibling-frame paths against the workdir, same as `frame`.
        for key in ["prev_frame", "golden"] {
            if let Some(Value::String(rel)) = sub_params_json.get(key).cloned() {
                let abs = ctx.workdir.join(&rel);
                sub_params_json[key] = Value::String(abs.to_string_lossy().into_owned());
            }
        }

        match run_named_shader(shader, &frame_path, &sub_params_json) {
            Ok(res) => {
                let ok = res.pass;
                let detail = if ok {
                    json!({
                        "shader": res.shader,
                        "params": sub_params_json,
                        "pass": true,
                        "score": res.score,
                        "metrics": res.metrics,
                        "reason_code": res.reason_code,
                        "reason": res.reason,
                        "elapsed_us": res.elapsed_us,
                    })
                } else {
                    json!({
                        "shader": res.shader,
                        "params": sub_params_json,
                        "pass": false,
                        "score": res.score,
                        "metrics": res.metrics,
                        "reason_code": res.reason_code,
                        "reason": res.reason,
                        "severity": severity,
                        "elapsed_us": res.elapsed_us,
                    })
                };
                ValidatorOutcome {
                    ok,
                    detail,
                    cost_usd: 0.0,
                    wall_ms: start.elapsed().as_millis(),
                }
            }
            Err(e) => fail(
                json!({
                    "error": "dispatch_failed",
                    "shader": shader,
                    "frame": frame_rel,
                    "reason": e.to_string(),
                }),
                start,
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::{Path, PathBuf};

    fn write_solid_png(path: &Path, w: u32, h: u32, rgba: [u8; 4]) {
        let mut pixels = Vec::with_capacity((w * h * 4) as usize);
        for _ in 0..(w * h) {
            pixels.extend_from_slice(&rgba);
        }
        write_rgba_png(path, w, h, &pixels);
    }

    fn write_split_png(path: &Path, w: u32, h: u32, left: [u8; 4], right: [u8; 4]) {
        let mut pixels = Vec::with_capacity((w * h * 4) as usize);
        for _y in 0..h {
            for x in 0..w {
                let px = if x < w / 2 { left } else { right };
                pixels.extend_from_slice(&px);
            }
        }
        write_rgba_png(path, w, h, &pixels);
    }

    fn write_rgba_png(path: &Path, w: u32, h: u32, pixels: &[u8]) {
        let file = fs::File::create(path).unwrap();
        let mut enc = png::Encoder::new(file, w, h);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        let mut writer = enc.write_header().unwrap();
        writer.write_image_data(pixels).unwrap();
        writer.finish().unwrap();
    }

    fn ctx<'a>(workdir: &'a Path, gamut_bin: &'a Path) -> ValidatorCtx<'a> {
        ValidatorCtx {
            workdir,
            gamut_bin,
            session_cost_usd: 0.0,
        }
    }

    fn yaml(s: &str) -> serde_yaml::Value {
        serde_yaml::from_str(s).unwrap()
    }

    #[test]
    fn contrast_in_region_pass_low_threshold() {
        let dir = tempfile::tempdir().unwrap();
        let png = dir.path().join("frame.png");
        // Black-vs-white split frame easily meets a min_contrast of 1.0.
        write_split_png(&png, 16, 16, [0, 0, 0, 255], [255, 255, 255, 255]);

        let bin = PathBuf::from("wavelet");
        let v = QueryShader;
        let out = v.check(
            &yaml(
                "{ shader: contrast_in_region, frame: frame.png, \
                   params: { region: [0.0, 0.0, 1.0, 1.0], min_contrast: 1.0 } }",
            ),
            &ctx(dir.path(), &bin),
        );
        assert!(out.ok, "detail={:?}", out.detail);
        assert_eq!(out.detail["pass"], serde_json::Value::Bool(true));
        assert_eq!(
            out.detail["shader"],
            serde_json::Value::String("contrast_in_region".into())
        );
        assert!(out.detail["metrics"].is_array());
        assert!(out.detail["score"].as_f64().unwrap() > 1.0);
    }

    #[test]
    fn contrast_in_region_fails_near_gray() {
        let dir = tempfile::tempdir().unwrap();
        let png = dir.path().join("frame.png");
        // Two near-equal grays — contrast ratio ~1.1, well below AA 4.5.
        write_split_png(&png, 16, 16, [120, 120, 120, 255], [132, 132, 132, 255]);

        let bin = PathBuf::from("wavelet");
        let v = QueryShader;
        let out = v.check(
            &yaml(
                "{ shader: contrast_in_region, frame: frame.png, \
                   params: { region: [0.0, 0.0, 1.0, 1.0], min_contrast: 4.5 } }",
            ),
            &ctx(dir.path(), &bin),
        );
        assert!(!out.ok, "detail={:?}", out.detail);
        assert_eq!(out.detail["pass"], serde_json::Value::Bool(false));
        assert_eq!(
            out.detail["shader"],
            serde_json::Value::String("contrast_in_region".into())
        );
        // Failure detail must carry score + reason + severity.
        assert!(out.detail.get("score").is_some());
        assert!(out.detail.get("reason").is_some());
        assert_eq!(
            out.detail["severity"],
            serde_json::Value::String("warn".into())
        );
        // params surface back so the model can act on them.
        assert!(out.detail["params"].get("min_contrast").is_some());
    }

    #[test]
    fn fail_action_abort_render_marks_severity() {
        let dir = tempfile::tempdir().unwrap();
        let png = dir.path().join("frame.png");
        write_split_png(&png, 16, 16, [120, 120, 120, 255], [132, 132, 132, 255]);

        let bin = PathBuf::from("wavelet");
        let v = QueryShader;
        let out = v.check(
            &yaml(
                "{ shader: contrast_in_region, frame: frame.png, \
                   fail_action: abort_render, \
                   params: { region: [0.0, 0.0, 1.0, 1.0], min_contrast: 4.5 } }",
            ),
            &ctx(dir.path(), &bin),
        );
        assert!(!out.ok);
        assert_eq!(
            out.detail["severity"],
            serde_json::Value::String("abort".into())
        );
    }

    #[test]
    fn unknown_shader_returns_structured_error() {
        let dir = tempfile::tempdir().unwrap();
        let bin = PathBuf::from("wavelet");
        let v = QueryShader;
        let out = v.check(
            &yaml("{ shader: not_a_real_shader, frame: x.png }"),
            &ctx(dir.path(), &bin),
        );
        assert!(!out.ok);
        assert_eq!(
            out.detail["error"],
            serde_json::Value::String("unknown_shader".into())
        );
        assert_eq!(
            out.detail["shader"],
            serde_json::Value::String("not_a_real_shader".into())
        );
        assert!(out.detail["available"].is_array());
    }

    #[test]
    fn missing_shader_param_is_structured() {
        let dir = tempfile::tempdir().unwrap();
        let bin = PathBuf::from("wavelet");
        let v = QueryShader;
        let out = v.check(&yaml("{ frame: x.png }"), &ctx(dir.path(), &bin));
        assert!(!out.ok);
        assert_eq!(
            out.detail["error"],
            serde_json::Value::String("missing_param".into())
        );
        assert_eq!(
            out.detail["param"],
            serde_json::Value::String("shader".into())
        );
    }

    #[test]
    fn missing_frame_param_is_structured() {
        let dir = tempfile::tempdir().unwrap();
        let bin = PathBuf::from("wavelet");
        let v = QueryShader;
        let out = v.check(
            &yaml("{ shader: contrast_in_region }"),
            &ctx(dir.path(), &bin),
        );
        assert!(!out.ok);
        assert_eq!(
            out.detail["error"],
            serde_json::Value::String("missing_param".into())
        );
        assert_eq!(
            out.detail["param"],
            serde_json::Value::String("frame".into())
        );
    }

    #[cfg(unix)]
    #[test]
    fn cli_query_shader_pass_and_fail_exit_codes() {
        // Hermetic spawn of the built wavelet binary. The CLI is wired
        // through the same `run_named_shader` helper as the validator
        // and the agent tool; this test just confirms the
        // stdout-JSON + exit-code contract is preserved.
        let dir = tempfile::tempdir().unwrap();
        let png = dir.path().join("frame.png");
        write_split_png(&png, 16, 16, [0, 0, 0, 255], [255, 255, 255, 255]);

        // CARGO_BIN_EXE_wavelet is set by cargo for integration tests in
        // tests/, but unit tests inside the crate don't get it. Walk
        // up from CARGO_MANIFEST_DIR/target/{profile}/wavelet instead.
        let manifest_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let profile = if cfg!(debug_assertions) { "debug" } else { "release" };
        let bin = manifest_dir.join("target").join(profile).join("wavelet");
        if !bin.exists() {
            eprintln!("skip: wavelet binary not built at {}", bin.display());
            return;
        }

        let ok = std::process::Command::new(&bin)
            .args([
                "query-shader",
                "--shader",
                "contrast_in_region",
                "--frame",
            ])
            .arg(&png)
            .args([
                "--params",
                r#"{"region":[0.0,0.0,1.0,1.0],"min_contrast":1.0}"#,
            ])
            .output()
            .expect("spawn wavelet");
        assert!(ok.status.success(), "stderr={}", String::from_utf8_lossy(&ok.stderr));
        let stdout = String::from_utf8_lossy(&ok.stdout);
        let v: serde_json::Value = serde_json::from_str(stdout.trim()).expect("json");
        assert_eq!(v["pass"], json!(true));
        assert_eq!(v["shader"], json!("contrast_in_region"));

        let bad = std::process::Command::new(&bin)
            .args([
                "query-shader",
                "--shader",
                "contrast_in_region",
                "--frame",
            ])
            .arg(&png)
            .args([
                "--params",
                r#"{"region":[0.0,0.0,1.0,1.0],"min_contrast":99.0}"#,
            ])
            .output()
            .expect("spawn wavelet");
        assert_eq!(bad.status.code(), Some(1));
        let v: serde_json::Value = serde_json::from_str(
            String::from_utf8_lossy(&bad.stdout).trim()
        ).expect("json");
        assert_eq!(v["pass"], json!(false));
    }

    #[test]
    fn run_named_shader_unknown_carries_available_list() {
        let dir = tempfile::tempdir().unwrap();
        let png = dir.path().join("frame.png");
        write_solid_png(&png, 4, 4, [255, 255, 255, 255]);
        let err = run_named_shader("nope", &png, &json!({})).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("unknown_shader"), "msg={msg}");
        assert!(msg.contains("contrast_in_region"), "msg={msg}");
    }
}
