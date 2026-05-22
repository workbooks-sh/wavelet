use std::process::ExitCode;
use crate::render_offline::{Composition, render_composition};
use crate::query::VisibilityVerdict;
use std::time::Instant;
use crate::handlers::util::QueryArgs;
use crate::handlers::util::QueryEntry;
use crate::handlers::util::QueryOutput;
use crate::handlers::util::QuerySummary;
use crate::handlers::util::parse_rect;
use crate::handlers::util::parse_time;
use crate::handlers::util::parse_xy;
use crate::query::FramePixels;
use crate::query::FrameSnapshot;
use crate::query::banding;
use crate::query::bbox_of;
use crate::query::color_at;
use crate::query::color_in;
use crate::query::contrast;
use crate::query::events_from_composition;
use crate::query::in_safe_area;
use crate::query::no_overlap;
use crate::query::on_beat;
use crate::query::text_visible;
use crate::query::transform_inherits;
use crate::query::visibility_of;

/// (auto-generated placeholder)
pub fn run_query(args: QueryArgs) -> ExitCode {
    let total_start = Instant::now();

    let (comp, root_dir) = match Composition::from_json_path(&args.comp) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("error loading {}: {e}", args.comp.display());
            return ExitCode::from(3);
        }
    };

    let t_secs = if args.at.is_empty() {
        (comp.duration_frames as f32 / comp.fps as f32) / 2.0
    } else {
        match parse_time(&args.at, comp.fps) {
            Ok(t) => t,
            Err(e) => {
                eprintln!("invalid --at: {e}");
                return ExitCode::from(3);
            }
        }
    };

    let any_query = args.bbox.is_some()
        || args.visible.is_some()
        || args.safe_sel.is_some()
        || args.xform_sel.is_some()
        || args.no_overlap
        || args.color_at.is_some()
        || args.color_in.is_some()
        || args.contrast.is_some()
        || args.banding.is_some()
        || args.on_beat.is_some()
        || args.text_visible.is_some()
        || args.snapshot;
    if !any_query {
        eprintln!(
            "no query ops provided. one of: --bbox / --visible / --in-safe-area / \
             --transform-inherits / --color-at / --color-in / --contrast / --banding / \
             --on-beat / --snapshot"
        );
        return ExitCode::from(3);
    }

    // Build snapshot eagerly (cheap; no pixel render).
    let snap = FrameSnapshot::at(&comp, &root_dir, t_secs);
    // Render pixels only if a pixel-touching op was requested.
    let needs_pixels = args.color_at.is_some()
        || args.color_in.is_some()
        || args.contrast.is_some()
        || args.banding.is_some()
        || args.text_visible.is_some();
    let pixels: Option<FramePixels> = if needs_pixels {
        FramePixels::at(&comp, &root_dir, t_secs)
    } else {
        None
    };

    let mut entries: Vec<QueryEntry> = Vec::new();
    let mut passed = 0usize;
    let mut failed = 0usize;

    if let Some(sel) = args.bbox.as_deref() {
        let t = Instant::now();
        let r = bbox_of(&snap, sel);
        if r.ok { passed += 1 } else { failed += 1 }
        entries.push(QueryEntry::Bbox {
            ok: r.ok,
            selector: r.selector,
            bbox: r.bbox,
            exec_ms: t.elapsed().as_millis(),
        });
    }
    if let Some(sel) = args.visible.as_deref() {
        let t = Instant::now();
        let verdict = visibility_of(&snap, sel);
        let ok = matches!(verdict, VisibilityVerdict::Visible);
        if ok { passed += 1 } else { failed += 1 }
        entries.push(QueryEntry::Visible {
            ok,
            selector: sel.to_string(),
            verdict,
            exec_ms: t.elapsed().as_millis(),
        });
    }
    if let Some(sel) = args.safe_sel.as_deref() {
        let t = Instant::now();
        let r = in_safe_area(&snap, sel, args.inset);
        if r.ok { passed += 1 } else { failed += 1 }
        entries.push(QueryEntry::InSafeArea {
            ok: r.ok,
            selector: r.selector,
            bbox: r.bbox,
            safe_area: r.safe_area,
            inset: r.inset,
            exec_ms: t.elapsed().as_millis(),
        });
    }
    if let Some(sel) = args.xform_sel.as_deref() {
        let t = Instant::now();
        let r = transform_inherits(&snap, sel);
        if r.ok { passed += 1 } else { failed += 1 }
        entries.push(QueryEntry::TransformInherits {
            ok: r.ok,
            selector: r.selector,
            affected_ancestors: r.affected_ancestors,
            exec_ms: t.elapsed().as_millis(),
        });
    }
    if args.no_overlap {
        let t = Instant::now();
        let r = no_overlap(&snap);
        if r.ok { passed += 1 } else { failed += 1 }
        entries.push(QueryEntry::NoOverlap {
            ok: r.ok,
            overlaps: r.overlaps,
            exec_ms: t.elapsed().as_millis(),
        });
    }
    if let Some(xy) = args.color_at.as_deref() {
        let t = Instant::now();
        let (x, y) = match parse_xy(xy) {
            Some(p) => p,
            None => {
                eprintln!("invalid --color-at '{xy}', want 'x,y'");
                return ExitCode::from(3);
            }
        };
        let r = pixels
            .as_ref()
            .map(|p| color_at(p, x, y))
            .unwrap_or(crate::query::ColorAtResult { ok: false, color: None, hex: None });
        if r.ok { passed += 1 } else { failed += 1 }
        entries.push(QueryEntry::ColorAt {
            ok: r.ok,
            x,
            y,
            color: r.color,
            hex: r.hex,
            exec_ms: t.elapsed().as_millis(),
        });
    }
    if let Some(spec) = args.color_in.as_deref() {
        let t = Instant::now();
        let (sel, target) = match spec.split_once('=') {
            Some(p) => p,
            None => {
                eprintln!("invalid --color-in '{spec}', want '<selector>=#rrggbb'");
                return ExitCode::from(3);
            }
        };
        let r = pixels
            .as_ref()
            .map(|p| color_in(&snap, p, sel, target, args.max_de))
            .unwrap_or(crate::query::ColorInResult {
                ok: false,
                selector: sel.to_string(),
                mean: None,
                delta_e: None,
                target: target.to_string(),
                max_de: args.max_de,
            });
        if r.ok { passed += 1 } else { failed += 1 }
        entries.push(QueryEntry::ColorIn {
            ok: r.ok,
            selector: r.selector,
            mean: r.mean,
            delta_e: r.delta_e,
            target: r.target,
            max_de: r.max_de,
            exec_ms: t.elapsed().as_millis(),
        });
    }
    if let Some(sel) = args.contrast.as_deref() {
        let t = Instant::now();
        let r = pixels
            .as_ref()
            .map(|p| contrast(&snap, p, sel, args.contrast_threshold))
            .unwrap_or(crate::query::ContrastResult {
                ok: false,
                selector: sel.to_string(),
                ratio: None,
                threshold: args.contrast_threshold,
            });
        if r.ok { passed += 1 } else { failed += 1 }
        entries.push(QueryEntry::Contrast {
            ok: r.ok,
            selector: r.selector,
            ratio: r.ratio,
            threshold: r.threshold,
            exec_ms: t.elapsed().as_millis(),
        });
    }
    if let Some(rect) = args.banding.as_deref() {
        let t = Instant::now();
        let region = match parse_rect(rect) {
            Some(r) => r,
            None => {
                eprintln!("invalid --banding '{rect}', want 'x,y,w,h'");
                return ExitCode::from(3);
            }
        };
        let r = pixels
            .as_ref()
            .map(|p| banding(p, region))
            .unwrap_or(crate::query::BandingResult {
                ok: true,
                unique_colors: 0,
                sampled_rows: 0,
                diversity: 1.0,
            });
        if r.ok { passed += 1 } else { failed += 1 }
        entries.push(QueryEntry::Banding {
            ok: r.ok,
            unique_colors: r.unique_colors,
            sampled_rows: r.sampled_rows,
            diversity: r.diversity,
            exec_ms: t.elapsed().as_millis(),
        });
    }
    if let Some(audio) = args.on_beat.as_ref() {
        let t = Instant::now();
        let audio_path = if audio.is_absolute() {
            audio.clone()
        } else {
            // Resolve relative paths against the composition's root dir so
            // `--on-beat assets/music.mp3` works from any cwd.
            let resolved = root_dir.join(audio);
            if resolved.exists() {
                resolved
            } else {
                audio.clone()
            }
        };
        let events = events_from_composition(&comp);
        match on_beat(&audio_path, &events, args.tolerance_ms) {
            Ok(r) => {
                if r.ok { passed += 1 } else { failed += 1 }
                entries.push(QueryEntry::OnBeat {
                    ok: r.ok,
                    aligned: r.aligned,
                    total: r.total,
                    worst_delta_ms: r.worst_delta_ms,
                    failed: r.failed,
                    tolerance_ms: r.tolerance_ms,
                    onset_count: r.onset_count,
                    events: r.events,
                    exec_ms: t.elapsed().as_millis(),
                });
            }
            Err(e) => {
                eprintln!("--on-beat: {e}");
                return ExitCode::from(3);
            }
        }
    }
    if let Some(text) = args.text_visible.as_deref() {
        let t = Instant::now();
        let r = pixels
            .as_ref()
            .map(|p| text_visible(&snap, p, text, args.text_in.as_deref(), args.text_tolerance, 8))
            .unwrap_or(crate::query::TextVisibleResult {
                ok: false,
                expected: text.to_string(),
                detected: None,
                edit_distance: None,
                tolerance: args.text_tolerance,
                selector: args.text_in.clone(),
                bbox: None,
                error: Some("no rendered pixels".into()),
            });
        if r.ok { passed += 1 } else { failed += 1 }
        entries.push(QueryEntry::TextVisible {
            ok: r.ok,
            expected: r.expected,
            detected: r.detected,
            edit_distance: r.edit_distance,
            tolerance: r.tolerance,
            selector: r.selector,
            error: r.error,
            exec_ms: t.elapsed().as_millis(),
        });
    }
    if args.snapshot {
        let t = Instant::now();
        entries.push(QueryEntry::Snapshot {
            node_count: snap.nodes.len(),
            exec_ms: t.elapsed().as_millis(),
        });
    }

    let out = QueryOutput {
        comp: args.comp.display().to_string(),
        t_secs: snap.t_secs,
        frame_index: snap.frame_index,
        queries: entries,
        summary: QuerySummary {
            passed,
            failed,
            total_ms: total_start.elapsed().as_millis(),
        },
    };

    if args.snapshot {
        let snap_json = serde_json::to_string_pretty(&snap).expect("snapshot serialize");
        eprintln!("{snap_json}");
    }
    let result_json = serde_json::to_string_pretty(&out).expect("query result serialize");
    println!("{result_json}");

    if failed > 0 {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    }
}
