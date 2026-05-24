//! Fountain v1.1 parser. Line-and-paragraph driven; no external dep.
//!
//! Strategy:
//! 1. Split off the boneyard (`/* ... */`) globally first — easier to
//!    handle as a comment strip than to track block state line-by-line.
//! 2. Split off the title page: if the first non-blank line matches the
//!    pattern `Key: value`, consume contiguous key/value pairs until the
//!    first blank line. Multi-line values continue under an indented
//!    second line.
//! 3. Split the remaining body into "blocks" separated by blank lines.
//! 4. Classify each block:
//!    - Forced markers (`.`, `!`, `@`, `~`, `>`, `[[`, `#`, `=`) win
//!      over heuristic detection.
//!    - Otherwise apply the disambiguation rules (scene headings start
//!      with INT./EXT./...; character cues are ALL-CAPS single lines
//!      followed by dialogue; transitions end in `TO:` AND are
//!      uppercase, etc.).
//! 5. Emit the AST.

use crate::ast::*;
use thiserror::Error;

/// Errors a Fountain parse can surface.
#[derive(Debug, Error)]
pub enum ParseError {
    /// The input was empty (after trim).
    #[error("input is empty")]
    Empty,
}

/// Parse a Fountain source string into a `Screenplay`. Doesn't fail on
/// malformed elements; the parser is permissive (per the spec) and
/// reclassifies anything it can't recognize as `Action`.
pub fn parse(source: &str) -> Result<Screenplay, ParseError> {
    let source = strip_boneyard(source);
    if source.trim().is_empty() {
        return Err(ParseError::Empty);
    }

    // Title-page split. The title page is the contiguous prefix of
    // `Key: value` lines from the top, terminated by a blank line.
    let (title_page, body_start) = parse_title_page(&source);
    let body = &source[body_start..];

    let elements = parse_body(body);

    Ok(Screenplay {
        title_page,
        elements,
    })
}

/// Strip `/* … */` boneyard comments globally. Multi-line aware.
fn strip_boneyard(src: &str) -> String {
    let mut out = String::with_capacity(src.len());
    let bytes = src.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'/' && i + 1 < bytes.len() && bytes[i + 1] == b'*' {
            // Find closing */.
            if let Some(end) = src[i + 2..].find("*/") {
                i += 2 + end + 2;
                continue;
            } else {
                // Unterminated; preserve as-is.
                out.push_str(&src[i..]);
                break;
            }
        }
        out.push(bytes[i] as char);
        i += 1;
    }
    out
}

/// Parse the title page, returning `(Option<TitlePage>, body_offset)`.
fn parse_title_page(src: &str) -> (Option<TitlePage>, usize) {
    // Skip leading blank lines.
    let mut offset = 0;
    while let Some(line_end) = src[offset..].find('\n') {
        let line = &src[offset..offset + line_end];
        if line.trim().is_empty() {
            offset += line_end + 1;
            continue;
        }
        break;
    }
    // Look at the first content line; does it have `Key: value` shape?
    let mut probe = offset;
    let first_nonblank = match src[probe..].find('\n') {
        Some(e) => &src[probe..probe + e],
        None => &src[probe..],
    };
    if !is_title_page_key(first_nonblank) {
        return (None, offset);
    }

    let mut tp = TitlePage::default();
    let mut current_key: Option<String> = None;
    let mut current_val: Vec<String> = Vec::new();

    let commit = |tp: &mut TitlePage, key: Option<String>, val: Vec<String>| {
        let Some(key) = key else { return };
        let v = val.join("\n").trim().to_string();
        if v.is_empty() {
            return;
        }
        match key.to_ascii_lowercase().as_str() {
            "title" => tp.title = Some(v),
            "author" | "authors" => tp.author = Some(v),
            "credit" => tp.credit = Some(v),
            "source" => tp.source = Some(v),
            "draft date" | "draft_date" => tp.draft_date = Some(v),
            "contact" => tp.contact = Some(v),
            "copyright" => tp.copyright = Some(v),
            other => {
                tp.other.insert(other.to_string(), v);
            }
        }
    };

    loop {
        let line_end = src[probe..].find('\n').map(|e| probe + e);
        let (line, next_offset) = match line_end {
            Some(end) => (&src[probe..end], end + 1),
            None => (&src[probe..], src.len()),
        };
        // Blank line ends the title page.
        if line.trim().is_empty() {
            commit(&mut tp, current_key.take(), std::mem::take(&mut current_val));
            probe = next_offset;
            // Skip trailing blank lines.
            while let Some(e) = src[probe..].find('\n') {
                if src[probe..probe + e].trim().is_empty() {
                    probe += e + 1;
                } else {
                    break;
                }
            }
            return (Some(tp), probe);
        }

        if let Some((key, val)) = split_title_page_kv(line) {
            // Commit the previous key/value before starting a new one.
            commit(&mut tp, current_key.take(), std::mem::take(&mut current_val));
            current_key = Some(key);
            if !val.is_empty() {
                current_val.push(val);
            }
        } else {
            // Continuation line — must be indented per spec.
            if line.starts_with(['\t', ' ']) {
                current_val.push(line.trim().to_string());
            } else {
                // Not a valid title-page continuation — bail out, treat
                // everything from `probe` onward as body.
                commit(&mut tp, current_key.take(), std::mem::take(&mut current_val));
                return (Some(tp), probe);
            }
        }
        probe = next_offset;
        if probe >= src.len() {
            commit(&mut tp, current_key.take(), std::mem::take(&mut current_val));
            return (Some(tp), src.len());
        }
    }
}

fn is_title_page_key(line: &str) -> bool {
    split_title_page_kv(line).is_some()
}

fn split_title_page_kv(line: &str) -> Option<(String, String)> {
    let trimmed = line.trim_end();
    let colon = trimmed.find(':')?;
    let key = trimmed[..colon].trim();
    if key.is_empty() || key.contains(' ') && !key.split_whitespace().all(|w| w.chars().all(|c| c.is_alphabetic() || c == '_')) {
        // Keys are short alphabetic words (possibly space-separated like
        // "Draft date"). If we have weirder content (sentence-with-colon),
        // it's not a title-page key.
        if !key.chars().all(|c| c.is_ascii_alphabetic() || c == ' ' || c == '_') {
            return None;
        }
    }
    let value = trimmed[colon + 1..].trim();
    Some((key.to_string(), value.to_string()))
}

/// Split body into blocks separated by blank lines, classify each.
fn parse_body(body: &str) -> Vec<Element> {
    let mut out: Vec<Element> = Vec::new();
    let blocks = split_blocks(body);
    let mut i = 0;
    while i < blocks.len() {
        let block = &blocks[i];
        // Single-line blocks may be character cues that introduce a
        // following dialogue block — peek ahead.
        if let Some(elem) = classify_block(block) {
            match elem {
                Element::Dialogue { .. } => {
                    // The classifier returned an empty-dialogue stub —
                    // we need to coalesce subsequent blocks into the
                    // lines. (Dialogue blocks are character + dialogue
                    // lines without intervening blank lines per spec.)
                    // In practice we ALSO let the classifier emit a
                    // complete dialogue if all lines were in one block.
                    out.push(elem);
                }
                other => out.push(other),
            }
        }
        i += 1;
    }
    out
}

fn split_blocks(body: &str) -> Vec<Vec<String>> {
    let mut blocks: Vec<Vec<String>> = Vec::new();
    let mut current: Vec<String> = Vec::new();
    for raw_line in body.lines() {
        let line = strip_notes(raw_line);
        if line.trim().is_empty() {
            if !current.is_empty() {
                blocks.push(std::mem::take(&mut current));
            }
        } else {
            current.push(line);
        }
    }
    if !current.is_empty() {
        blocks.push(current);
    }
    blocks
}

/// Strip `[[ note ]]` notes from a line. Notes can be multiline in the
/// spec; v0 we handle the single-line case.
fn strip_notes(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let bytes = line.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if i + 1 < bytes.len() && bytes[i] == b'[' && bytes[i + 1] == b'[' {
            if let Some(end) = line[i + 2..].find("]]") {
                i += 2 + end + 2;
                continue;
            }
        }
        out.push(bytes[i] as char);
        i += 1;
    }
    out
}

/// Classify a block of lines into a single `Element` (or multiple, for
/// character + dialogue compound blocks). Returns the first element;
/// for compound classifications, additional elements are pushed via the
/// outer loop in `parse_body` — but in practice Fountain dialogue is
/// one block (character cue is the first line, dialogue is the rest).
fn classify_block(block: &[String]) -> Option<Element> {
    let first = block.first()?;
    let first_trimmed = first.trim_start();

    // Forced element prefixes win over heuristic detection.
    if let Some(rest) = first_trimmed.strip_prefix('.') {
        // Forced scene heading — but only if not `..` (which is action
        // with a literal period prefix per some implementations).
        if !rest.starts_with('.') {
            return Some(parse_scene_heading(rest, &block[1..]));
        }
    }
    if let Some(rest) = first_trimmed.strip_prefix('!') {
        return Some(Element::Action {
            text: join_lines(rest, &block[1..]),
            centered: false,
        });
    }
    if let Some(rest) = first_trimmed.strip_prefix('@') {
        // Forced character cue.
        return Some(parse_dialogue_block(rest, &block[1..]));
    }
    if first_trimmed.starts_with("===") {
        return Some(Element::PageBreak);
    }
    if let Some(rest) = first_trimmed.strip_prefix('#') {
        // Section. Count leading #s for level.
        let mut level: u8 = 1;
        let mut after = rest;
        while let Some(s) = after.strip_prefix('#') {
            level += 1;
            after = s;
        }
        return Some(Element::Section {
            level,
            text: after.trim().to_string(),
        });
    }
    if let Some(rest) = first_trimmed.strip_prefix('=') {
        return Some(Element::Synopsis {
            text: rest.trim().to_string(),
        });
    }
    if let Some(rest) = first_trimmed.strip_prefix('~') {
        return Some(Element::Lyric {
            text: rest.trim().to_string(),
        });
    }
    if let Some(rest) = first_trimmed.strip_prefix('>') {
        // Either centered text (`> text <`) or forced transition
        // (`> CUT TO:` or `> FADE IN`).
        let trimmed = rest.trim();
        if let Some(stripped) = trimmed.strip_suffix('<') {
            return Some(Element::Action {
                text: stripped.trim().to_string(),
                centered: true,
            });
        }
        return Some(Element::Transition(classify_transition(trimmed)));
    }

    // Heuristic classification.
    // Scene heading: starts with one of the IE prefixes, followed by '.' or ' '.
    if looks_like_scene_heading(first_trimmed) {
        return Some(parse_scene_heading(first_trimmed, &block[1..]));
    }
    // Transition: single line, ALL CAPS, ends in "TO:" or is "FADE OUT."
    if block.len() == 1 && looks_like_transition(first_trimmed) {
        return Some(Element::Transition(classify_transition(first_trimmed)));
    }
    // Dialogue: first line is ALL CAPS, no trailing colon-as-transition,
    // and there's at least one more line in the block. Character cues
    // commonly include a parenthetical extension like (V.O.).
    if block.len() >= 2 && looks_like_character_cue(first_trimmed) {
        return Some(parse_dialogue_block(first_trimmed, &block[1..]));
    }

    // Fallback: action.
    Some(Element::Action {
        text: join_lines(first_trimmed, &block[1..]),
        centered: false,
    })
}

fn join_lines(head: &str, rest: &[String]) -> String {
    let mut out = head.trim_end().to_string();
    for line in rest {
        out.push('\n');
        out.push_str(line.trim_end());
    }
    out
}

fn looks_like_scene_heading(line: &str) -> bool {
    let u = line.to_ascii_uppercase();
    const PREFIXES: &[&str] = &[
        "INT.", "EXT.", "EST.", "INT/EXT", "INT./EXT.", "I/E", "I./E.",
    ];
    PREFIXES.iter().any(|p| u.starts_with(*p))
}

fn parse_scene_heading(first: &str, rest_lines: &[String]) -> Element {
    let slugline = if rest_lines.is_empty() {
        first.trim().to_string()
    } else {
        join_lines(first, rest_lines).trim().to_string()
    };
    let (ie, location, time_of_day) = decompose_slugline(&slugline);
    Element::SceneHeading {
        slugline,
        ie,
        location,
        time_of_day,
    }
}

/// Best-effort breakdown of a slugline into `(ie, location, time_of_day)`.
fn decompose_slugline(slug: &str) -> (Option<InteriorExterior>, Option<String>, Option<String>) {
    let upper = slug.trim_start();
    let upper_u = upper.to_ascii_uppercase();
    let (ie, rest_start) = if let Some(r) = upper_u.strip_prefix("INT./EXT.") {
        (Some(InteriorExterior::IntExt), upper_u.len() - r.len())
    } else if let Some(r) = upper_u.strip_prefix("INT/EXT") {
        (Some(InteriorExterior::IntExt), upper_u.len() - r.len())
    } else if let Some(r) = upper_u.strip_prefix("I/E") {
        (Some(InteriorExterior::IntExt), upper_u.len() - r.len())
    } else if let Some(r) = upper_u.strip_prefix("INT.") {
        (Some(InteriorExterior::Int), upper_u.len() - r.len())
    } else if let Some(r) = upper_u.strip_prefix("EXT.") {
        (Some(InteriorExterior::Ext), upper_u.len() - r.len())
    } else if let Some(r) = upper_u.strip_prefix("EST.") {
        (Some(InteriorExterior::Est), upper_u.len() - r.len())
    } else {
        (None, 0)
    };

    let rest = upper[rest_start..].trim();
    // Time-of-day is conventionally after the last ` - `.
    if let Some(dash) = rest.rfind(" - ") {
        let location = rest[..dash].trim().to_string();
        let time = rest[dash + 3..].trim().to_string();
        return (
            ie,
            if location.is_empty() { None } else { Some(location) },
            if time.is_empty() { None } else { Some(time) },
        );
    }
    (
        ie,
        if rest.is_empty() { None } else { Some(rest.to_string()) },
        None,
    )
}

fn looks_like_transition(line: &str) -> bool {
    let t = line.trim();
    if !is_all_caps(t) {
        return false;
    }
    t.ends_with("TO:")
        || t == "FADE OUT."
        || t == "FADE TO BLACK."
        || t == "FADE IN:"
}

fn classify_transition(line: &str) -> Transition {
    let text = line.trim().to_string();
    let u = text.to_ascii_uppercase();
    let kind = if u == "FADE IN:" || u == "FADE IN." {
        TransitionKind::FadeIn
    } else if u == "FADE OUT." || u == "FADE TO BLACK." || u == "FADE OUT:" {
        TransitionKind::FadeOut
    } else if u.starts_with("FADE TO ") {
        TransitionKind::FadeTo
    } else if u.starts_with("CUT TO") {
        TransitionKind::Cut
    } else if u.starts_with("MATCH CUT") {
        TransitionKind::MatchCut
    } else if u.starts_with("SMASH CUT") {
        TransitionKind::SmashCut
    } else if u.starts_with("JUMP CUT") {
        TransitionKind::JumpCut
    } else if u.starts_with("DISSOLVE") {
        TransitionKind::Dissolve
    } else if u.starts_with("WHIP PAN") || u.starts_with("WHIP TO") {
        TransitionKind::WhipPan
    } else if u.starts_with("J-CUT") || u.starts_with("J CUT") {
        TransitionKind::JCut
    } else if u.starts_with("L-CUT") || u.starts_with("L CUT") {
        TransitionKind::LCut
    } else {
        TransitionKind::Other
    };
    Transition { text, kind }
}

fn looks_like_character_cue(line: &str) -> bool {
    let t = line.trim();
    if t.is_empty() {
        return false;
    }
    // Strip dual-dialogue caret.
    let t = t.trim_end_matches('^').trim();
    // Strip extension (e.g. " (V.O.)").
    let core = if let Some(open) = t.find('(') {
        t[..open].trim()
    } else {
        t
    };
    if core.is_empty() {
        return false;
    }
    // Has to be all-caps + may include digits / apostrophes / hyphens.
    is_all_caps(core) && core.chars().any(|c| c.is_ascii_alphabetic())
}

fn is_all_caps(s: &str) -> bool {
    let mut saw_letter = false;
    for c in s.chars() {
        if c.is_ascii_alphabetic() {
            if !c.is_ascii_uppercase() {
                return false;
            }
            saw_letter = true;
        }
    }
    saw_letter
}

fn parse_dialogue_block(cue_line: &str, rest_lines: &[String]) -> Element {
    let mut cue = cue_line.trim().to_string();
    let dual = cue.ends_with('^');
    if dual {
        cue.pop();
        cue = cue.trim().to_string();
    }
    let (character, extension) = split_cue_extension(&cue);

    let mut lines: Vec<DialogueLine> = Vec::new();
    for raw in rest_lines {
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        if let Some(rest) = line.strip_prefix('(').and_then(|s| s.strip_suffix(')')) {
            lines.push(DialogueLine::Parenthetical(rest.trim().to_string()));
        } else if let Some(rest) = line.strip_prefix('~') {
            lines.push(DialogueLine::Lyric(rest.trim().to_string()));
        } else {
            lines.push(DialogueLine::Text(line.to_string()));
        }
    }

    let ext_upper = extension.as_deref().map(|s| s.to_ascii_uppercase());
    let is_voiceover = ext_upper
        .as_deref()
        .map(|e| e.contains("V.O.") || e == "VOICEOVER" || e == "VO")
        .unwrap_or(false);
    let is_off_screen = ext_upper
        .as_deref()
        .map(|e| e.contains("O.S.") || e == "OS" || e == "OFFSCREEN" || e == "OFF SCREEN")
        .unwrap_or(false);

    Element::Dialogue {
        character,
        extension,
        dual,
        is_voiceover,
        is_off_screen,
        lines,
    }
}

fn split_cue_extension(cue: &str) -> (String, Option<String>) {
    let cue = cue.trim();
    if let Some(open) = cue.find('(') {
        if let Some(close) = cue.rfind(')') {
            if close > open {
                let character = cue[..open].trim().to_string();
                let ext = cue[open + 1..close].trim().to_string();
                return (character, if ext.is_empty() { None } else { Some(ext) });
            }
        }
    }
    (cue.to_string(), None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_input_errors() {
        assert!(matches!(parse(""), Err(ParseError::Empty)));
        assert!(matches!(parse("   \n\n"), Err(ParseError::Empty)));
    }

    #[test]
    fn title_page_basic() {
        let src = r#"Title: My Movie
Author: Shane

EXT. PARK - DAY

A bench.
"#;
        let s = parse(src).unwrap();
        let tp = s.title_page.unwrap();
        assert_eq!(tp.title.as_deref(), Some("My Movie"));
        assert_eq!(tp.author.as_deref(), Some("Shane"));
    }

    #[test]
    fn scene_heading_with_ie_and_time() {
        let src = "INT. KITCHEN - DAY\n\nA bowl of cereal.\n";
        let s = parse(src).unwrap();
        match &s.elements[0] {
            Element::SceneHeading {
                ie,
                location,
                time_of_day,
                ..
            } => {
                assert_eq!(*ie, Some(InteriorExterior::Int));
                assert_eq!(location.as_deref(), Some("KITCHEN"));
                assert_eq!(time_of_day.as_deref(), Some("DAY"));
            }
            other => panic!("expected SceneHeading, got {:?}", other),
        }
    }

    #[test]
    fn ext_and_int_ext_variants() {
        for (src, expect) in [
            ("EXT. ROAD - NIGHT", InteriorExterior::Ext),
            ("INT./EXT. CAR - DAY", InteriorExterior::IntExt),
            ("EST. CITY - DAWN", InteriorExterior::Est),
        ] {
            let s = parse(&format!("{src}\n\nAction.")).unwrap();
            match &s.elements[0] {
                Element::SceneHeading { ie, .. } => assert_eq!(*ie, Some(expect)),
                _ => panic!(),
            }
        }
    }

    #[test]
    fn dialogue_with_voiceover_extension() {
        let src = r#"INT. ROOM - DAY

NARRATOR (V.O.)
It was a dark and stormy night.
"#;
        let s = parse(src).unwrap();
        match &s.elements[1] {
            Element::Dialogue {
                character,
                extension,
                is_voiceover,
                lines,
                ..
            } => {
                assert_eq!(character, "NARRATOR");
                assert_eq!(extension.as_deref(), Some("V.O."));
                assert!(is_voiceover);
                assert_eq!(lines.len(), 1);
                match &lines[0] {
                    DialogueLine::Text(t) => assert!(t.contains("dark and stormy")),
                    _ => panic!(),
                }
            }
            other => panic!("expected Dialogue, got {:?}", other),
        }
    }

    #[test]
    fn dialogue_with_parenthetical() {
        let src = r#"EXT. PARK - DAY

JANE
(whispered)
Don't look back.
"#;
        let s = parse(src).unwrap();
        match &s.elements[1] {
            Element::Dialogue { lines, .. } => {
                assert_eq!(lines.len(), 2);
                match &lines[0] {
                    DialogueLine::Parenthetical(p) => assert_eq!(p, "whispered"),
                    _ => panic!(),
                }
                match &lines[1] {
                    DialogueLine::Text(t) => assert_eq!(t, "Don't look back."),
                    _ => panic!(),
                }
            }
            _ => panic!(),
        }
    }

    #[test]
    fn dual_dialogue_marker() {
        let src = "EXT. STREET - DAY\n\nALICE ^\nHi!\n";
        let s = parse(src).unwrap();
        match &s.elements[1] {
            Element::Dialogue { dual, .. } => assert!(*dual),
            _ => panic!(),
        }
    }

    #[test]
    fn transitions_classified() {
        for (text, expected) in [
            ("CUT TO:", TransitionKind::Cut),
            ("FADE IN:", TransitionKind::FadeIn),
            ("FADE OUT.", TransitionKind::FadeOut),
            ("DISSOLVE TO:", TransitionKind::Dissolve),
            ("MATCH CUT TO:", TransitionKind::MatchCut),
            ("SMASH CUT TO:", TransitionKind::SmashCut),
            ("WHIP PAN TO:", TransitionKind::WhipPan),
        ] {
            let src = format!("INT. ROOM - DAY\n\n{text}\n\nEXT. PARK - DAY\n\nAction.");
            let s = parse(&src).unwrap();
            let mut found = false;
            for e in &s.elements {
                if let Element::Transition(t) = e {
                    assert_eq!(t.kind, expected, "for input {text}");
                    found = true;
                }
            }
            assert!(found, "no transition found for {text}");
        }
    }

    #[test]
    fn forced_scene_heading_with_dot() {
        let src = ".A WEIRD HEADER\n\nAction.";
        let s = parse(src).unwrap();
        match &s.elements[0] {
            Element::SceneHeading { slugline, .. } => assert_eq!(slugline, "A WEIRD HEADER"),
            _ => panic!(),
        }
    }

    #[test]
    fn forced_action_with_bang() {
        let src = "!INT. NOT REALLY A SCENE\n\nMore.";
        let s = parse(src).unwrap();
        match &s.elements[0] {
            Element::Action { text, .. } => assert!(text.contains("NOT REALLY")),
            _ => panic!(),
        }
    }

    #[test]
    fn centered_text() {
        let src = "INT. ROOM - DAY\n\n> THE END <\n";
        let s = parse(src).unwrap();
        match &s.elements[1] {
            Element::Action { text, centered } => {
                assert!(*centered);
                assert_eq!(text, "THE END");
            }
            _ => panic!(),
        }
    }

    #[test]
    fn page_break() {
        let src = "INT. ROOM - DAY\n\n===\n\nEXT. PARK - DAY\n";
        let s = parse(src).unwrap();
        assert!(matches!(s.elements[1], Element::PageBreak));
    }

    #[test]
    fn sections_with_levels() {
        let src = "# Act One\n\n## Scene One\n\nINT. ROOM - DAY\n";
        let s = parse(src).unwrap();
        match &s.elements[0] {
            Element::Section { level, text } => {
                assert_eq!(*level, 1);
                assert_eq!(text, "Act One");
            }
            _ => panic!(),
        }
        match &s.elements[1] {
            Element::Section { level, text } => {
                assert_eq!(*level, 2);
                assert_eq!(text, "Scene One");
            }
            _ => panic!(),
        }
    }

    #[test]
    fn boneyard_is_stripped() {
        let src = "EXT. PARK - DAY\n\nAction one. /* not seen */ Action two.\n";
        let s = parse(src).unwrap();
        let txt = match &s.elements[1] {
            Element::Action { text, .. } => text.clone(),
            _ => panic!(),
        };
        assert!(!txt.contains("not seen"));
    }

    #[test]
    fn notes_are_stripped() {
        let src = "EXT. PARK - DAY\n\nA bench. [[ TODO: rewrite ]]\n";
        let s = parse(src).unwrap();
        let txt = match &s.elements[1] {
            Element::Action { text, .. } => text.clone(),
            _ => panic!(),
        };
        assert!(!txt.contains("TODO"));
    }
}
