//! HTML overlay generator for word-level captions.
//!
//! Takes a [`super::CaptionsResult`] + a style preset and emits a
//! self-contained `<!doctype html>` document the workbook-video
//! renderer can drop into the standard scene-overlay flow. The HTML
//! uses CSS `@keyframes` (one per word) instead of JavaScript so it
//! renders correctly under Blitz / RVST / headless browsers without a
//! JS runtime.
//!
//! ## Style presets
//!
//! - **`hormozi`** — single word at a time, large, bottom-center,
//!   yellow highlight on the emphasis word per beat. Dwell: ~0.25-0.4s.
//! - **`capcut`** — sliding groups of 2-3 words, soft fade. Dwell:
//!   ~0.5-0.8s per group.
//! - **`minimal`** — full sentence at once, classic broadcast
//!   lower-third. Useful for editorial spots.
//!
//! ## Emphasis detection (hormozi)
//!
//! Naive: the longest word in each beat (4-word window) is marked
//! emphasis, with a tie-break preference for ALL-CAPS source words.
//! Documented as a v0 heuristic; future ML upgrade lives behind a
//! follow-up issue (the `wavelet-director` skill calls this out).

use super::{CaptionsResult, WordTimestamp};

/// One of the three style presets. Maps to a different HTML template
/// + CSS animation strategy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OverlayStyle {
    /// Single word at a time, bottom-center, yellow highlight on the
    /// emphasis word per beat.
    Hormozi,
    /// Sliding groups of 2-3 words, soft fade.
    Capcut,
    /// Full sentence at once, classic broadcast lower-third.
    Minimal,
}

impl OverlayStyle {
    /// Parse from a CLI string. Accepts `hormozi`, `capcut`, `minimal`
    /// case-insensitively.
    pub fn from_str(s: &str) -> Result<Self, String> {
        match s.to_ascii_lowercase().as_str() {
            "hormozi" => Ok(Self::Hormozi),
            "capcut" => Ok(Self::Capcut),
            "minimal" => Ok(Self::Minimal),
            other => Err(format!(
                "unknown style '{other}'; want hormozi|capcut|minimal"
            )),
        }
    }

    /// Canonical lowercase string form.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Hormozi => "hormozi",
            Self::Capcut => "capcut",
            Self::Minimal => "minimal",
        }
    }
}

/// Render options. `duration_ms` overrides the audio's natural span
/// when callers want the overlay to last longer than the last word's
/// end timestamp (typical for ad spots that hold the last frame).
#[derive(Debug, Clone)]
pub struct OverlayConfig {
    /// Style preset.
    pub style: OverlayStyle,
    /// Total animation duration. `0` means "use the last word's end_ms".
    pub duration_ms: u32,
    /// Output width hint, in CSS pixels. Drives the font-size scale.
    /// Default: 1080 (vertical 9:16).
    pub width_px: u32,
    /// Output height hint, in CSS pixels. Default: 1920 (vertical 9:16).
    pub height_px: u32,
}

impl Default for OverlayConfig {
    fn default() -> Self {
        Self {
            style: OverlayStyle::Hormozi,
            duration_ms: 0,
            width_px: 1080,
            height_px: 1920,
        }
    }
}

/// Render the overlay HTML for one captions result + config.
pub fn render_overlay_html(result: &CaptionsResult, config: &OverlayConfig) -> String {
    let total_ms = if config.duration_ms > 0 {
        config.duration_ms
    } else {
        result.total_ms.max(1)
    };
    match config.style {
        OverlayStyle::Hormozi => render_hormozi(&result.words, total_ms, config),
        OverlayStyle::Capcut => render_capcut(&result.words, total_ms, config),
        OverlayStyle::Minimal => render_minimal(&result.words, total_ms, config),
    }
}

// -- Hormozi ----------------------------------------------------------------

fn render_hormozi(words: &[WordTimestamp], total_ms: u32, cfg: &OverlayConfig) -> String {
    let mut keyframes = String::new();
    let mut spans = String::new();
    let emphasis_set = emphasis_indexes_per_beat(words, 4);

    for (i, w) in words.iter().enumerate() {
        let start_pct = pct(w.start_ms, total_ms);
        let end_pct = pct(w.end_ms, total_ms);
        let fade_in_pct = clamp_pct(start_pct - 0.5);
        let fade_out_pct = clamp_pct(end_pct + 0.2);

        let kf_name = format!("hormozi_w{i}");
        keyframes.push_str(&format!(
            "@keyframes {kf_name} {{ \
              0% {{ opacity: 0; transform: scale(0.9); }} \
              {fade_in_pct:.3}% {{ opacity: 0; transform: scale(0.9); }} \
              {start_pct:.3}% {{ opacity: 1; transform: scale(1.05); }} \
              {end_pct:.3}% {{ opacity: 1; transform: scale(1); }} \
              {fade_out_pct:.3}% {{ opacity: 0; transform: scale(1); }} \
              100% {{ opacity: 0; transform: scale(1); }} \
            }}\n"
        ));
        let color = if emphasis_set.contains(&i) {
            "#FFD60A" // emphasized yellow
        } else {
            "#FFFFFF"
        };
        spans.push_str(&format!(
            "<span class=\"w\" style=\"color:{color}; animation: {kf_name} {dur}ms linear forwards;\">{word}</span>",
            dur = total_ms,
            word = html_escape(&w.word),
        ));
    }

    let font_size_px = (cfg.width_px as f32 * 0.12).round() as u32;
    format!(
        "<!doctype html><html><head><meta charset=\"utf-8\">\
<style>\n\
:root {{ color-scheme: dark; }}\n\
body, html {{ margin: 0; padding: 0; width: {w}px; height: {h}px; background: transparent; font-family: 'Inter', 'Helvetica Neue', Arial, sans-serif; font-weight: 900; }}\n\
.stage {{ position: relative; width: 100%; height: 100%; }}\n\
.scrim {{ position: absolute; left: 0; right: 0; bottom: 0; height: 35%; background: linear-gradient(to top, rgba(0,0,0,0.55), rgba(0,0,0,0)); pointer-events: none; }}\n\
.captions {{ position: absolute; left: 50%; bottom: 18%; transform: translateX(-50%); width: 90%; text-align: center; }}\n\
.w {{ display: inline-block; position: absolute; left: 50%; transform: translateX(-50%); font-size: {fs}px; letter-spacing: -0.02em; line-height: 1; text-shadow: 0 4px 18px rgba(0,0,0,0.55), 0 0 2px rgba(0,0,0,0.9); opacity: 0; white-space: nowrap; }}\n\
{kf}</style></head><body>\n\
<div class=\"stage\"><div class=\"scrim\"></div><div class=\"captions\">{spans}</div></div>\n\
</body></html>",
        w = cfg.width_px,
        h = cfg.height_px,
        fs = font_size_px,
        kf = keyframes,
    )
}

// -- CapCut -----------------------------------------------------------------

fn render_capcut(words: &[WordTimestamp], total_ms: u32, cfg: &OverlayConfig) -> String {
    // Group every 2-3 words.
    let groups: Vec<(u32, u32, String)> = group_words(words, 3);
    let mut keyframes = String::new();
    let mut spans = String::new();
    for (i, (start_ms, end_ms, text)) in groups.iter().enumerate() {
        let start_pct = pct(*start_ms, total_ms);
        let end_pct = pct(*end_ms, total_ms);
        let pre_pct = clamp_pct(start_pct - 1.0);
        let post_pct = clamp_pct(end_pct + 0.5);
        let kf = format!("capcut_g{i}");
        keyframes.push_str(&format!(
            "@keyframes {kf} {{ \
              0% {{ opacity: 0; transform: translate(-50%, 24px); }} \
              {pre_pct:.3}% {{ opacity: 0; transform: translate(-50%, 24px); }} \
              {start_pct:.3}% {{ opacity: 1; transform: translate(-50%, 0); }} \
              {end_pct:.3}% {{ opacity: 1; transform: translate(-50%, 0); }} \
              {post_pct:.3}% {{ opacity: 0; transform: translate(-50%, -8px); }} \
              100% {{ opacity: 0; transform: translate(-50%, -8px); }} \
            }}\n"
        ));
        spans.push_str(&format!(
            "<span class=\"g\" style=\"animation: {kf} {dur}ms linear forwards;\">{text}</span>",
            dur = total_ms,
            text = html_escape(text),
        ));
    }
    let font_size_px = (cfg.width_px as f32 * 0.07).round() as u32;
    format!(
        "<!doctype html><html><head><meta charset=\"utf-8\">\
<style>\n\
:root {{ color-scheme: dark; }}\n\
body, html {{ margin: 0; padding: 0; width: {w}px; height: {h}px; background: transparent; font-family: 'Inter', 'Helvetica Neue', Arial, sans-serif; font-weight: 800; }}\n\
.stage {{ position: relative; width: 100%; height: 100%; }}\n\
.scrim {{ position: absolute; left: 0; right: 0; bottom: 0; height: 30%; background: linear-gradient(to top, rgba(0,0,0,0.5), rgba(0,0,0,0)); pointer-events: none; }}\n\
.captions {{ position: absolute; left: 0; right: 0; bottom: 14%; height: {fs}px; }}\n\
.g {{ position: absolute; left: 50%; bottom: 0; transform: translate(-50%, 24px); opacity: 0; color: #FFFFFF; background: rgba(0,0,0,0.62); padding: 0.32em 0.6em; border-radius: 0.18em; font-size: {fs}px; letter-spacing: -0.01em; line-height: 1.1; white-space: nowrap; text-shadow: 0 2px 6px rgba(0,0,0,0.5); }}\n\
{kf}</style></head><body>\n\
<div class=\"stage\"><div class=\"scrim\"></div><div class=\"captions\">{spans}</div></div>\n\
</body></html>",
        w = cfg.width_px,
        h = cfg.height_px,
        fs = font_size_px,
        kf = keyframes,
    )
}

// -- Minimal ----------------------------------------------------------------

fn render_minimal(words: &[WordTimestamp], total_ms: u32, cfg: &OverlayConfig) -> String {
    let full: String = words
        .iter()
        .map(|w| w.word.as_str())
        .collect::<Vec<_>>()
        .join(" ");
    // One keyframe block: fade in for the first 5%, hold, fade out for last 5%.
    let kf = "@keyframes minimal_show { \
        0% { opacity: 0; } \
        5% { opacity: 1; } \
        95% { opacity: 1; } \
        100% { opacity: 0; } \
    }\n";
    let font_size_px = (cfg.width_px as f32 * 0.045).round() as u32;
    format!(
        "<!doctype html><html><head><meta charset=\"utf-8\">\
<style>\n\
:root {{ color-scheme: dark; }}\n\
body, html {{ margin: 0; padding: 0; width: {w}px; height: {h}px; background: transparent; font-family: 'Inter', 'Helvetica Neue', Arial, sans-serif; font-weight: 600; }}\n\
.stage {{ position: relative; width: 100%; height: 100%; }}\n\
.scrim {{ position: absolute; left: 0; right: 0; bottom: 0; height: 22%; background: linear-gradient(to top, rgba(0,0,0,0.45), rgba(0,0,0,0)); pointer-events: none; }}\n\
.captions {{ position: absolute; left: 0; right: 0; bottom: 8%; text-align: center; color: #FFFFFF; font-size: {fs}px; line-height: 1.25; letter-spacing: 0; padding: 0 6%; text-shadow: 0 2px 6px rgba(0,0,0,0.6); opacity: 0; animation: minimal_show {dur}ms linear forwards; }}\n\
{kf}</style></head><body>\n\
<div class=\"stage\"><div class=\"scrim\"></div><div class=\"captions\">{text}</div></div>\n\
</body></html>",
        w = cfg.width_px,
        h = cfg.height_px,
        fs = font_size_px,
        dur = total_ms,
        kf = kf,
        text = html_escape(&full),
    )
}

// -- Helpers ----------------------------------------------------------------

fn pct(ms: u32, total: u32) -> f32 {
    if total == 0 {
        return 0.0;
    }
    (ms as f32 / total as f32) * 100.0
}

fn clamp_pct(p: f32) -> f32 {
    p.clamp(0.0, 100.0)
}

fn group_words(words: &[WordTimestamp], group_size: usize) -> Vec<(u32, u32, String)> {
    let g = group_size.max(1);
    words
        .chunks(g)
        .map(|chunk| {
            let start = chunk.first().map(|w| w.start_ms).unwrap_or(0);
            let end = chunk.last().map(|w| w.end_ms).unwrap_or(0);
            let text = chunk
                .iter()
                .map(|w| w.word.as_str())
                .collect::<Vec<_>>()
                .join(" ");
            (start, end, text)
        })
        .collect()
}

/// Per-beat emphasis-word indexes — longest word in each window of
/// `beat_size`. Documented as a v0 heuristic; ALL-CAPS source words
/// outrank longer-but-mixed-case neighbours.
fn emphasis_indexes_per_beat(words: &[WordTimestamp], beat_size: usize) -> std::collections::HashSet<usize> {
    let n = beat_size.max(1);
    let mut out = std::collections::HashSet::new();
    for chunk_start in (0..words.len()).step_by(n) {
        let chunk_end = (chunk_start + n).min(words.len());
        if let Some(pick) = pick_emphasis(&words[chunk_start..chunk_end]) {
            out.insert(chunk_start + pick);
        }
    }
    out
}

fn pick_emphasis(chunk: &[WordTimestamp]) -> Option<usize> {
    if chunk.is_empty() {
        return None;
    }
    // ALL-CAPS first.
    if let Some((i, _)) = chunk.iter().enumerate().find(|(_, w)| is_all_caps(&w.word)) {
        return Some(i);
    }
    // Else longest word (by stripped letter count).
    let mut best = (0, 0usize);
    for (i, w) in chunk.iter().enumerate() {
        let l = w.word.chars().filter(|c| c.is_alphanumeric()).count();
        if l > best.1 {
            best = (i, l);
        }
    }
    Some(best.0)
}

fn is_all_caps(w: &str) -> bool {
    let letters: Vec<char> = w.chars().filter(|c| c.is_alphabetic()).collect();
    letters.len() >= 2 && letters.iter().all(|c| c.is_uppercase())
}

fn html_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '&' => out.push_str("&amp;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            other => out.push(other),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backends::captions::WordTimestamp;

    fn four_word_result() -> CaptionsResult {
        CaptionsResult {
            provider: "test".into(),
            total_ms: 2000,
            words: vec![
                WordTimestamp { word: "fast".into(), start_ms: 0, end_ms: 500 },
                WordTimestamp { word: "cheap".into(), start_ms: 500, end_ms: 1000 },
                WordTimestamp { word: "BIG".into(), start_ms: 1000, end_ms: 1500 },
                WordTimestamp { word: "win".into(), start_ms: 1500, end_ms: 2000 },
            ],
        }
    }

    #[test]
    fn style_parses_each_preset() {
        assert_eq!(OverlayStyle::from_str("hormozi").unwrap(), OverlayStyle::Hormozi);
        assert_eq!(OverlayStyle::from_str("CAPCUT").unwrap(), OverlayStyle::Capcut);
        assert_eq!(OverlayStyle::from_str("Minimal").unwrap(), OverlayStyle::Minimal);
        assert!(OverlayStyle::from_str("foo").is_err());
    }

    #[test]
    fn hormozi_emits_one_keyframe_per_word() {
        let r = four_word_result();
        let cfg = OverlayConfig { style: OverlayStyle::Hormozi, ..Default::default() };
        let html = render_overlay_html(&r, &cfg);
        assert!(html.contains("hormozi_w0"));
        assert!(html.contains("hormozi_w1"));
        assert!(html.contains("hormozi_w2"));
        assert!(html.contains("hormozi_w3"));
        // ALL-CAPS BIG should be the emphasis pick → yellow.
        assert!(html.contains("#FFD60A"));
    }

    #[test]
    fn capcut_groups_words() {
        let r = four_word_result();
        let cfg = OverlayConfig { style: OverlayStyle::Capcut, ..Default::default() };
        let html = render_overlay_html(&r, &cfg);
        // 4 words / group_size 3 → 2 groups.
        assert!(html.contains("capcut_g0"));
        assert!(html.contains("capcut_g1"));
        assert!(!html.contains("capcut_g2"));
    }

    #[test]
    fn minimal_emits_full_sentence() {
        let r = four_word_result();
        let cfg = OverlayConfig { style: OverlayStyle::Minimal, ..Default::default() };
        let html = render_overlay_html(&r, &cfg);
        assert!(html.contains("fast cheap BIG win"));
        assert!(html.contains("minimal_show"));
    }

    #[test]
    fn three_styles_produce_different_html() {
        let r = four_word_result();
        let h = render_overlay_html(&r, &OverlayConfig { style: OverlayStyle::Hormozi, ..Default::default() });
        let c = render_overlay_html(&r, &OverlayConfig { style: OverlayStyle::Capcut, ..Default::default() });
        let m = render_overlay_html(&r, &OverlayConfig { style: OverlayStyle::Minimal, ..Default::default() });
        assert_ne!(h, c);
        assert_ne!(c, m);
        assert_ne!(h, m);
    }

    #[test]
    fn html_escapes_special_chars() {
        let r = CaptionsResult {
            provider: "t".into(),
            total_ms: 1000,
            words: vec![WordTimestamp {
                word: "<b>&\"hi\"</b>".into(),
                start_ms: 0,
                end_ms: 1000,
            }],
        };
        let html = render_overlay_html(&r, &OverlayConfig::default());
        assert!(html.contains("&lt;b&gt;"));
        assert!(!html.contains("<b>&\"hi\"</b>"));
    }

    #[test]
    fn all_caps_detection() {
        assert!(is_all_caps("BIG"));
        assert!(!is_all_caps("big"));
        assert!(!is_all_caps("Big"));
        assert!(!is_all_caps("A")); // single letter not enough
    }
}
