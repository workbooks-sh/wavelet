//! `baked-text-ocr` lint rule — catches garbled or off-brand text baked
//! into AI-generated video frames.
//!
//! ## Why this rule exists
//!
//! Veo and similar text-to-video models hallucinate letterforms. The 008
//! New Balance eval used a Veo prompt that asked the model to form the
//! brand wordmark out of paint strokes — Veo sometimes garbled the
//! letters. The existing rules walk only the HTML DOM and can never see
//! pixels baked into the video; this rule samples 4 frames from the final
//! MP4 and runs PaddleOCR v5 ONNX inference to detect those failures.
//!
//! ## Two finding classes
//!
//! - **`baked-text`** (subkind): low OCR confidence on a detected region
//!   indicates likely garbled letterforms (confidence < `GARBLE_THRESHOLD`).
//! - **`baked-text-mismatch`** (subkind): high-confidence text whose
//!   content does not appear in the expected brand/product token set
//!   extracted from the commercial HTML (title, `data-brand` attrs, etc.).
//!
//! ## Feature gate
//!
//! Inference requires the `ocr` cargo feature and PaddleOCR model files
//! (≈ 60 MB). When the feature is disabled this module still compiles and
//! emits one `Info` finding per run telling the user how to opt in.
//!
//! ## Integration
//!
//! Called from `handlers/lint.rs` when `--mp4` is provided and the rule
//! `baked-text-ocr` is in the enabled-rules list.

use std::path::Path;

use super::report::{LintFinding, Severity};
use crate::query::Rect;

#[cfg(feature = "ocr")]
use super::mp4_frames;

/// Rule identifier emitted in [`LintFinding::rule`].
pub const RULE: &str = "baked-text-ocr";

/// Subkind for garbled-letterform findings.
pub const SUBKIND_GARBLE: &str = "baked-text";

/// Subkind for brand-mismatch findings.
pub const SUBKIND_MISMATCH: &str = "baked-text-mismatch";

/// OCR confidence below this threshold is treated as probable garbling.
/// PaddleOCR v5 typically scores 0.90+ on clean Latin text, 0.40–0.60 on
/// distorted or hallucinated letterforms.
pub const GARBLE_THRESHOLD: f32 = 0.60;

/// Minimum confidence a box must reach before we compare its text to
/// the expected brand set. Below this we already flag it as garbled —
/// no need for an additional mismatch finding.
const MISMATCH_MIN_CONFIDENCE: f32 = 0.70;

/// Number of evenly-spaced frames to sample from the MP4.
#[cfg(feature = "ocr")]
const SAMPLE_FRAMES: u32 = 4;

/// Run the `baked-text-ocr` pass against `mp4_path`.
///
/// `scene_path` is used only for provenance in the emitted findings.
/// `expected_tokens` is the set of brand/product strings extracted from
/// the commercial HTML (lower-cased, one token per element).
/// `canvas_w` / `canvas_h` set the sample frame resolution.
///
/// Returns an empty Vec when the `ocr` feature is not enabled (no models
/// available) or when `ffmpeg` fails to sample the MP4.
pub fn run(
    mp4_path: &Path,
    scene_path: &Path,
    expected_tokens: &[String],
    canvas_w: u32,
    canvas_h: u32,
) -> Vec<LintFinding> {
    #[cfg(not(feature = "ocr"))]
    {
        let _ = (mp4_path, scene_path, expected_tokens, canvas_w, canvas_h);
        return vec![ocr_disabled_info(scene_path)];
    }

    #[cfg(feature = "ocr")]
    {
        run_with_ocr(mp4_path, scene_path, expected_tokens, canvas_w, canvas_h)
    }
}

/// Fallback finding emitted when the `ocr` feature is disabled.
fn ocr_disabled_info(scene_path: &Path) -> LintFinding {
    LintFinding {
        rule: RULE.to_string(),
        severity: Severity::Info,
        scene_path: scene_path.to_path_buf(),
        t_secs: 0.0,
        element_selector: String::new(),
        element_bbox: Rect { x: 0.0, y: 0.0, w: 0.0, h: 0.0 },
        message: "baked-text-ocr rule is disabled — the `ocr` cargo feature is not enabled. \
                  Recompile with `--features ocr` to detect garbled AI-generated letterforms."
            .to_string(),
        fix_hint: "cargo build --features ocr  (downloads ~60 MB PaddleOCR v5 ONNX models on \
                   first run to ~/.wavelet/models/ocr/)"
            .to_string(),
        subkind: Some(SUBKIND_GARBLE.to_string()),
    }
}

#[cfg(feature = "ocr")]
fn run_with_ocr(
    mp4_path: &Path,
    scene_path: &Path,
    expected_tokens: &[String],
    canvas_w: u32,
    canvas_h: u32,
) -> Vec<LintFinding> {
    use crate::ocr::{ensure_models, run_ocr};

    let model_dir = match ensure_models() {
        Ok(d) => d,
        Err(e) => {
            eprintln!("wavelet lint baked-text-ocr: model error: {e}");
            return Vec::new();
        }
    };

    let duration = mp4_frames::probe_duration_secs(mp4_path).unwrap_or(12.0);
    let mut findings = Vec::new();

    for i in 0..SAMPLE_FRAMES {
        let frac = (i as f32 + 0.5) / SAMPLE_FRAMES as f32;
        let t = (duration * frac).max(0.0);

        let Some(frame) = mp4_frames::sample_frame_rgba(mp4_path, t, canvas_w, canvas_h) else {
            eprintln!("wavelet lint baked-text-ocr: ffmpeg sample failed at t={t:.2}s");
            continue;
        };

        let result = match run_ocr(&frame.rgba, frame.width, frame.height, &model_dir) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("wavelet lint baked-text-ocr: OCR failed at t={t:.2}s: {e}");
                continue;
            }
        };

        for ocr_box in &result.boxes {
            findings.extend(findings_for_box(
                ocr_box,
                expected_tokens,
                scene_path,
                t,
                canvas_w,
                canvas_h,
            ));
        }
    }

    findings
}

/// Produce findings for one OCR box, if warranted.
///
/// Split out so it can be called from unit tests with a mock OCR result.
pub fn findings_for_box(
    ocr_box: &crate::ocr::OcrBox,
    expected_tokens: &[String],
    scene_path: &Path,
    t_secs: f32,
    _frame_w: u32,
    _frame_h: u32,
) -> Vec<LintFinding> {
    let mut out = Vec::new();

    // Bbox in fractional coords → re-map to CSS-px-like canvas coords so
    // the report is consistent with other rules that use canvas units.
    let bbox = Rect {
        x: ocr_box.x as f32,
        y: ocr_box.y as f32,
        w: ocr_box.w as f32,
        h: ocr_box.h as f32,
    };

    // --- Garbled letterforms -----------------------------------------------
    if ocr_box.confidence < GARBLE_THRESHOLD {
        out.push(LintFinding {
            rule: RULE.to_string(),
            severity: Severity::Warn,
            scene_path: scene_path.to_path_buf(),
            t_secs,
            element_selector: format!("frame@{t_secs:.2}s"),
            element_bbox: bbox,
            message: format!(
                "OCR confidence {:.2} (< {:.2}) on region \"{}\". \
                 Likely garbled or hallucinated letterforms from the AI video \
                 generator. Verify the letterforms are legible in the final MP4.",
                ocr_box.confidence,
                GARBLE_THRESHOLD,
                truncate_text(&ocr_box.text, 40),
            ),
            fix_hint: "Review the detected region in the final MP4. If the letterforms \
                       are garbled, regenerate the clip with a stronger prompt or replace \
                       this segment with an HTML overlay that the renderer controls directly."
                .to_string(),
            subkind: Some(SUBKIND_GARBLE.to_string()),
        });
        // Don't also emit a mismatch finding for the same box — it's garbled,
        // so comparing text to the brand set would be noise.
        return out;
    }

    // --- Brand / product name mismatch -----------------------------------
    if ocr_box.confidence >= MISMATCH_MIN_CONFIDENCE && !expected_tokens.is_empty() {
        let normalized = ocr_box.text.to_lowercase();
        let matched = expected_tokens
            .iter()
            .any(|t| normalized.contains(t.as_str()));

        if !matched {
            out.push(LintFinding {
                rule: RULE.to_string(),
                severity: Severity::Warn,
                scene_path: scene_path.to_path_buf(),
                t_secs,
                element_selector: format!("frame@{t_secs:.2}s"),
                element_bbox: bbox,
                message: format!(
                    "High-confidence OCR text \"{}\" does not match any expected brand \
                     or product token ({expected_list}). \
                     The AI video generator may have hallucinated unexpected text content.",
                    truncate_text(&ocr_box.text, 60),
                    expected_list = expected_tokens.join(", "),
                ),
                fix_hint: "Check the baked text against the brief. If the text is an \
                           unintended artefact, regenerate the clip with an explicit \
                           negative prompt (\"no text, no watermark\") or crop it out."
                    .to_string(),
                subkind: Some(SUBKIND_MISMATCH.to_string()),
            });
        }
    }

    out
}

/// Extract brand / product tokens from a commercial HTML file.
///
/// Looks at:
/// - `<title>` content
/// - `data-brand`, `data-product`, `data-brandwork-brand`, `data-adalign-brand` attribute values
/// - `<meta name="brand" content="...">` tags
///
/// Returns lower-cased non-empty tokens. Used by the lint handler to build
/// the `expected_tokens` list for brand-mismatch detection.
pub fn extract_brand_tokens(html: &str) -> Vec<String> {
    let mut tokens: Vec<String> = Vec::new();

    // <title>
    if let Some(t) = extract_between(html, "<title>", "</title>") {
        push_words(&mut tokens, t);
    }

    // data-brand / data-product / data-brandwork-brand / data-adalign-brand (legacy)
    for attr in &["data-brand", "data-product", "data-brandwork-brand", "data-adalign-brand"] {
        for value in extract_attr_values(html, attr) {
            push_words(&mut tokens, &value);
        }
    }

    // <meta name="brand" content="...">
    for value in extract_meta_content(html, "brand") {
        push_words(&mut tokens, &value);
    }
    for value in extract_meta_content(html, "product") {
        push_words(&mut tokens, &value);
    }

    tokens.sort();
    tokens.dedup();
    tokens
}

/// Split `text` on whitespace/punctuation and push tokens ≥ 2 chars
/// (lower-cased) into `out`.
fn push_words(out: &mut Vec<String>, text: &str) {
    for word in text.split(|c: char| !c.is_alphanumeric()) {
        let w = word.trim().to_lowercase();
        if w.len() >= 2 {
            out.push(w);
        }
    }
}

/// Extract content between `open` and `close` delimiters (first match).
fn extract_between<'a>(s: &'a str, open: &str, close: &str) -> Option<&'a str> {
    let start = s.find(open)? + open.len();
    let end = s[start..].find(close)?;
    Some(&s[start..start + end])
}

/// Scan `html` for `attr="value"` or `attr='value'` patterns and return
/// all found values.
fn extract_attr_values(html: &str, attr: &str) -> Vec<String> {
    let mut out = Vec::new();
    let needle = format!("{attr}=");
    let mut pos = 0;
    while let Some(found) = html[pos..].find(&needle) {
        let start = pos + found + needle.len();
        if start >= html.len() {
            break;
        }
        let quote = html[start..].chars().next().unwrap_or(' ');
        if quote == '"' || quote == '\'' {
            let body_start = start + 1;
            if let Some(end) = html[body_start..].find(quote) {
                out.push(html[body_start..body_start + end].to_string());
                pos = body_start + end + 1;
                continue;
            }
        }
        pos = start + 1;
    }
    out
}

/// Extract `<meta name="$name" content="...">` values.
fn extract_meta_content(html: &str, name: &str) -> Vec<String> {
    let mut out = Vec::new();
    let needle = format!("name=\"{name}\"");
    let needle2 = format!("name='{name}'");
    let mut pos = 0;
    while let Some(found) = html[pos..].find(&needle).or_else(|| html[pos..].find(&needle2)) {
        let meta_start = pos + found;
        // Find closing `>` after the name attr.
        let Some(close) = html[meta_start..].find('>') else { break };
        let tag = &html[meta_start..meta_start + close];
        // Look for content= in the same tag.
        let marker = "content=";
        if let Some(c) = tag.find(marker) {
            let rest = &tag[c + marker.len()..];
            if let Some(q) = rest.chars().next() {
                if q == '"' || q == '\'' {
                    if let Some(end) = rest[1..].find(q) {
                        out.push(rest[1..1 + end].to_string());
                    }
                }
            }
        }
        pos = meta_start + close + 1;
    }
    out
}

/// Truncate a string to `max_chars` with an ellipsis indicator.
fn truncate_text(s: &str, max_chars: usize) -> String {
    let chars: Vec<char> = s.chars().collect();
    if chars.len() <= max_chars {
        s.to_string()
    } else {
        chars[..max_chars].iter().collect::<String>() + "…"
    }
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ocr::OcrBox;

    fn scene_path() -> std::path::PathBuf {
        std::path::PathBuf::from("/tmp/test-scene.html")
    }

    /// Garbled box (low confidence) → one baked-text finding, no mismatch.
    #[test]
    fn garbled_box_emits_baked_text_finding() {
        let box_ = OcrBox {
            text: "NbW Ba|ance".to_string(),
            x: 100, y: 200, w: 300, h: 50,
            confidence: 0.42, // below GARBLE_THRESHOLD
        };
        let tokens = vec!["new".to_string(), "balance".to_string()];
        let findings = findings_for_box(&box_, &tokens, &scene_path(), 2.5, 1080, 1920);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].subkind.as_deref(), Some(SUBKIND_GARBLE));
        assert_eq!(findings[0].severity, Severity::Warn);
        assert!(findings[0].message.contains("0.42"));
    }

    /// High-confidence box matching brand → no finding.
    #[test]
    fn brand_match_no_finding() {
        let box_ = OcrBox {
            text: "New Balance".to_string(),
            x: 100, y: 200, w: 300, h: 50,
            confidence: 0.95,
        };
        let tokens = vec!["new".to_string(), "balance".to_string()];
        let findings = findings_for_box(&box_, &tokens, &scene_path(), 1.5, 1080, 1920);
        assert!(findings.is_empty(), "brand match should produce no finding");
    }

    /// High-confidence box NOT matching brand → mismatch finding.
    #[test]
    fn brand_mismatch_emits_mismatch_finding() {
        let box_ = OcrBox {
            text: "Nike Just Do It".to_string(),
            x: 50, y: 50, w: 200, h: 40,
            confidence: 0.92,
        };
        let tokens = vec!["new".to_string(), "balance".to_string()];
        let findings = findings_for_box(&box_, &tokens, &scene_path(), 3.0, 1080, 1920);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].subkind.as_deref(), Some(SUBKIND_MISMATCH));
        assert!(findings[0].message.contains("Nike Just Do It"));
    }

    /// Empty expected_tokens → no mismatch finding (insufficient signal).
    #[test]
    fn no_tokens_no_mismatch_finding() {
        let box_ = OcrBox {
            text: "Whatever text".to_string(),
            x: 0, y: 0, w: 100, h: 30,
            confidence: 0.93,
        };
        let findings = findings_for_box(&box_, &[], &scene_path(), 1.0, 1080, 1920);
        assert!(findings.is_empty());
    }

    /// [`extract_brand_tokens`] picks up the `<title>` words.
    #[test]
    fn extract_brand_tokens_from_title() {
        let html = "<html><head><title>New Balance 990v6</title></head></html>";
        let tokens = extract_brand_tokens(html);
        assert!(tokens.contains(&"new".to_string()), "tokens: {tokens:?}");
        assert!(tokens.contains(&"balance".to_string()), "tokens: {tokens:?}");
        assert!(tokens.contains(&"990v6".to_string()), "tokens: {tokens:?}");
    }

    /// `data-brand` attribute extraction.
    #[test]
    fn extract_brand_tokens_from_data_brand() {
        let html = r#"<div data-brand="New Balance" data-product="990v6"></div>"#;
        let tokens = extract_brand_tokens(html);
        assert!(tokens.contains(&"balance".to_string()), "tokens: {tokens:?}");
        assert!(tokens.contains(&"990v6".to_string()), "tokens: {tokens:?}");
    }

    /// `<meta name="brand">` extraction.
    #[test]
    fn extract_brand_tokens_from_meta_brand() {
        let html = r#"<meta name="brand" content="Whirlpool">"#;
        let tokens = extract_brand_tokens(html);
        assert!(tokens.contains(&"whirlpool".to_string()), "tokens: {tokens:?}");
    }

    #[test]
    fn truncate_text_short_string_unchanged() {
        assert_eq!(truncate_text("hello", 10), "hello");
    }

    #[test]
    fn truncate_text_long_string_gets_ellipsis() {
        let s = truncate_text("hello world foo", 8);
        assert!(s.ends_with('…'), "expected ellipsis: {s}");
        assert!(s.chars().count() <= 9); // 8 chars + ellipsis
    }

    #[test]
    fn extract_attr_values_double_and_single_quotes() {
        let html = r#"<div data-brand="Nike" data-brand='Adidas'></div>"#;
        let vals = extract_attr_values(html, "data-brand");
        assert_eq!(vals.len(), 2);
        assert!(vals.contains(&"Nike".to_string()));
        assert!(vals.contains(&"Adidas".to_string()));
    }

    /// Without the `ocr` feature, `run()` should return the info-level
    /// disabled finding rather than nothing (tells the user how to enable).
    #[test]
    #[cfg(not(feature = "ocr"))]
    fn run_without_ocr_feature_returns_info_finding() {
        let findings = run(
            std::path::Path::new("/tmp/nonexistent.mp4"),
            std::path::Path::new("/tmp/scene.html"),
            &[],
            1080,
            1920,
        );
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].severity, Severity::Info);
        assert!(findings[0].message.contains("ocr"));
    }
}
