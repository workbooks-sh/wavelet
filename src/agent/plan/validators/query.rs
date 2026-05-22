//! `query.*` validators — shell out to `wavelet query` and grade the JSON
//! response against declarative `expect` clauses.
//!
//! Failure detail always includes the actual response (or the failing
//! sub-clause) so the model can act on it without re-running the query.

use std::path::PathBuf;
use std::time::Instant;

use serde_json::{json, Value};

use super::super::validator::{Validator, ValidatorCtx, ValidatorOutcome};
use super::util::{argv_with_bin, run_json, ShellJson};

/// `query_scene_graph` — params:
/// `{comp, at, expect: {visible?: [sel], bbox_at_least?: {selector, w, h},
///   bbox_within?: {selector, max_overflow_px}}}`.
pub struct QuerySceneGraph;

impl Validator for QuerySceneGraph {
    fn kind(&self) -> &'static str { "query_scene_graph" }

    fn check(&self, params: &serde_yaml::Value, ctx: &ValidatorCtx) -> ValidatorOutcome {
        let start = Instant::now();
        let Some(comp) = params.get("comp").and_then(|v| v.as_str()) else {
            return missing("comp", start);
        };
        let at = params.get("at").and_then(|v| v.as_str()).unwrap_or("");
        let expect = params.get("expect");

        let mut argv = vec!["query".to_string(), comp.to_string()];
        if !at.is_empty() {
            argv.push("--at".into());
            argv.push(at.into());
        }
        // Aggregate the visible/bbox flags from expect into a single
        // wavelet-query invocation; the binary returns one JSON object.
        let visible_targets: Vec<String> = expect
            .and_then(|e| e.get("visible"))
            .and_then(|v| v.as_sequence())
            .map(|s| s.iter().filter_map(|x| x.as_str().map(String::from)).collect())
            .unwrap_or_default();
        for sel in &visible_targets {
            argv.push("--visible".into());
            argv.push(sel.clone());
        }
        let bbox_target_at_least = expect
            .and_then(|e| e.get("bbox_at_least"))
            .and_then(|b| b.get("selector"))
            .and_then(|s| s.as_str())
            .map(String::from);
        let bbox_target_within = expect
            .and_then(|e| e.get("bbox_within"))
            .and_then(|b| b.get("selector"))
            .and_then(|s| s.as_str())
            .map(String::from);
        for sel in bbox_target_at_least.iter().chain(bbox_target_within.iter()) {
            argv.push("--bbox".into());
            argv.push(sel.clone());
        }
        argv.push("--json".into());

        match run_json(ctx.gamut_bin, &argv, ctx.workdir) {
            ShellJson::Ok(v) => grade_scene_graph(v, &visible_targets, expect, &argv, ctx.gamut_bin, start),
            other => fail(other.into_failure_detail(&argv_with_bin(ctx.gamut_bin, &argv)), start),
        }
    }
}

fn grade_scene_graph(
    resp: Value,
    visible_targets: &[String],
    expect: Option<&serde_yaml::Value>,
    argv: &[String],
    bin: &std::path::Path,
    start: Instant,
) -> ValidatorOutcome {
    // `visible` clause — each selector must report a visible verdict.
    for sel in visible_targets {
        let verdict = resp
            .pointer(&format!("/visible/{}", json_pointer_escape(sel)))
            .or_else(|| resp.get("visible").and_then(|v| v.get(sel)));
        let is_visible = verdict
            .and_then(|v| v.get("visible").or_else(|| v.get("ok")))
            .and_then(|b| b.as_bool());
        if is_visible != Some(true) {
            return ValidatorOutcome {
                ok: false,
                detail: json!({
                    "argv": argv_with_bin(bin, argv),
                    "failed_clause": "visible",
                    "selector": sel,
                    "actual": verdict.cloned().unwrap_or(json!({ "not_found": true })),
                    "response": resp,
                }),
                cost_usd: 0.0,
                wall_ms: start.elapsed().as_millis(),
            };
        }
    }

    // `bbox_at_least` — element bbox.w >= w && bbox.h >= h.
    if let Some(rule) = expect.and_then(|e| e.get("bbox_at_least")) {
        let sel = rule.get("selector").and_then(|s| s.as_str()).unwrap_or("");
        let need_w = rule.get("w").and_then(|w| w.as_f64()).unwrap_or(0.0);
        let need_h = rule.get("h").and_then(|h| h.as_f64()).unwrap_or(0.0);
        let bbox = resp.get("bbox").and_then(|m| m.get(sel));
        let (w, h) = bbox
            .map(|b| {
                (
                    b.get("w").and_then(|x| x.as_f64()).unwrap_or(0.0),
                    b.get("h").and_then(|x| x.as_f64()).unwrap_or(0.0),
                )
            })
            .unwrap_or((0.0, 0.0));
        if bbox.is_none() || w < need_w || h < need_h {
            return ValidatorOutcome {
                ok: false,
                detail: json!({
                    "argv": argv_with_bin(bin, argv),
                    "failed_clause": "bbox_at_least",
                    "selector": sel,
                    "expected": {"w": need_w, "h": need_h},
                    "actual": bbox.cloned().unwrap_or(json!({"not_found": true})),
                }),
                cost_usd: 0.0,
                wall_ms: start.elapsed().as_millis(),
            };
        }
    }

    // `bbox_within` — element bbox must lie inside frame bounds, with
    // up to `max_overflow_px` slack on each side. We assume the
    // response includes `frame: {w, h}` and `bbox: {selector: {...}}`.
    if let Some(rule) = expect.and_then(|e| e.get("bbox_within")) {
        let sel = rule.get("selector").and_then(|s| s.as_str()).unwrap_or("");
        let slack = rule
            .get("max_overflow_px")
            .and_then(|x| x.as_f64())
            .unwrap_or(0.0);
        let frame = resp.get("frame");
        let bbox = resp.get("bbox").and_then(|m| m.get(sel));
        let frame_w = frame
            .and_then(|f| f.get("w").or_else(|| f.get("width")))
            .and_then(|w| w.as_f64());
        let frame_h = frame
            .and_then(|f| f.get("h").or_else(|| f.get("height")))
            .and_then(|h| h.as_f64());
        let ok = match (frame_w, frame_h, bbox) {
            (Some(fw), Some(fh), Some(b)) => {
                let x = b.get("x").and_then(|x| x.as_f64()).unwrap_or(0.0);
                let y = b.get("y").and_then(|y| y.as_f64()).unwrap_or(0.0);
                let w = b.get("w").and_then(|w| w.as_f64()).unwrap_or(0.0);
                let h = b.get("h").and_then(|h| h.as_f64()).unwrap_or(0.0);
                x >= -slack
                    && y >= -slack
                    && (x + w) <= (fw + slack)
                    && (y + h) <= (fh + slack)
            }
            _ => false,
        };
        if !ok {
            return ValidatorOutcome {
                ok: false,
                detail: json!({
                    "argv": argv_with_bin(bin, argv),
                    "failed_clause": "bbox_within",
                    "selector": sel,
                    "max_overflow_px": slack,
                    "frame": frame.cloned().unwrap_or(json!({"not_found": true})),
                    "actual": bbox.cloned().unwrap_or(json!({"not_found": true})),
                }),
                cost_usd: 0.0,
                wall_ms: start.elapsed().as_millis(),
            };
        }
    }

    ValidatorOutcome {
        ok: true,
        detail: json!({ "argv": argv_with_bin(bin, argv), "response": resp }),
        cost_usd: 0.0,
        wall_ms: start.elapsed().as_millis(),
    }
}

fn json_pointer_escape(s: &str) -> String {
    s.replace('~', "~0").replace('/', "~1")
}

/// `query_pixels` — params:
/// `{comp, at, expect: {color_at?: {selector, hex, tolerance},
///   contrast?: {selector, min_ratio}, banding?: {region, max_score}}}`.
pub struct QueryPixels;

impl Validator for QueryPixels {
    fn kind(&self) -> &'static str { "query_pixels" }

    fn check(&self, params: &serde_yaml::Value, ctx: &ValidatorCtx) -> ValidatorOutcome {
        let start = Instant::now();
        let Some(comp) = params.get("comp").and_then(|v| v.as_str()) else {
            return missing("comp", start);
        };
        let at = params.get("at").and_then(|v| v.as_str()).unwrap_or("");
        let expect = params.get("expect");

        let mut argv = vec!["query".into(), comp.to_string()];
        if !at.is_empty() {
            argv.push("--at".into());
            argv.push(at.into());
        }

        let color_at = expect.and_then(|e| e.get("color_at"));
        let contrast = expect.and_then(|e| e.get("contrast"));
        let banding = expect.and_then(|e| e.get("banding"));

        // `color_at` uses --color-in (selector=hex) when selector form is given,
        // or --color-at (x,y) when coordinate form is given.
        if let Some(c) = color_at {
            if let (Some(sel), Some(hex)) = (
                c.get("selector").and_then(|x| x.as_str()),
                c.get("hex").and_then(|x| x.as_str()),
            ) {
                argv.push("--color-in".into());
                argv.push(format!("{sel}={hex}"));
                if let Some(tol) = c.get("tolerance").and_then(|t| t.as_f64()) {
                    argv.push("--max-de".into());
                    argv.push(format!("{tol}"));
                }
            } else if let Some(xy) = c.get("xy").and_then(|x| x.as_str()) {
                argv.push("--color-at".into());
                argv.push(xy.into());
            }
        }
        if let Some(c) = contrast {
            if let Some(sel) = c.get("selector").and_then(|s| s.as_str()) {
                argv.push("--contrast".into());
                argv.push(sel.into());
                if let Some(t) = c.get("min_ratio").and_then(|r| r.as_f64()) {
                    argv.push("--contrast-threshold".into());
                    argv.push(format!("{t}"));
                }
            }
        }
        if let Some(b) = banding {
            if let Some(region) = b.get("region").and_then(|r| r.as_str()) {
                argv.push("--banding".into());
                argv.push(region.into());
            }
        }
        argv.push("--json".into());

        match run_json(ctx.gamut_bin, &argv, ctx.workdir) {
            ShellJson::Ok(resp) => grade_pixels(resp, expect, &argv, ctx.gamut_bin, start),
            other => fail(other.into_failure_detail(&argv_with_bin(ctx.gamut_bin, &argv)), start),
        }
    }
}

fn grade_pixels(
    resp: Value,
    expect: Option<&serde_yaml::Value>,
    argv: &[String],
    bin: &std::path::Path,
    start: Instant,
) -> ValidatorOutcome {
    // color_at — response shape: { color_in: { selector: { mean_hex, delta_e, within } } }
    if let Some(c) = expect.and_then(|e| e.get("color_at")) {
        let sel = c.get("selector").and_then(|x| x.as_str());
        let expected_hex = c.get("hex").and_then(|x| x.as_str()).unwrap_or("").to_string();
        let tol = c.get("tolerance").and_then(|t| t.as_f64()).unwrap_or(5.0);
        if let Some(sel) = sel {
            let probe = resp.pointer("/color_in").and_then(|m| m.get(sel));
            let actual_hex = probe
                .and_then(|p| p.get("mean_hex").or_else(|| p.get("hex")))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let delta_e = probe.and_then(|p| p.get("delta_e")).and_then(|v| v.as_f64()).unwrap_or(f64::INFINITY);
            if delta_e > tol {
                return ValidatorOutcome {
                    ok: false,
                    detail: json!({
                        "argv": argv_with_bin(bin, argv),
                        "failed_clause": "color_at",
                        "selector": sel,
                        "expected_hex": expected_hex,
                        "actual_hex": actual_hex,
                        "delta_e": delta_e,
                        "tolerance": tol,
                    }),
                    cost_usd: 0.0,
                    wall_ms: start.elapsed().as_millis(),
                };
            }
        }
    }
    if let Some(c) = expect.and_then(|e| e.get("contrast")) {
        let sel = c.get("selector").and_then(|s| s.as_str()).unwrap_or("");
        let min = c.get("min_ratio").and_then(|r| r.as_f64()).unwrap_or(4.5);
        let probe = resp.pointer("/contrast").and_then(|m| m.get(sel));
        let ratio = probe.and_then(|p| p.get("ratio")).and_then(|v| v.as_f64()).unwrap_or(0.0);
        if ratio < min {
            return ValidatorOutcome {
                ok: false,
                detail: json!({
                    "argv": argv_with_bin(bin, argv),
                    "failed_clause": "contrast",
                    "selector": sel,
                    "expected_min": min,
                    "actual_ratio": ratio,
                }),
                cost_usd: 0.0,
                wall_ms: start.elapsed().as_millis(),
            };
        }
    }
    if let Some(b) = expect.and_then(|e| e.get("banding")) {
        let max_score = b.get("max_score").and_then(|x| x.as_f64()).unwrap_or(0.5);
        let score = resp.pointer("/banding/score").and_then(|v| v.as_f64()).unwrap_or(0.0);
        if score > max_score {
            return ValidatorOutcome {
                ok: false,
                detail: json!({
                    "argv": argv_with_bin(bin, argv),
                    "failed_clause": "banding",
                    "expected_max": max_score,
                    "actual_score": score,
                }),
                cost_usd: 0.0,
                wall_ms: start.elapsed().as_millis(),
            };
        }
    }

    ValidatorOutcome {
        ok: true,
        detail: json!({ "argv": argv_with_bin(bin, argv), "response": resp }),
        cost_usd: 0.0,
        wall_ms: start.elapsed().as_millis(),
    }
}

/// `query_snapshot` — params `{comp, at, snapshot: <path>, expect_exists}`.
/// Generates the snapshot JSON by shelling `wavelet query --snapshot --json`
/// (which embeds the full FrameSnapshot in stdout), writes the JSON to
/// `snapshot`, then asserts the file exists with non-zero size.
pub struct QuerySnapshot;

impl Validator for QuerySnapshot {
    fn kind(&self) -> &'static str { "query_snapshot" }

    fn check(&self, params: &serde_yaml::Value, ctx: &ValidatorCtx) -> ValidatorOutcome {
        let start = Instant::now();
        let Some(comp) = params.get("comp").and_then(|v| v.as_str()) else {
            return missing("comp", start);
        };
        let Some(snap_rel) = params.get("snapshot").and_then(|v| v.as_str()) else {
            return missing("snapshot", start);
        };
        let at = params.get("at").and_then(|v| v.as_str()).unwrap_or("");
        let expect_exists = params
            .get("expect_exists")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);

        let mut argv = vec!["query".into(), comp.to_string(), "--snapshot".into(), "--json".into()];
        if !at.is_empty() {
            argv.push("--at".into());
            argv.push(at.into());
        }

        let snap_path: PathBuf = ctx.workdir.join(snap_rel);
        match run_json(ctx.gamut_bin, &argv, ctx.workdir) {
            ShellJson::Ok(v) => {
                let dump = serde_json::to_vec_pretty(&v).unwrap_or_default();
                if let Some(parent) = snap_path.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                let write_res = std::fs::write(&snap_path, &dump);
                let meta = std::fs::metadata(&snap_path);
                let exists_ok = matches!(&meta, Ok(m) if m.is_file() && m.len() > 0);
                let ok = expect_exists == exists_ok && write_res.is_ok();
                let detail = json!({
                    "argv": argv_with_bin(ctx.gamut_bin, &argv),
                    "snapshot_path": snap_rel,
                    "exists": exists_ok,
                    "size": meta.as_ref().map(|m| m.len()).unwrap_or(0),
                    "write_error": write_res.err().map(|e| e.to_string()),
                });
                ValidatorOutcome {
                    ok,
                    detail,
                    cost_usd: 0.0,
                    wall_ms: start.elapsed().as_millis(),
                }
            }
            other => fail(
                other.into_failure_detail(&argv_with_bin(ctx.gamut_bin, &argv)),
                start,
            ),
        }
    }
}

/// `query_beat` — params: `{comp, at?, on_beat: <audio>, tolerance_ms,
/// expect: {within_tolerance: true}}`.
pub struct QueryBeat;

impl Validator for QueryBeat {
    fn kind(&self) -> &'static str { "query_beat" }

    fn check(&self, params: &serde_yaml::Value, ctx: &ValidatorCtx) -> ValidatorOutcome {
        let start = Instant::now();
        let Some(comp) = params.get("comp").and_then(|v| v.as_str()) else {
            return missing("comp", start);
        };
        let Some(audio) = params.get("on_beat").and_then(|v| v.as_str()) else {
            return missing("on_beat", start);
        };
        let tol = params
            .get("tolerance_ms")
            .and_then(|v| v.as_u64())
            .unwrap_or(33);
        let at = params.get("at").and_then(|v| v.as_str()).unwrap_or("");

        let mut argv = vec![
            "query".into(),
            comp.to_string(),
            "--on-beat".into(),
            audio.into(),
            "--tolerance-ms".into(),
            tol.to_string(),
            "--json".into(),
        ];
        if !at.is_empty() {
            argv.push("--at".into());
            argv.push(at.into());
        }

        match run_json(ctx.gamut_bin, &argv, ctx.workdir) {
            ShellJson::Ok(resp) => grade_beat(resp, params, &argv, ctx.gamut_bin, start),
            other => fail(
                other.into_failure_detail(&argv_with_bin(ctx.gamut_bin, &argv)),
                start,
            ),
        }
    }
}

fn grade_beat(
    resp: Value,
    params: &serde_yaml::Value,
    argv: &[String],
    bin: &std::path::Path,
    start: Instant,
) -> ValidatorOutcome {
    let want_within = params
        .get("expect")
        .and_then(|e| e.get("within_tolerance"))
        .and_then(|v| v.as_bool())
        .unwrap_or(true);

    // `on_beat` response shape (per wavelet::query::on_beat): array of
    // ScoredEvent under key `on_beat`, each with `within_tolerance: bool`.
    let events = resp
        .get("on_beat")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let total = events.len();
    let off: Vec<&Value> = events
        .iter()
        .filter(|e| e.get("within_tolerance").and_then(|b| b.as_bool()) != Some(true))
        .collect();
    let all_within = off.is_empty() && total > 0;
    let ok = want_within == all_within;

    let detail = if ok {
        json!({
            "argv": argv_with_bin(bin, argv),
            "events_total": total,
            "events_off_beat": off.len(),
        })
    } else {
        json!({
            "argv": argv_with_bin(bin, argv),
            "failed_clause": "within_tolerance",
            "events_total": total,
            "events_off_beat": off.len(),
            "off_beat_sample": off.iter().take(8).cloned().collect::<Vec<_>>(),
        })
    };

    ValidatorOutcome { ok, detail, cost_usd: 0.0, wall_ms: start.elapsed().as_millis() }
}

fn missing(param: &str, start: Instant) -> ValidatorOutcome {
    ValidatorOutcome {
        ok: false,
        detail: json!({ "error": "missing_param", "param": param }),
        cost_usd: 0.0,
        wall_ms: start.elapsed().as_millis(),
    }
}

fn fail(detail: Value, start: Instant) -> ValidatorOutcome {
    ValidatorOutcome {
        ok: false,
        detail,
        cost_usd: 0.0,
        wall_ms: start.elapsed().as_millis(),
    }
}
