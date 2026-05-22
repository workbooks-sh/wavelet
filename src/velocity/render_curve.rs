//! Render a velocity curve as a standalone SVG. Pure string-building —
//! no external SVG dep. Output goes straight to stdout or a file.
//!
//! Optional overlay: detected BPM points from a `ValidationReport`, so
//! the user can eyeball the proposed-vs-actual fit at a glance.

use crate::velocity::{ValidationReport, VelocityProfile};
use std::fmt::Write;

/// SVG canvas size + padding. 800×320 is big enough for screenshots and
/// small enough to drop inline into a markdown doc.
const W: f32 = 800.0;
const H: f32 = 320.0;
const PAD_LEFT: f32 = 60.0;
const PAD_RIGHT: f32 = 20.0;
const PAD_TOP: f32 = 30.0;
const PAD_BOTTOM: f32 = 40.0;

const PLOT_MIN_BPM: f32 = 40.0;
const PLOT_MAX_BPM: f32 = 180.0;

/// Render `profile` as a standalone SVG string. If `validation` is
/// provided, overlay detected-BPM points and a global verdict.
pub fn render_curve_svg(profile: &VelocityProfile, validation: Option<&ValidationReport>) -> String {
    let plot_w = W - PAD_LEFT - PAD_RIGHT;
    let plot_h = H - PAD_TOP - PAD_BOTTOM;
    let duration = profile.duration_secs.max(0.01);

    let x_of = |t: f32| PAD_LEFT + (t / duration) * plot_w;
    let y_of = |bpm: f32| {
        PAD_TOP + (1.0 - ((bpm - PLOT_MIN_BPM) / (PLOT_MAX_BPM - PLOT_MIN_BPM)).clamp(0.0, 1.0))
            * plot_h
    };

    let mut svg = String::with_capacity(2048);
    let _ = write!(
        svg,
        "<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 {w} {h}' width='{w}' height='{h}' font-family='sans-serif'>",
        w = W,
        h = H,
    );

    svg.push_str("<rect width='100%' height='100%' fill='#0c0c10'/>");

    let mut bpm = PLOT_MIN_BPM;
    while bpm <= PLOT_MAX_BPM {
        let y = y_of(bpm);
        let _ = write!(
            svg,
            "<line x1='{px:.1}' y1='{y:.1}' x2='{w:.1}' y2='{y:.1}' stroke='#1f1f29' stroke-width='1'/>",
            px = PAD_LEFT,
            w = W - PAD_RIGHT,
        );
        let _ = write!(
            svg,
            "<text x='{x:.1}' y='{y:.1}' fill='#5e6275' font-size='10' text-anchor='end' dy='3'>{bpm:.0}</text>",
            x = PAD_LEFT - 6.0,
        );
        bpm += 20.0;
    }

    for i in 0..=5 {
        let t = (i as f32 / 5.0) * duration;
        let x = x_of(t);
        let _ = write!(
            svg,
            "<line x1='{x:.1}' y1='{ty:.1}' x2='{x:.1}' y2='{by:.1}' stroke='#1f1f29' stroke-width='1'/>",
            ty = PAD_TOP,
            by = H - PAD_BOTTOM,
        );
        let _ = write!(
            svg,
            "<text x='{x:.1}' y='{y:.1}' fill='#5e6275' font-size='10' text-anchor='middle'>{t:.1}s</text>",
            y = H - PAD_BOTTOM + 14.0,
        );
    }

    if profile.anchors.len() >= 2 {
        let mut path = String::from("M ");
        for (i, a) in profile.anchors.iter().enumerate() {
            if i > 0 {
                path.push_str(" L ");
            }
            let _ = write!(path, "{:.1},{:.1}", x_of(a.t), y_of(a.bpm));
        }
        let _ = write!(
            svg,
            "<path d='{path}' fill='none' stroke='#7dd3fc' stroke-width='2'/>",
        );
    }

    for a in &profile.anchors {
        let cx = x_of(a.t);
        let cy = y_of(a.bpm);
        let _ = write!(
            svg,
            "<circle cx='{cx:.1}' cy='{cy:.1}' r='3' fill='#7dd3fc'/>"
        );
        if let Some(label) = &a.label {
            let _ = write!(
                svg,
                "<text x='{cx:.1}' y='{ly:.1}' fill='#9ba3af' font-size='9' text-anchor='middle'>{label}</text>",
                ly = cy - 8.0,
                label = escape_svg(label),
            );
        }
    }

    if let Some(report) = validation {
        for f in &report.findings {
            if let Some(detected) = f.detected_bpm {
                let cx = x_of(f.t);
                let cy = y_of(detected);
                let color = if f.within_tolerance { "#86efac" } else { "#fb7185" };
                let _ = write!(
                    svg,
                    "<circle cx='{cx:.1}' cy='{cy:.1}' r='3' fill='{color}' fill-opacity='0.9'/>",
                );
            }
        }
        let verdict = if report.ok { "OK" } else { "DRIFT" };
        let vcolor = if report.ok { "#86efac" } else { "#fb7185" };
        let _ = write!(
            svg,
            "<text x='{x:.1}' y='{y:.1}' fill='{vcolor}' font-size='11' text-anchor='end'>{verdict} · {a}/{t} aligned · worst {w:.1} BPM</text>",
            x = W - PAD_RIGHT,
            y = PAD_TOP - 10.0,
            a = report.aligned,
            t = report.total,
            w = report.worst_delta_bpm,
        );
    }

    let mean = profile.mean_bpm;
    let _ = write!(
        svg,
        "<text x='{x:.1}' y='{y:.1}' fill='#e5e7eb' font-size='12'>Velocity profile · {dur:.1}s · mean {mean:.1} BPM</text>",
        x = PAD_LEFT,
        y = PAD_TOP - 10.0,
        dur = profile.duration_secs,
        mean = mean,
    );

    svg.push_str("</svg>");
    svg
}

fn escape_svg(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::velocity::{Anchor, VelocityProfile};

    #[test]
    fn svg_contains_polyline() {
        let p = VelocityProfile {
            duration_secs: 10.0,
            mean_bpm: 0.0,
            anchors: vec![
                Anchor { t: 0.0, bpm: 80.0, label: Some("open".into()) },
                Anchor { t: 5.0, bpm: 130.0, label: None },
                Anchor { t: 10.0, bpm: 90.0, label: Some("close".into()) },
            ],
        };
        let svg = render_curve_svg(&p, None);
        assert!(svg.starts_with("<svg"));
        assert!(svg.contains("<path"));
        assert!(svg.contains("Velocity profile"));
        assert!(svg.contains("close"));
    }

    #[test]
    fn empty_profile_doesnt_panic() {
        let p = VelocityProfile { duration_secs: 1.0, mean_bpm: 0.0, anchors: vec![] };
        let svg = render_curve_svg(&p, None);
        assert!(svg.contains("<svg"));
        assert!(svg.contains("</svg>"));
    }

    #[test]
    fn label_is_escaped() {
        let p = VelocityProfile {
            duration_secs: 2.0,
            mean_bpm: 0.0,
            anchors: vec![Anchor {
                t: 0.0,
                bpm: 100.0,
                label: Some("a < b & c".into()),
            }],
        };
        let svg = render_curve_svg(&p, None);
        assert!(svg.contains("a &lt; b &amp; c"));
    }
}
