//! HTML-level filter declaration rewriting — strip CSS `filter:`
//! at the source so Blitz never paints them, then re-apply via our own
//! pipeline.

#![allow(missing_docs)]

use super::parse::parse_filter_value;
use super::types::{FilterFn, FilterParseError, Length, LengthUnit};

#[derive(Debug)]
pub struct HijackResult {
    /// HTML with every `filter:` CSS declaration stripped. Pass this to
    /// Blitz instead of the original so the upstream filter
    /// implementation never gets a chance to hang. Elements that had
    /// filter (and where we know how to apply it per-element — inline
    /// style + simple class selectors) are tagged with a
    /// `data-wavelet-fxid="N"` attribute for post-paint bbox lookup.
    pub stripped_html: String,
    /// The filter chain extracted from the `<body>` element's inline
    /// style attribute or from a `body { ... }` CSS rule in a `<style>`
    /// block. Applied as a whole-scene post-process — CSS-spec-correct
    /// because the body's bbox equals the viewport.
    pub body_filter_chain: Vec<FilterFn>,
    /// Per-element filter chains, keyed by the `fxid` injected as a
    /// `data-wavelet-fxid` attribute on the host element. The render
    /// path walks the DOM after layout, finds each fxid, resolves the
    /// element's absolute bbox via Blitz's `absolute_position()`, and
    /// applies the chain to that region of the rendered RGBA.
    ///
    /// Covered: inline `style="filter:..."` on any tag; CSS rules with
    /// simple class selectors (`.classname { filter:... }`).
    ///
    /// NOT covered (still stripped without apply, listed in
    /// `stripped_no_apply`): descendant combinators (`.cls img`), tag
    /// selectors, ID selectors, pseudo-classes, attribute selectors.
    pub element_filter_chains: Vec<(String, Vec<FilterFn>)>,
    /// Filter declarations that were stripped without being applied —
    /// either body filter (re-applied separately via
    /// `body_filter_chain`) or CSS rules whose selector shape we don't
    /// yet handle. Surfaced as a diagnostic so the agent / harness
    /// can warn about effects that won't render.
    pub stripped_no_apply: Vec<String>,
}

/// Walk scene HTML, extract any body-level `filter:` declaration, and
/// strip ALL `filter:` declarations so the unsupported / pathological
/// ones (per wb-5w9s.1.2 audit: `filter: blur(N>=4)`, drop-shadow,
/// brightness/saturate on `<video>`) never reach Blitz/Vello.
///
/// MVP scope:
/// - Body-level filter: captured + applied as a whole-scene post-process
///   in the render path. CSS-spec-correct because the body's bbox
///   equals the viewport.
/// - Non-body filter: stripped (to prevent the hang) + reported in
///   `stripped_no_apply` so the harness can warn. Per-element bbox
///   support lands in round-2.
///
/// Implementation: text-level regex, not a full CSS parser. Handles the
/// shapes scenes actually use:
/// - `<body style="...; filter: X; ...">`
/// - `<style>body { ...; filter: X; ... }</style>`
/// - `<style>.cls { ...; filter: X; ... }</style>` (the non-body case)
/// - `<div style="filter: X">` (inline non-body case)
///
/// Does NOT handle filter inside `@media` queries cleanly — the strip
/// catches them but the body-vs-non-body classification may misroute
/// edge cases. Good enough for ad creatives.
pub fn hijack_filters_in_html(html: &str) -> HijackResult {
    let mut body_chain: Vec<FilterFn> = Vec::new();
    let mut stripped_no_apply: Vec<String> = Vec::new();
    let mut element_filter_chains: Vec<(String, Vec<FilterFn>)> = Vec::new();
    // Injections to apply BEFORE the strip pass — sorted descending by
    // position so applying them doesn't shift earlier offsets.
    let mut injections: Vec<(usize, String)> = Vec::new();
    // Track byte ranges of declarations we've already classified as
    // per-element (so the strip pass doesn't double-count them in
    // stripped_no_apply).
    let mut classified_decls: Vec<String> = Vec::new();
    let mut next_fxid: u32 = 0;

    // 1. Extract body-level filter from inline style on the <body> tag.
    //    Find the <body ...> open tag, look for style="...".
    if let Some((body_tag_start, body_tag_end)) = locate_body_open_tag(html) {
        let body_tag = &html[body_tag_start..body_tag_end];
        if let Some(style_val) = extract_attr_value(body_tag, "style") {
            for (_decl, chain_str) in iter_filter_decls(&style_val) {
                if let Ok(chain) = parse_filter_value(&chain_str) {
                    body_chain.extend(chain);
                }
            }
        }
    }

    // 2. Walk <style> blocks rule by rule. For each rule with a
    //    filter declaration, classify the selector:
    //    - Body → contribute to body_filter_chain
    //    - Simple class → find host elements in HTML, plan markers
    //    - Unsupported → strip-without-apply
    for style_block in iter_style_blocks(html) {
        for (selector, declarations) in iter_css_rules(&style_block) {
            let decls = iter_filter_decls(&declarations);
            if decls.is_empty() {
                continue;
            }
            match classify_selector(&selector) {
                SimpleSelector::Body => {
                    for (_d, chain_str) in &decls {
                        if let Ok(chain) = parse_filter_value(chain_str) {
                            body_chain.extend(chain);
                        }
                    }
                }
                SimpleSelector::Class(class_name) => {
                    // Build a chain combining all filter decls in the rule.
                    let mut rule_chain: Vec<FilterFn> = Vec::new();
                    for (_d, chain_str) in &decls {
                        if let Ok(c) = parse_filter_value(chain_str) {
                            rule_chain.extend(c);
                        }
                        classified_decls.push(chain_str.trim().to_string());
                    }
                    if rule_chain.is_empty() {
                        continue;
                    }
                    let fxid = format!("fx{}", next_fxid);
                    next_fxid += 1;
                    // Find every element with this class — plan one
                    // marker per match (all share the same fxid).
                    let positions = find_class_match_tags(html, &class_name);
                    if positions.is_empty() {
                        // Class declared in CSS but no element uses it.
                        // Effectively dead CSS — still strip the filter,
                        // but don't bother recording the chain.
                        continue;
                    }
                    for pos in positions {
                        injections.push((pos, format!(" data-wavelet-fxid=\"{fxid}\"")));
                    }
                    element_filter_chains.push((fxid, rule_chain));
                }
                SimpleSelector::Unsupported => {
                    for (_d, chain_str) in &decls {
                        stripped_no_apply.push(format!(
                            "filter: {} (selector `{}` unsupported by per-element apply; \
                             effect stripped without rendering)",
                            chain_str.trim(),
                            selector.trim()
                        ));
                        // Mark classified so the strip pass doesn't
                        // double-report this declaration.
                        classified_decls.push(chain_str.trim().to_string());
                    }
                }
            }
        }
    }

    // 2b. Inline-style filters on individual elements. Match any tag
    //     with `style="...filter:..."` (excluding body, since body's
    //     inline style was already extracted into body_chain at step 1).
    let lower_html = html.to_ascii_lowercase();
    let mut search_cursor = 0;
    while search_cursor < html.len() {
        let Some(rel) = lower_html[search_cursor..].find("style=") else { break };
        let style_attr_start = search_cursor + rel;
        // Make sure we're inside a tag, not inside a CSS string. The
        // simple heuristic: look backward for `<`; if we hit `>` first
        // we're in text content.
        let mut i = style_attr_start;
        let mut inside_tag = false;
        while i > 0 {
            i -= 1;
            match html.as_bytes()[i] {
                b'<' => { inside_tag = true; break; }
                b'>' => { break; }
                _ => {}
            }
        }
        if !inside_tag {
            search_cursor = style_attr_start + "style=".len();
            continue;
        }
        // Extract the style value, find any filter declarations.
        let after_eq = style_attr_start + "style=".len();
        let bytes = html.as_bytes();
        let q = match bytes.get(after_eq).copied() {
            Some(b'"') | Some(b'\'') => bytes[after_eq],
            _ => {
                search_cursor = after_eq;
                continue;
            }
        };
        let value_start = after_eq + 1;
        let value_end_rel = match html[value_start..].find(q as char) {
            Some(r) => r,
            None => break,
        };
        let value_end = value_start + value_end_rel;
        let style_val = &html[value_start..value_end];
        let decls = iter_filter_decls(style_val);
        if !decls.is_empty() {
            // Check if this tag is the <body> — if so, skip (already handled).
            let host_start = (0..style_attr_start)
                .rev()
                .find(|&i| html.as_bytes()[i] == b'<')
                .unwrap_or(0);
            let host_tag_start = host_start;
            let is_body = lower_html[host_tag_start..]
                .starts_with("<body")
                && lower_html
                    .as_bytes()
                    .get(host_tag_start + 5)
                    .map_or(false, |b| matches!(b, b' ' | b'>' | b'\t' | b'\n' | b'\r'));
            if !is_body {
                let mut rule_chain: Vec<FilterFn> = Vec::new();
                for (_d, chain_str) in &decls {
                    if let Ok(c) = parse_filter_value(chain_str) {
                        rule_chain.extend(c);
                    }
                    classified_decls.push(chain_str.trim().to_string());
                }
                if !rule_chain.is_empty() {
                    if let Some(insert) = find_inline_filter_host_tag(html, value_start) {
                        let fxid = format!("fx{}", next_fxid);
                        next_fxid += 1;
                        injections.push((insert, format!(" data-wavelet-fxid=\"{fxid}\"")));
                        element_filter_chains.push((fxid, rule_chain));
                    }
                }
            }
        }
        search_cursor = value_end + 1;
    }

    // Collect the body-level filter declaration values we already
    // extracted; the strip pass below uses this to avoid mislabeling
    // them as "stripped non-body" in the diagnostic.
    let body_already_extracted: Vec<String> = {
        // Re-derive raw declaration strings from the body inline style +
        // body { ... } rules so we can match by exact text. The
        // extraction above goes through parse_filter_value which is
        // lossy on whitespace/casing, hence the second pass here.
        let mut out = Vec::new();
        if let Some((s, e)) = locate_body_open_tag(html) {
            if let Some(style_val) = extract_attr_value(&html[s..e], "style") {
                for (_, v) in iter_filter_decls(&style_val) {
                    out.push(v);
                }
            }
        }
        for style_block in iter_style_blocks(html) {
            for (selector, declarations) in iter_css_rules(&style_block) {
                if !selector_matches_body(&selector) {
                    continue;
                }
                for (_, v) in iter_filter_decls(&declarations) {
                    out.push(v);
                }
            }
        }
        out
    };
    // 3a. Apply marker injections to the HTML BEFORE the strip pass.
    //     Sort descending by position so each injection doesn't shift
    //     earlier offsets.
    let html_with_markers = if injections.is_empty() {
        html.to_string()
    } else {
        let mut sorted = injections;
        sorted.sort_by(|a, b| b.0.cmp(&a.0));
        let mut out = html.to_string();
        for (pos, attr) in sorted {
            out.insert_str(pos, &attr);
        }
        out
    };

    // 3b. Strip every `filter:` declaration anywhere. Capture stripped
    //    declarations for the diagnostic. This pass also strips the
    //    body-level + per-element ones we already extracted — they're
    //    being re-applied via the post-process, not via Blitz, so they
    //    need to go.
    let html_for_strip = html_with_markers.as_str();
    let mut stripped_html = String::with_capacity(html_for_strip.len());
    let mut cursor = 0;
    let bytes = html_for_strip.as_bytes();
    while cursor < bytes.len() {
        // Find next `filter:` (case-insensitive) skipping over content
        // already copied.
        let remainder = &html_for_strip[cursor..];
        let lower = remainder.to_ascii_lowercase();
        let Some(rel) = lower.find("filter:") else {
            stripped_html.push_str(remainder);
            break;
        };
        // Look backward briefly to skip 'backdrop-filter:' (which is a
        // different property we don't intercept).
        let abs_filter = cursor + rel;
        let is_backdrop = abs_filter >= 9
            && html_for_strip[abs_filter - 9..abs_filter].eq_ignore_ascii_case("backdrop-");
        if is_backdrop {
            stripped_html.push_str(&html_for_strip[cursor..abs_filter + "filter:".len()]);
            cursor = abs_filter + "filter:".len();
            continue;
        }
        // Append the chunk before the filter:.
        stripped_html.push_str(&html_for_strip[cursor..abs_filter]);
        // Find the end of this declaration. In HTML attribute values
        // ('style="..."'), declarations end at `;` or at the closing
        // quote. In CSS rules, declarations end at `;` or `}`. Scan
        // for the soonest of `;`, `}`, `"`, `'`.
        let after_colon = abs_filter + "filter:".len();
        let tail = &html_for_strip[after_colon..];
        let end_offset = tail
            .find(|c: char| matches!(c, ';' | '}' | '"' | '\''))
            .unwrap_or(tail.len());
        let decl_value = tail[..end_offset].trim().to_string();
        // Record the stripped declaration for the diagnostic — but
        // only if it's NOT one we already extracted (either as body
        // filter or as per-element). Body + classified per-element
        // declarations get re-applied via the post-process, so
        // reporting them here would mislead the agent into thinking
        // their effect didn't render.
        let is_classified = body_already_extracted
            .iter()
            .any(|v| v.trim() == decl_value)
            || classified_decls.iter().any(|v| v.trim() == decl_value);
        if !decl_value.is_empty() && !is_classified {
            stripped_no_apply.push(format!("filter: {decl_value}"));
        }
        // Move cursor past the declaration value, but PRESERVE the
        // terminator (`;` or `}` or quote) so the surrounding CSS / HTML
        // stays well-formed.
        cursor = after_colon + end_offset;
    }

    HijackResult {
        stripped_html,
        body_filter_chain: body_chain,
        element_filter_chains,
        stripped_no_apply,
    }
}

/// Find the `<body ...>` open tag and return its byte range.
fn locate_body_open_tag(html: &str) -> Option<(usize, usize)> {
    let lower = html.to_ascii_lowercase();
    let start = lower.find("<body")?;
    // Make sure this is actually the body tag (not <bodysomething>).
    let after = lower.as_bytes().get(start + 5).copied();
    if !matches!(after, Some(b' ') | Some(b'>') | Some(b'\t') | Some(b'\n') | Some(b'\r')) {
        return None;
    }
    let end_rel = html[start..].find('>')?;
    Some((start, start + end_rel + 1))
}

/// Extract a quoted attribute value from a tag string. Returns None
/// when the attribute is absent or malformed.
fn extract_attr_value(tag: &str, attr: &str) -> Option<String> {
    let lower = tag.to_ascii_lowercase();
    let needle = format!("{attr}=");
    let idx = lower.find(&needle)?;
    let after_eq = idx + needle.len();
    let bytes = tag.as_bytes();
    let quote = *bytes.get(after_eq)?;
    if quote != b'"' && quote != b'\'' {
        return None;
    }
    let value_start = after_eq + 1;
    let value_end = tag[value_start..].find(quote as char)?;
    Some(tag[value_start..value_start + value_end].to_string())
}

/// Iterate over `filter: X;` declarations inside a style-attr or
/// rule-body string. Yields `(full_decl_with_terminator, value_only)`.
fn iter_filter_decls(text: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let lower = text.to_ascii_lowercase();
    let mut cursor = 0;
    while let Some(rel) = lower[cursor..].find("filter:") {
        let abs = cursor + rel;
        let is_backdrop = abs >= 9 && text[abs - 9..abs].eq_ignore_ascii_case("backdrop-");
        if is_backdrop {
            cursor = abs + "filter:".len();
            continue;
        }
        let after = abs + "filter:".len();
        let tail = &text[after..];
        let end = tail
            .find(|c: char| matches!(c, ';' | '}'))
            .unwrap_or(tail.len());
        let value = tail[..end].trim().to_string();
        let full = text[abs..after + end].to_string();
        out.push((full, value));
        cursor = after + end;
    }
    out
}

/// Iterate over `<style>...</style>` block contents. CSS comments
/// (`/* ... */`) are stripped from each block before it's returned so
/// downstream selector parsers don't see them.
fn iter_style_blocks(html: &str) -> Vec<String> {
    let mut out = Vec::new();
    let lower = html.to_ascii_lowercase();
    let mut cursor = 0;
    while let Some(rel) = lower[cursor..].find("<style") {
        let open = cursor + rel;
        // Skip to '>' to find the start of the block.
        let body_start = html[open..].find('>').map(|i| open + i + 1);
        let Some(body_start) = body_start else { break };
        // Find the matching </style>.
        let close_rel = lower[body_start..].find("</style>");
        let Some(close_rel) = close_rel else { break };
        let body = &html[body_start..body_start + close_rel];
        out.push(strip_css_comments(body));
        cursor = body_start + close_rel + "</style>".len();
    }
    out
}

/// Remove `/* ... */` comments from a CSS string. Naive — assumes
/// comments are well-formed and don't appear inside string literals
/// (which CSS technically allows via `content:` but is exceedingly
/// rare in scene HTML).
fn strip_css_comments(css: &str) -> String {
    let mut out = String::with_capacity(css.len());
    let mut cursor = 0;
    while cursor < css.len() {
        if css[cursor..].starts_with("/*") {
            match css[cursor + 2..].find("*/") {
                Some(end) => cursor = cursor + 2 + end + 2,
                None => break,
            }
        } else {
            out.push(css.as_bytes()[cursor] as char);
            cursor += 1;
        }
    }
    out
}

/// Iterate over `selector { declarations }` rules inside a CSS block.
fn iter_css_rules(css: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let bytes = css.as_bytes();
    let mut cursor = 0;
    while cursor < bytes.len() {
        // Find next '{' as the selector/decl-block boundary. We're
        // tolerant of @media + nested rules — just scan past them.
        let open = match css[cursor..].find('{') {
            Some(r) => cursor + r,
            None => break,
        };
        let selector = css[cursor..open].trim().to_string();
        // Find matching '}' counting nesting depth.
        let mut depth = 1usize;
        let mut idx = open + 1;
        while idx < bytes.len() && depth > 0 {
            match bytes[idx] {
                b'{' => depth += 1,
                b'}' => depth -= 1,
                _ => {}
            }
            idx += 1;
        }
        if depth != 0 {
            break;
        }
        let declarations = css[open + 1..idx - 1].to_string();
        out.push((selector, declarations));
        cursor = idx;
    }
    out
}

/// Classify a CSS selector into one of the shapes the per-element
/// hijack knows how to target. Anything outside this list falls back
/// to strip-without-apply.
enum SimpleSelector {
    /// `body`, `body.cls`, `body#id`, etc. — handled by the whole-
    /// scene body filter path (different code).
    Body,
    /// `.classname` — match every tag with that class.
    Class(String),
    /// Anything else (descendant combinators, multiple class chains,
    /// attribute selectors, pseudo-classes, tag selectors, etc.). The
    /// strip pass still removes the filter declaration but per-element
    /// apply is skipped.
    Unsupported,
}

fn classify_selector(selector: &str) -> SimpleSelector {
    let s = selector.trim();
    // Multi-selector lists ("a, b, c"): take the first item — if
    // we can match it, we'll match that one and miss the others.
    // Imperfect but safe.
    let first = s.split(',').next().unwrap_or(s).trim();
    if selector_matches_body(first) {
        return SimpleSelector::Body;
    }
    if let Some(class) = first.strip_prefix('.') {
        // Reject anything that has further selector punctuation
        // (descendant, attribute, pseudo).
        if class.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
            return SimpleSelector::Class(class.to_string());
        }
    }
    SimpleSelector::Unsupported
}

/// Find every open-tag whose `class="..."` attribute contains the
/// given class. Returns the byte offset of the tag-name end (the
/// position where we want to inject a new attribute).
///
/// Returns `(insert_offset, tag_open_start)` pairs. The insert offset
/// is where the new attribute should land (right after the tag name +
/// any existing attributes, just before the closing `>` or `/>`).
fn find_class_match_tags(html: &str, class_name: &str) -> Vec<usize> {
    let mut out = Vec::new();
    let lower = html.to_ascii_lowercase();
    let bytes = html.as_bytes();
    let mut cursor = 0;
    while cursor < bytes.len() {
        // Find next '<' that starts an open tag (not '</' or '<!').
        let Some(rel) = lower[cursor..].find('<') else { break };
        let tag_start = cursor + rel;
        let after_lt = tag_start + 1;
        if after_lt >= bytes.len() {
            break;
        }
        let first = bytes[after_lt];
        // Skip closing tags, doctype, comments, end-of-input cases.
        if first == b'/' || first == b'!' || !first.is_ascii_alphabetic() {
            cursor = after_lt;
            continue;
        }
        // Find tag end '>' (not inside an attribute value).
        let tag_end = match find_tag_end(html, tag_start) {
            Some(e) => e,
            None => break,
        };
        let tag = &html[tag_start..=tag_end];
        // Check for class="...class_name..." attribute.
        if let Some(class_attr) = extract_attr_value(tag, "class") {
            // Split by whitespace, exact match.
            if class_attr.split_ascii_whitespace().any(|c| c == class_name) {
                // Insert position: right before the closing `>` (or `/>`).
                let insert = if bytes.get(tag_end.saturating_sub(1)).copied() == Some(b'/') {
                    tag_end - 1
                } else {
                    tag_end
                };
                out.push(insert);
            }
        }
        cursor = tag_end + 1;
    }
    out
}

/// Find the closing `>` of a tag starting at `<`, accounting for
/// quoted attribute values that may contain `>`.
fn find_tag_end(html: &str, start: usize) -> Option<usize> {
    let bytes = html.as_bytes();
    let mut i = start;
    let mut in_quote: Option<u8> = None;
    while i < bytes.len() {
        let b = bytes[i];
        match in_quote {
            Some(q) if b == q => in_quote = None,
            None => {
                if b == b'"' || b == b'\'' {
                    in_quote = Some(b);
                } else if b == b'>' {
                    return Some(i);
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

/// Find the byte position to inject a `data-wavelet-fxid` attribute on a
/// tag whose `style="..."` attribute we're stripping filter from. For
/// inline-style filters: the tag containing the `style=` attribute is
/// the host. Returns insert position (just before the closing `>`).
fn find_inline_filter_host_tag(html: &str, style_text_byte_in_html: usize) -> Option<usize> {
    // Walk backward from the style_text position to find the enclosing
    // `<tagname ...>` open tag.
    let bytes = html.as_bytes();
    let mut i = style_text_byte_in_html;
    while i > 0 {
        if bytes[i] == b'<' && bytes.get(i + 1).copied().map_or(false, |b| b.is_ascii_alphabetic()) {
            // Found the tag start. Get the tag end.
            return find_tag_end(html, i).map(|e| {
                if bytes.get(e.saturating_sub(1)).copied() == Some(b'/') {
                    e - 1
                } else {
                    e
                }
            });
        }
        i -= 1;
    }
    None
}

/// Check if a selector matches the body element. Accepts `body`,
/// `body.something`, `html body`, `body, html`, etc. — anything that
/// names body somewhere in the selector list.
fn selector_matches_body(selector: &str) -> bool {
    selector
        .split(',')
        .any(|s| {
            let trimmed = s.trim().to_ascii_lowercase();
            trimmed == "body"
                || trimmed.ends_with(" body")
                || trimmed.starts_with("body ")
                || trimmed.starts_with("body.")
                || trimmed.starts_with("body#")
                || trimmed.starts_with("body[")
                || trimmed.starts_with("body:")
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_single_blur() {
        let r = parse_filter_value("blur(8px)").unwrap();
        assert_eq!(r.len(), 1);
        match &r[0] {
            FilterFn::Blur(l) => {
                assert_eq!(l.value, 8.0);
                assert_eq!(l.unit, LengthUnit::Px);
            }
            other => panic!("expected Blur, got {other:?}"),
        }
    }

    #[test]
    fn parses_blur_without_unit_defaults_to_px() {
        let r = parse_filter_value("blur(4)").unwrap();
        match &r[0] {
            FilterFn::Blur(l) => assert_eq!(l.unit, LengthUnit::Px),
            other => panic!("got {other:?}"),
        }
    }

    #[test]
    fn parses_blur_with_em_unit() {
        let r = parse_filter_value("blur(0.5em)").unwrap();
        match &r[0] {
            FilterFn::Blur(l) => {
                assert_eq!(l.value, 0.5);
                assert_eq!(l.unit, LengthUnit::Em);
            }
            other => panic!("got {other:?}"),
        }
    }

    #[test]
    fn parses_brightness_with_percentage() {
        let r = parse_filter_value("brightness(85%)").unwrap();
        assert!(matches!(&r[0], FilterFn::Brightness(v) if (*v - 0.85).abs() < 1e-6));
    }

    #[test]
    fn parses_chain_with_multiple_functions() {
        let r = parse_filter_value("blur(8px) brightness(0.85) saturate(0.92)").unwrap();
        assert_eq!(r.len(), 3);
        matches!(r[0], FilterFn::Blur(_));
        matches!(r[1], FilterFn::Brightness(_));
        matches!(r[2], FilterFn::Saturate(_));
    }

    #[test]
    fn parses_drop_shadow_with_hex_color() {
        let r = parse_filter_value("drop-shadow(0 30px 40px #00000088)").unwrap();
        match &r[0] {
            FilterFn::DropShadow { offset_x, offset_y, blur_radius, color } => {
                assert_eq!(offset_x.value, 0.0);
                assert_eq!(offset_y.value, 30.0);
                assert_eq!(blur_radius.value, 40.0);
                // #00000088 = rgba(0, 0, 0, 0.533)
                assert!((color[3] - (0x88 as f32 / 255.0)).abs() < 1e-6);
            }
            other => panic!("got {other:?}"),
        }
    }

    #[test]
    fn parses_drop_shadow_with_rgba() {
        let r = parse_filter_value("drop-shadow(0 30px 40px rgba(0, 0, 0, 0.85))").unwrap();
        match &r[0] {
            FilterFn::DropShadow { color, .. } => {
                assert!((color[3] - 0.85).abs() < 1e-6);
            }
            other => panic!("got {other:?}"),
        }
    }

    #[test]
    fn parses_multi_filter_with_drop_shadow_nested_parens() {
        // The exact failing pattern from the 004 eval's scene-1.html.
        let raw = "brightness(0.78) contrast(1.05) \
                   drop-shadow(0 30px 40px rgba(0,0,0,0.85)) \
                   drop-shadow(-40px 0px 60px rgba(255, 120, 30, 0.20))";
        let r = parse_filter_value(raw).unwrap();
        assert_eq!(r.len(), 4);
        matches!(r[0], FilterFn::Brightness(_));
        matches!(r[1], FilterFn::Contrast(_));
        matches!(r[2], FilterFn::DropShadow { .. });
        matches!(r[3], FilterFn::DropShadow { .. });
    }

    #[test]
    fn parses_hue_rotate_degrees() {
        let r = parse_filter_value("hue-rotate(45deg)").unwrap();
        assert!(matches!(&r[0], FilterFn::HueRotate(v) if (*v - 45.0).abs() < 1e-6));
    }

    #[test]
    fn parses_hue_rotate_turn() {
        let r = parse_filter_value("hue-rotate(0.5turn)").unwrap();
        assert!(matches!(&r[0], FilterFn::HueRotate(v) if (*v - 180.0).abs() < 1e-6));
    }

    #[test]
    fn rejects_none_keyword() {
        assert!(parse_filter_value("none").is_err());
    }

    #[test]
    fn rejects_unknown_function() {
        let r = parse_filter_value("kaleidoscope(4)");
        assert!(matches!(r, Err(FilterParseError::UnknownFunction(s)) if s == "kaleidoscope"));
    }

    #[test]
    fn rejects_unterminated_function() {
        let r = parse_filter_value("blur(8px");
        assert!(matches!(r, Err(FilterParseError::Unterminated(_))));
    }

    #[test]
    fn length_to_px_resolves_units() {
        let l = Length { value: 50.0, unit: LengthUnit::Vw };
        assert_eq!(l.to_px(1080.0, 1920.0, 16.0), 540.0);
        let l = Length { value: 50.0, unit: LengthUnit::Vh };
        assert_eq!(l.to_px(1080.0, 1920.0, 16.0), 960.0);
        let l = Length { value: 2.0, unit: LengthUnit::Em };
        assert_eq!(l.to_px(1080.0, 1920.0, 16.0), 32.0);
    }

    #[test]
    fn hijack_extracts_body_filter_from_inline_style() {
        let html = r#"<html><body style="margin:0;background:#000;filter:blur(8px) brightness(0.85)">
          <video src="x.mp4"></video>
        </body></html>"#;
        let r = hijack_filters_in_html(html);
        assert_eq!(r.body_filter_chain.len(), 2);
        matches!(r.body_filter_chain[0], FilterFn::Blur(_));
        matches!(r.body_filter_chain[1], FilterFn::Brightness(_));
        assert!(!r.stripped_html.to_ascii_lowercase().contains("filter:"),
            "stripped html should have no filter: declarations, got: {}", r.stripped_html);
    }

    #[test]
    fn hijack_extracts_body_filter_from_style_block() {
        let html = r#"<html><head><style>
          html, body { margin: 0; padding: 0; }
          body { filter: brightness(0.9) saturate(1.1); }
          .candle { filter: blur(28px); }
        </style></head><body><div class="candle"></div></body></html>"#;
        let r = hijack_filters_in_html(html);
        // body chain: brightness + saturate
        assert_eq!(r.body_filter_chain.len(), 2);
        // All filter declarations gone from output
        assert!(!r.stripped_html.to_ascii_lowercase().contains("filter:"),
            "stripped html should have no filter: declarations, got: {}", r.stripped_html);
        // .candle is a simple class selector → routes through
        // element_filter_chains (per-element apply), NOT
        // stripped_no_apply (which is for selectors we can't target).
        assert_eq!(r.element_filter_chains.len(), 1,
            "candle class selector should yield one per-element entry");
        let (_fxid, chain) = &r.element_filter_chains[0];
        assert_eq!(chain.len(), 1);
        matches!(chain[0], FilterFn::Blur(_));
        // The host element should have a data-wavelet-fxid marker injected.
        assert!(r.stripped_html.contains("data-wavelet-fxid="),
            "host element should have a fxid marker, got: {}", r.stripped_html);
    }

    #[test]
    fn hijack_routes_inline_non_body_filter_to_per_element() {
        // Inline-style filter on a non-body element now routes through
        // element_filter_chains (per-element apply), not stripped_no_apply.
        let html = r#"<html><body>
          <div style="filter: blur(28px)">candle</div>
          <img style="filter: brightness(0.78) contrast(1.05)" src="x.png"/>
        </body></html>"#;
        let r = hijack_filters_in_html(html);
        assert!(r.body_filter_chain.is_empty(), "no body filter declared");
        assert!(!r.stripped_html.contains("filter:"), "must strip");
        assert_eq!(r.element_filter_chains.len(), 2,
            "two inline filters → two per-element entries");
        // Both host elements get fxid markers (2 occurrences in stripped HTML).
        let marker_count = r.stripped_html.matches("data-wavelet-fxid=").count();
        assert_eq!(marker_count, 2);
        // stripped_no_apply is empty because we classified everything.
        assert!(r.stripped_no_apply.is_empty(),
            "all inline filters classified; nothing should be in stripped_no_apply, got: {:?}",
            r.stripped_no_apply);
    }

    #[test]
    fn hijack_preserves_backdrop_filter() {
        // backdrop-filter is a different property; don't touch it.
        let html = r#"<html><body><div style="backdrop-filter: blur(8px); filter: invert(1)">x</div></body></html>"#;
        let r = hijack_filters_in_html(html);
        assert!(r.stripped_html.contains("backdrop-filter"),
            "backdrop-filter should be preserved, got: {}", r.stripped_html);
        // The plain `filter:` after it should still be stripped.
        assert!(!r.stripped_html.contains("filter: invert") &&
                !r.stripped_html.contains("filter:invert"));
    }

    #[test]
    fn hijack_handles_eval_004_scene_1() {
        // The actual scene-1.html shape that hangs render today.
        let html = r#"<!doctype html><html><head><style>
          html, body { margin: 0; padding: 0; width: 100%; height: 100%; background: #000; }
          .plate { position: absolute; inset: 0; filter: brightness(0.85) saturate(0.92); }
          .candle-warmth {
            position: absolute; left: 8%; top: 58%; width: 32%; height: 28%;
            filter: blur(28px);
          }
          .product-wrap img {
            filter: brightness(0.78) contrast(1.05)
              drop-shadow(0 30px 40px rgba(0,0,0,0.85));
          }
        </style></head><body>
          <video class="plate" src="../shots/shot-1.mp4"></video>
          <div class="candle-warmth"></div>
          <div class="product-wrap"><img src="../product.png"/></div>
        </body></html>"#;
        let r = hijack_filters_in_html(html);
        // No body-level filter in this scene → body_filter_chain empty.
        assert!(r.body_filter_chain.is_empty());
        // All three filter declarations stripped from CSS.
        assert!(!r.stripped_html.to_ascii_lowercase().contains("filter:"),
            "all filter decls should be stripped, got: {}", r.stripped_html);
        // .plate and .candle-warmth are simple class selectors → routed
        // to element_filter_chains for per-element apply.
        assert_eq!(r.element_filter_chains.len(), 2,
            "two class-selector filters should land per-element");
        // .product-wrap img is a descendant combinator → unsupported,
        // gets stripped without apply.
        assert_eq!(r.stripped_no_apply.len(), 1,
            "one descendant-selector filter should land in stripped_no_apply");
        // The host elements got fxid markers.
        assert!(r.stripped_html.contains("data-wavelet-fxid="));
    }

    #[test]
    fn parses_004_scene1_video_filter() {
        // Exact filter from the 004-liquid-death scene-1.html .plate rule
        // that hangs Vello/Blitz today.
        let r = parse_filter_value("brightness(0.85) saturate(0.92)").unwrap();
        assert_eq!(r.len(), 2);
        matches!(r[0], FilterFn::Brightness(_));
        matches!(r[1], FilterFn::Saturate(_));
    }
}

