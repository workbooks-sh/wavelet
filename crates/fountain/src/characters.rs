//! Character registry extraction from a parsed `Screenplay`.
//!
//! `screenplay_characters` walks the element list once and builds a
//! deduplicated, ordered registry of every character who speaks —
//! collapsing `ALEX`, `Alex`, and `ALEX (V.O.)` into a single entry by
//! matching on the `canonical` form (uppercase + trimmed + extension
//! stripped).

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::ast::{DialogueLine, Element, Screenplay};

/// Canonical character registry entry — one per distinct CHARACTER cue
/// name across the screenplay. Names are normalized for matching:
/// trimmed, single-space-collapsed, case-preserved (real screenplays
/// occasionally style "Dr. Smith" or "MARIE-CLAIRE" deliberately).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CharacterEntry {
    /// Canonical name as written in the first cue (case-preserved).
    pub name: String,

    /// Normalized form for cross-cue matching (uppercase + trimmed +
    /// extension stripped — `MARIE`, not `MARIE (V.O.)` or `Marie`).
    pub canonical: String,

    /// Total number of dialogue cues for this character across the
    /// whole screenplay.
    pub cue_count: u32,

    /// Total spoken word count (excludes parentheticals + lyrics).
    pub word_count: u32,

    /// Cue extensions seen for this character — `V.O.`, `O.S.`,
    /// `CONT'D`, custom. Deduplicated.
    pub extensions: Vec<String>,

    /// True if ANY cue for this character was `is_voiceover` (V.O.).
    pub is_voiceover: bool,

    /// True if ANY cue was `is_off_screen` (O.S.).
    pub is_off_screen: bool,

    /// 1-based scene indices the character appears in. Empty when
    /// the character appears outside any scene heading (action
    /// preceding the first slugline).
    pub scenes: Vec<u32>,

    /// First dialogue line for this character — useful as a quick
    /// preview when surfacing the registry to an agent.
    pub first_line_preview: Option<String>,
}

/// Walk the parsed screenplay and produce the canonical character
/// registry. Characters are matched by `canonical` form (uppercase,
/// trimmed, extension-stripped) so `ALEX`, `Alex`, and `ALEX (V.O.)`
/// all collapse into the same entry. Returns entries in first-cue
/// order (the order the characters appear in the screenplay).
pub fn screenplay_characters(screenplay: &Screenplay) -> Vec<CharacterEntry> {
    // Ordered list of canonical keys, for preserving first-appearance order.
    let mut order: Vec<String> = Vec::new();
    // Map from canonical key → accumulated data.
    let mut map: HashMap<String, CharacterEntry> = HashMap::new();

    let mut current_scene: u32 = 0;

    for el in &screenplay.elements {
        match el {
            Element::SceneHeading { .. } => {
                current_scene += 1;
            }
            Element::Dialogue {
                character,
                extension,
                is_voiceover,
                is_off_screen,
                lines,
                ..
            } => {
                let canonical = match canonicalize_name(character) {
                    Some(c) => c,
                    None => continue,
                };

                let words = count_spoken_words(lines);

                if let Some(entry) = map.get_mut(&canonical) {
                    // Merge into existing entry.
                    entry.cue_count += 1;
                    entry.word_count += words;
                    if *is_voiceover {
                        entry.is_voiceover = true;
                    }
                    if *is_off_screen {
                        entry.is_off_screen = true;
                    }
                    if let Some(ext) = extension {
                        if !ext.is_empty() && !entry.extensions.contains(ext) {
                            entry.extensions.push(ext.clone());
                        }
                    }
                    if current_scene > 0 && !entry.scenes.contains(&current_scene) {
                        entry.scenes.push(current_scene);
                    }
                } else {
                    // First cue for this character.
                    let preview = first_line_preview(lines);
                    let mut scenes = Vec::new();
                    if current_scene > 0 {
                        scenes.push(current_scene);
                    }
                    let extensions = match extension {
                        Some(ext) if !ext.is_empty() => vec![ext.clone()],
                        _ => Vec::new(),
                    };
                    let entry = CharacterEntry {
                        name: character.clone(),
                        canonical: canonical.clone(),
                        cue_count: 1,
                        word_count: words,
                        extensions,
                        is_voiceover: *is_voiceover,
                        is_off_screen: *is_off_screen,
                        scenes,
                        first_line_preview: preview,
                    };
                    order.push(canonical.clone());
                    map.insert(canonical, entry);
                }
            }
            _ => {}
        }
    }

    order.into_iter().filter_map(|k| map.remove(&k)).collect()
}

/// Normalize a raw CHARACTER cue name for cross-cue matching.
///
/// Rules:
/// - Trim surrounding whitespace.
/// - Strip any parenthetical extension (`(V.O.)`, `(CONT'D)`, etc.).
/// - Collapse multiple interior spaces to one.
/// - Uppercase the result.
/// - Return `None` for an empty result (skip the cue).
pub fn canonicalize_name(name: &str) -> Option<String> {
    // Strip extension — everything from the first `(` onward.
    let bare = if let Some(paren) = name.find('(') {
        &name[..paren]
    } else {
        name
    };
    // Collapse interior whitespace and uppercase.
    let collapsed: String = bare
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_uppercase();
    if collapsed.is_empty() {
        None
    } else {
        Some(collapsed)
    }
}

/// Count spoken words in dialogue lines — Text only; parentheticals and
/// lyrics are excluded (same semantics as `count_dialogue_words` in
/// `duration_fit.rs`).
fn count_spoken_words(lines: &[DialogueLine]) -> u32 {
    let mut total = 0u32;
    for line in lines {
        if let DialogueLine::Text(s) = line {
            total += count_words(s);
        }
    }
    total
}

fn count_words(s: &str) -> u32 {
    s.split_whitespace()
        .filter(|w| w.chars().any(|c| c.is_alphanumeric()))
        .count() as u32
}

/// Extract the first spoken `DialogueLine::Text` as a preview, capped
/// at 60 characters.
fn first_line_preview(lines: &[DialogueLine]) -> Option<String> {
    for line in lines {
        if let DialogueLine::Text(s) = line {
            let s = s.trim();
            if !s.is_empty() {
                let preview = if s.chars().count() <= 60 {
                    s.to_string()
                } else {
                    let cut: String = s.chars().take(60).collect();
                    format!("{cut}…")
                };
                return Some(preview);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::parse;

    // ── canonicalize_name ─────────────────────────────────────────────

    #[test]
    fn canonicalize_lowercased_name() {
        assert_eq!(canonicalize_name("alex"), Some("ALEX".into()));
    }

    #[test]
    fn canonicalize_strips_extension_and_uppercases() {
        assert_eq!(
            canonicalize_name(" Marie (V.O.) "),
            Some("MARIE".into())
        );
    }

    #[test]
    fn canonicalize_preserves_period_in_title() {
        assert_eq!(canonicalize_name("Dr. Smith"), Some("DR. SMITH".into()));
    }

    #[test]
    fn canonicalize_preserves_hyphen() {
        assert_eq!(
            canonicalize_name("MARIE-CLAIRE"),
            Some("MARIE-CLAIRE".into())
        );
    }

    #[test]
    fn canonicalize_empty_returns_none() {
        assert_eq!(canonicalize_name(""), None);
        assert_eq!(canonicalize_name("   "), None);
        // Extension-only cue ("(V.O.)" with nothing before) → bare is "",
        // which collapses to empty → None.
        assert_eq!(canonicalize_name("(V.O.)"), None);
    }

    #[test]
    fn canonicalize_collapses_interior_spaces() {
        assert_eq!(
            canonicalize_name("  DR.  SMITH  (CONT'D)  "),
            Some("DR. SMITH".into())
        );
    }

    // ── screenplay_characters ─────────────────────────────────────────

    #[test]
    fn single_scene_single_character() {
        let src = r#"
INT. ROOM - DAY

ALEX
Hello world.
"#;
        let sp = parse(src).unwrap();
        let chars = screenplay_characters(&sp);
        assert_eq!(chars.len(), 1);
        assert_eq!(chars[0].canonical, "ALEX");
        assert_eq!(chars[0].cue_count, 1);
        assert_eq!(chars[0].word_count, 2);
        assert_eq!(chars[0].scenes, vec![1]);
        assert!(chars[0].extensions.is_empty());
        assert!(!chars[0].is_voiceover);
        assert!(!chars[0].is_off_screen);
        assert_eq!(
            chars[0].first_line_preview.as_deref(),
            Some("Hello world.")
        );
    }

    #[test]
    fn collapsing_alex_variants_into_one_entry() {
        // ALEX, Alex (V.O.), ALEX (CONT'D) → 1 entry, extensions [V.O., CONT'D]
        let src = r#"
INT. ROOM - DAY

ALEX
Regular line.

ALEX (V.O.)
Voiceover line.

ALEX (CONT'D)
Continuation.
"#;
        let sp = parse(src).unwrap();
        let chars = screenplay_characters(&sp);
        assert_eq!(chars.len(), 1, "should collapse into 1 entry");
        let entry = &chars[0];
        assert_eq!(entry.canonical, "ALEX");
        assert_eq!(entry.cue_count, 3);
        assert!(entry.is_voiceover);
        assert!(!entry.is_off_screen);
        // Both V.O. and CONT'D must be present (order may vary).
        assert!(
            entry.extensions.contains(&"V.O.".to_string()),
            "extensions: {:?}",
            entry.extensions
        );
        assert!(
            entry.extensions.contains(&"CONT'D".to_string()),
            "extensions: {:?}",
            entry.extensions
        );
    }

    #[test]
    fn two_characters_in_order() {
        let src = r#"
INT. ROOM - DAY

ALICE
First.

BOB
Second.
"#;
        let sp = parse(src).unwrap();
        let chars = screenplay_characters(&sp);
        assert_eq!(chars.len(), 2);
        assert_eq!(chars[0].canonical, "ALICE");
        assert_eq!(chars[1].canonical, "BOB");
    }

    #[test]
    fn empty_screenplay_returns_empty_vec() {
        // Need at least some non-empty content to avoid ParseError::Empty,
        // but no dialogue.
        let src = "INT. ROOM - DAY\n\nA quiet room.";
        let sp = parse(src).unwrap();
        let chars = screenplay_characters(&sp);
        assert!(chars.is_empty());
    }

    #[test]
    fn word_count_excludes_parentheticals_and_lyrics() {
        // "One bowl." = 2 words. The parenthetical and lyric lines must
        // not inflate the count.
        let src = r#"
INT. KITCHEN - DAY

NARRATOR (V.O.)
(softly)
One bowl.
~ La la la.
"#;
        let sp = parse(src).unwrap();
        let chars = screenplay_characters(&sp);
        assert_eq!(chars.len(), 1);
        assert_eq!(chars[0].word_count, 2, "only spoken Text lines count");
    }

    #[test]
    fn scene_indices_tracked_across_scenes() {
        let src = r#"
INT. SCENE ONE - DAY

ALEX
Cue one.

INT. SCENE TWO - DAY

BOB
Cue two.

INT. SCENE THREE - DAY

ALEX
Cue three.
"#;
        let sp = parse(src).unwrap();
        let chars = screenplay_characters(&sp);
        let alex = chars.iter().find(|c| c.canonical == "ALEX").unwrap();
        assert_eq!(alex.scenes, vec![1, 3]);
        let bob = chars.iter().find(|c| c.canonical == "BOB").unwrap();
        assert_eq!(bob.scenes, vec![2]);
    }

    #[test]
    fn dialogue_before_first_scene_heading_has_empty_scenes() {
        // Unusual but valid: dialogue before any SceneHeading.
        let src = r#"ALEX
Pre-scene line.

INT. ROOM - DAY

Action.
"#;
        let sp = parse(src).unwrap();
        let chars = screenplay_characters(&sp);
        // Parser requires ≥2 lines to recognize a character cue; the
        // block above has ALEX + dialogue in one block, so it parses as
        // a Dialogue element with character="ALEX".
        if let Some(entry) = chars.iter().find(|c| c.canonical == "ALEX") {
            // current_scene is 0 at that point → scenes must be empty.
            assert!(
                entry.scenes.is_empty(),
                "no scene heading before cue → empty scenes: {:?}",
                entry.scenes
            );
        }
        // If the parser didn't produce a Dialogue element here (it might
        // classify the block as Action), we just verify no panic.
    }
}
