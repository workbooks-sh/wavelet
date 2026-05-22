//! NDJSON REPL dispatcher — agents drive `wavelet query` in a tight loop
//! without paying the per-query process-spawn cost. Phase 6 of epic wb-q4a6.
//!
//! Protocol: one command per line on stdin, one response per line on
//! stdout. Each response carries `_queue_ms` + `_exec_ms`. Frame snapshots
//! and rendered pixels are cached by frame index — 100 queries at the same
//! `--at` time take one render's worth of work.
//!
//! ## Example session
//!
//! ```text
//! $ wavelet query --repl examples/vsmoke/comp.json
//! > {"cmd":"bbox","selector":"#headline","at":"0.5s"}
//! < {"ok":true,"cmd":"bbox","value":{...},"_queue_ms":0,"_exec_ms":48}
//! > {"cmd":"color_at","x":640,"y":360,"at":"0.5s"}
//! < {"ok":true,"cmd":"color_at","value":{...},"_queue_ms":0,"_exec_ms":2}
//! > {"cmd":"close"}
//! ```

use super::pixels::FramePixels;
use super::snapshot::{FrameSnapshot, Rect};
use crate::render_offline::Composition;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::{BufRead, Write};
use std::path::Path;
use std::time::Instant;

/// One request line from the client.
#[derive(Debug, Clone, Deserialize)]
struct ReplRequest {
    cmd: String,
    /// Time at which the query applies. Accepts `"0.5s"`, `"frame:90"`,
    /// `"MM:SS"`, or a bare float (seconds). Defaults to the composition's
    /// midpoint when omitted.
    #[serde(default)]
    at: Option<String>,
    /// Most ops take a CSS-id selector.
    #[serde(default)]
    selector: Option<String>,
    /// Coordinates for `color_at`.
    #[serde(default)]
    x: Option<i32>,
    #[serde(default)]
    y: Option<i32>,
    /// Region for `banding` / `region_avg`.
    #[serde(default)]
    region: Option<[f32; 4]>,
    /// `color_in` target color (hex).
    #[serde(default)]
    target: Option<String>,
    /// `color_in` max ΔE.
    #[serde(default)]
    max_de: Option<f32>,
    /// `contrast` threshold (default 4.5).
    #[serde(default)]
    threshold: Option<f32>,
    /// `in_safe_area` inset (default 0.1).
    #[serde(default)]
    inset: Option<f32>,
    /// `text_visible` expected string.
    #[serde(default)]
    text: Option<String>,
    /// `text_visible` Levenshtein tolerance (default 2).
    #[serde(default)]
    text_tolerance: Option<u32>,
}

/// One response line. Tags the original `cmd` so the client can pipeline
/// requests without tracking ids.
#[derive(Debug, Clone, Serialize)]
struct ReplResponse {
    ok: bool,
    cmd: String,
    value: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
    #[serde(rename = "_queue_ms")]
    queue_ms: u128,
    #[serde(rename = "_exec_ms")]
    exec_ms: u128,
}

/// Run the REPL against `comp`. Reads lines from `stdin`, writes responses
/// to `stdout`. Returns the number of commands processed.
pub fn run(comp: &Composition, root_dir: &Path) -> u64 {
    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    let mut out = stdout.lock();

    let mut snap_cache: HashMap<u32, FrameSnapshot> = HashMap::new();
    let mut pixel_cache: HashMap<u32, FramePixels> = HashMap::new();
    let mut processed = 0u64;

    for line in stdin.lock().lines() {
        let received = Instant::now();
        let line = match line {
            Ok(l) => l,
            Err(e) => {
                let _ = writeln!(
                    out,
                    "{}",
                    json!({
                        "ok": false,
                        "cmd": "<read>",
                        "error": format!("stdin read: {e}"),
                        "_queue_ms": 0,
                        "_exec_ms": 0,
                    })
                );
                continue;
            }
        };
        if line.trim().is_empty() {
            continue;
        }

        let req: ReplRequest = match serde_json::from_str(&line) {
            Ok(r) => r,
            Err(e) => {
                let _ = writeln!(
                    out,
                    "{}",
                    json!({
                        "ok": false,
                        "cmd": "<parse>",
                        "error": format!("invalid request: {e}"),
                        "_queue_ms": received.elapsed().as_millis(),
                        "_exec_ms": 0,
                    })
                );
                let _ = out.flush();
                continue;
            }
        };

        if req.cmd == "close" || req.cmd == "exit" {
            // No response — client wants us gone.
            break;
        }
        if req.cmd == "ping" {
            let _ = writeln!(
                out,
                "{}",
                json!({
                    "ok": true,
                    "cmd": "ping",
                    "value": { "pong": true },
                    "_queue_ms": received.elapsed().as_millis(),
                    "_exec_ms": 0,
                })
            );
            let _ = out.flush();
            processed += 1;
            continue;
        }

        let exec_start = Instant::now();
        let queue_ms = received.elapsed().as_millis() - exec_start.elapsed().as_millis();

        // Resolve `at` → seconds.
        let t_secs = match resolve_at(&req.at, comp) {
            Ok(t) => t,
            Err(e) => {
                emit_error(&mut out, &req.cmd, e, queue_ms);
                continue;
            }
        };
        // Cache key — round to nearest frame so repeated subsecond queries
        // at the same frame share state.
        let frame_idx = (t_secs * comp.fps as f32).round() as u32;
        let canonical_t = frame_idx as f32 / comp.fps as f32;

        let snap = snap_cache
            .entry(frame_idx)
            .or_insert_with(|| FrameSnapshot::at(comp, root_dir, canonical_t));

        let resp = match req.cmd.as_str() {
            "bbox" => handle_bbox(snap, &req),
            "visible" => handle_visible(snap, &req),
            "in_safe_area" => handle_in_safe_area(snap, &req),
            "transform_inherits" => handle_transform_inherits(snap, &req),
            "no_overlap" => handle_no_overlap(snap),
            "snapshot" => handle_snapshot_meta(snap),
            "color_at" | "color_in" | "contrast" | "banding" | "text_visible" => {
                let pixels = pixel_cache
                    .entry(frame_idx)
                    .or_insert_with(|| {
                        FramePixels::at(comp, root_dir, canonical_t).unwrap_or(FramePixels {
                            rgba: vec![0; (comp.width * comp.height * 4) as usize],
                            width: comp.width,
                            height: comp.height,
                        })
                    });
                match req.cmd.as_str() {
                    "color_at" => handle_color_at(pixels, &req),
                    "color_in" => handle_color_in(snap, pixels, &req),
                    "contrast" => handle_contrast(snap, pixels, &req),
                    "banding" => handle_banding(pixels, &req),
                    "text_visible" => handle_text_visible(snap, pixels, &req),
                    _ => unreachable!(),
                }
            }
            other => Err(format!("unknown cmd '{other}'")),
        };

        let exec_ms = exec_start.elapsed().as_millis();
        match resp {
            Ok(value) => {
                let r = ReplResponse {
                    ok: value
                        .get("ok")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(true),
                    cmd: req.cmd.clone(),
                    value,
                    error: None,
                    queue_ms,
                    exec_ms,
                };
                let _ = writeln!(out, "{}", serde_json::to_string(&r).unwrap());
            }
            Err(e) => {
                emit_error(&mut out, &req.cmd, e, queue_ms);
            }
        }
        let _ = out.flush();
        processed += 1;
    }

    processed
}

fn emit_error<W: Write>(out: &mut W, cmd: &str, e: String, queue_ms: u128) {
    let _ = writeln!(
        out,
        "{}",
        json!({
            "ok": false,
            "cmd": cmd,
            "error": e,
            "_queue_ms": queue_ms,
            "_exec_ms": 0,
        })
    );
    let _ = out.flush();
}

fn resolve_at(at: &Option<String>, comp: &Composition) -> Result<f32, String> {
    let Some(s) = at.as_deref() else {
        return Ok((comp.duration_frames as f32 / comp.fps as f32) / 2.0);
    };
    let s = s.trim();
    if let Some(rest) = s.strip_prefix("frame:") {
        let f: u32 = rest.parse().map_err(|e| format!("frame: {e}"))?;
        return Ok(f as f32 / comp.fps as f32);
    }
    if let Some(stripped) = s.strip_suffix('s') {
        return stripped.parse::<f32>().map_err(|e| format!("seconds: {e}"));
    }
    if let Some((mm, ss)) = s.split_once(':') {
        let m: f32 = mm.parse().map_err(|e| format!("mm: {e}"))?;
        let s2: f32 = ss.parse().map_err(|e| format!("ss: {e}"))?;
        return Ok(m * 60.0 + s2);
    }
    s.parse::<f32>().map_err(|e| format!("at: {e}"))
}

fn require_selector(req: &ReplRequest) -> Result<&str, String> {
    req.selector
        .as_deref()
        .ok_or_else(|| "missing 'selector' field".to_string())
}

// ---- per-command handlers — thin wrappers around the library functions

fn handle_bbox(snap: &FrameSnapshot, req: &ReplRequest) -> Result<Value, String> {
    let sel = require_selector(req)?;
    Ok(serde_json::to_value(super::scene_graph::bbox_of(snap, sel)).unwrap())
}
fn handle_visible(snap: &FrameSnapshot, req: &ReplRequest) -> Result<Value, String> {
    let sel = require_selector(req)?;
    let v = super::scene_graph::visibility_of(snap, sel);
    let ok = matches!(v, crate::query::VisibilityVerdict::Visible);
    Ok(json!({ "ok": ok, "selector": sel, "verdict": v }))
}
fn handle_in_safe_area(snap: &FrameSnapshot, req: &ReplRequest) -> Result<Value, String> {
    let sel = require_selector(req)?;
    let inset = req.inset.unwrap_or(0.1);
    Ok(serde_json::to_value(super::scene_graph::in_safe_area(snap, sel, inset)).unwrap())
}
fn handle_transform_inherits(snap: &FrameSnapshot, req: &ReplRequest) -> Result<Value, String> {
    let sel = require_selector(req)?;
    Ok(serde_json::to_value(super::scene_graph::transform_inherits(snap, sel)).unwrap())
}
fn handle_no_overlap(snap: &FrameSnapshot) -> Result<Value, String> {
    Ok(serde_json::to_value(super::scene_graph::no_overlap(snap)).unwrap())
}
fn handle_snapshot_meta(snap: &FrameSnapshot) -> Result<Value, String> {
    Ok(json!({
        "ok": true,
        "t_secs": snap.t_secs,
        "frame_index": snap.frame_index,
        "active_scene": snap.active_scene,
        "node_count": snap.nodes.len(),
    }))
}
fn handle_color_at(pixels: &FramePixels, req: &ReplRequest) -> Result<Value, String> {
    let x = req.x.ok_or("missing 'x'")?;
    let y = req.y.ok_or("missing 'y'")?;
    Ok(serde_json::to_value(super::pixels::color_at(pixels, x, y)).unwrap())
}
fn handle_color_in(
    snap: &FrameSnapshot,
    pixels: &FramePixels,
    req: &ReplRequest,
) -> Result<Value, String> {
    let sel = require_selector(req)?;
    let target = req.target.as_deref().ok_or("missing 'target' hex")?;
    let max_de = req.max_de.unwrap_or(5.0);
    Ok(serde_json::to_value(super::pixels::color_in(snap, pixels, sel, target, max_de)).unwrap())
}
fn handle_contrast(
    snap: &FrameSnapshot,
    pixels: &FramePixels,
    req: &ReplRequest,
) -> Result<Value, String> {
    let sel = require_selector(req)?;
    let threshold = req.threshold.unwrap_or(4.5);
    Ok(serde_json::to_value(super::pixels::contrast(snap, pixels, sel, threshold)).unwrap())
}
fn handle_banding(pixels: &FramePixels, req: &ReplRequest) -> Result<Value, String> {
    let r = req.region.ok_or("missing 'region' as [x,y,w,h]")?;
    let rect = Rect { x: r[0], y: r[1], w: r[2], h: r[3] };
    Ok(serde_json::to_value(super::pixels::banding(pixels, rect)).unwrap())
}
fn handle_text_visible(
    snap: &FrameSnapshot,
    pixels: &FramePixels,
    req: &ReplRequest,
) -> Result<Value, String> {
    let text = req.text.as_deref().ok_or("missing 'text'")?;
    let in_sel = req.selector.as_deref();
    let tol = req.text_tolerance.unwrap_or(2);
    Ok(serde_json::to_value(super::ocr::text_visible(snap, pixels, text, in_sel, tol, 8)).unwrap())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render_offline::{Composition, SceneSpec};
    use std::path::PathBuf;

    fn mini_comp() -> Composition {
        Composition {
            width: 64,
            height: 64,
            fps: 30,
            duration_frames: 30,
            scenes: vec![SceneSpec {
                html_path: PathBuf::from("dummy.html"),
                start_frame: 0,
                duration_frames: 30,
                transition_in: None, video_bg: None,
            }],
            aspect: None,
            audio_cues: vec![],
        }
    }

    #[test]
    fn resolve_at_handles_forms() {
        let comp = mini_comp();
        assert_eq!(resolve_at(&None, &comp).unwrap(), 0.5);
        assert_eq!(resolve_at(&Some("0.5s".into()), &comp).unwrap(), 0.5);
        assert_eq!(resolve_at(&Some("frame:15".into()), &comp).unwrap(), 0.5);
        assert_eq!(resolve_at(&Some("0:30".into()), &comp).unwrap(), 30.0);
        assert_eq!(resolve_at(&Some("1.25".into()), &comp).unwrap(), 1.25);
        assert!(resolve_at(&Some("bogus".into()), &comp).is_err());
    }
}
