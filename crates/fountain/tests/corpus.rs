//! Integration tests against a public-style Fountain excerpt (Big Fish
//! by John August, the spec's canonical example). The full screenplay
//! isn't checked in for licensing reasons — we use a small public-style
//! excerpt that exercises every Fountain construct we care about.

use fountain::{parse, DialogueLine, Element, InteriorExterior, TransitionKind};

const FIXTURE: &str = include_str!("fixtures/big_fish_excerpt.fountain");

#[test]
fn title_page_extracted() {
    let s = parse(FIXTURE).unwrap();
    let tp = s.title_page.expect("title page");
    assert_eq!(tp.title.as_deref(), Some("Big Fish"));
    assert_eq!(tp.credit.as_deref(), Some("written by"));
    assert_eq!(tp.author.as_deref(), Some("John August"));
    assert_eq!(tp.draft_date.as_deref(), Some("3/12/04"));
    assert!(tp.source.as_deref().unwrap().contains("Daniel Wallace"));
}

#[test]
fn body_element_sequence() {
    let s = parse(FIXTURE).unwrap();
    let kinds: Vec<&str> = s
        .elements
        .iter()
        .map(|e| match e {
            Element::SceneHeading { .. } => "scene",
            Element::Action { .. } => "action",
            Element::Dialogue { .. } => "dialogue",
            Element::Transition(_) => "transition",
            Element::PageBreak => "page_break",
            Element::Section { .. } => "section",
            Element::Synopsis { .. } => "synopsis",
            Element::Lyric { .. } => "lyric",
        })
        .collect();
    // Three scenes, mixed action / dialogue / transitions, ends with a
    // centered "THE END" action + a FADE OUT.
    assert!(
        kinds.iter().filter(|k| **k == "scene").count() >= 3,
        "expected at least 3 scenes, got {kinds:?}"
    );
    assert!(
        kinds.iter().filter(|k| **k == "transition").count() >= 3,
        "expected at least 3 transitions, got {kinds:?}"
    );
    assert!(
        kinds.iter().any(|k| *k == "dialogue"),
        "expected at least one dialogue, got {kinds:?}"
    );
}

#[test]
fn voiceover_extension_detected() {
    let s = parse(FIXTURE).unwrap();
    let has_vo = s.elements.iter().any(|e| matches!(
        e,
        Element::Dialogue {
            character,
            is_voiceover: true,
            ..
        } if character == "WILL"
    ));
    assert!(has_vo, "WILL (V.O.) wasn't recognized");
}

#[test]
fn dual_dialogue_marker_round_trips() {
    let s = parse(FIXTURE).unwrap();
    let has_dual = s.elements.iter().any(|e| matches!(
        e,
        Element::Dialogue { character, dual: true, .. } if character == "JENNY"
    ));
    assert!(has_dual, "JENNY ^ should mark dual-dialogue");
}

#[test]
fn parenthetical_extracted() {
    let s = parse(FIXTURE).unwrap();
    let saw_paren = s.elements.iter().any(|e| {
        if let Element::Dialogue { lines, .. } = e {
            lines
                .iter()
                .any(|l| matches!(l, DialogueLine::Parenthetical(p) if p == "softly"))
        } else {
            false
        }
    });
    assert!(saw_paren, "(softly) parenthetical missing");
}

#[test]
fn scene_headings_have_ie_and_time() {
    let s = parse(FIXTURE).unwrap();
    let int_count = s.elements.iter().filter(|e| matches!(
        e,
        Element::SceneHeading { ie: Some(InteriorExterior::Int), .. }
    )).count();
    let ext_count = s.elements.iter().filter(|e| matches!(
        e,
        Element::SceneHeading { ie: Some(InteriorExterior::Ext), .. }
    )).count();
    assert!(int_count >= 2, "expected ≥2 INT scenes");
    assert_eq!(ext_count, 1, "expected exactly 1 EXT scene");
}

#[test]
fn fade_out_classified() {
    let s = parse(FIXTURE).unwrap();
    let last_transition_kind = s
        .elements
        .iter()
        .rev()
        .find_map(|e| match e {
            Element::Transition(t) => Some(t.kind),
            _ => None,
        })
        .expect("at least one transition");
    assert_eq!(last_transition_kind, TransitionKind::FadeOut);
}

#[test]
fn smash_cut_classified() {
    let s = parse(FIXTURE).unwrap();
    let has_smash = s.elements.iter().any(|e| {
        if let Element::Transition(t) = e {
            t.kind == TransitionKind::SmashCut
        } else {
            false
        }
    });
    assert!(has_smash, "SMASH CUT TO: should be classified as SmashCut");
}

#[test]
fn centered_text_preserved() {
    let s = parse(FIXTURE).unwrap();
    let has_centered = s.elements.iter().any(|e| matches!(
        e,
        Element::Action { centered: true, text } if text.contains("THE END")
    ));
    assert!(has_centered, "centered '> THE END <' missing");
}

#[test]
fn json_round_trip_stable() {
    let s = parse(FIXTURE).unwrap();
    let json = serde_json::to_string(&s).expect("serialize");
    let back: fountain::Screenplay = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(back.elements.len(), s.elements.len());
}
