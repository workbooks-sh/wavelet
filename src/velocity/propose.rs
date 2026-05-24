//! Heuristic velocity-profile proposer.
//!
//! Walks a Fountain screenplay, estimates per-element screen time, and
//! emits a piecewise-linear BPM curve. No LLM; pure heuristic. The agent
//! is expected to redline the output — this is a starting draft.
//!
//! Heuristics (tunable constants documented inline):
//! - Action: 2.5 words/sec on-screen pacing.
//! - Dialogue: ~150 words/min = 2.5 wps; min line beat 1.2s.
//! - Scene heading: 0.4s (a beat for the slugline).
//! - Transition: 0.5s.
//!
//! BPM mapping per element:
//! - Action: 60 + 20 × (action_density - 1.0) clamped to [60, 140].
//!   Density = words per second relative to 2.5 wps baseline.
//! - Dialogue: 80 BPM base; +10 if VO/OS (more reflective), -5 if
//!   parenthetical-heavy.
//! - Transition: kick per kind (CUT +15, MATCH/SMASH +30, WHIP +35,
//!   DISSOLVE/FADE -10), then decay back to baseline over 1.5s.
//! - Scene heading: held at preceding BPM (no kick; sluglines are
//!   structural, not rhythmic).

use crate::velocity::{Anchor, VelocityProfile};
use fountain::{DialogueLine, Element, Screenplay, TransitionKind};

/// Words-per-second baseline for action paragraphs. Tuned from
/// industry-standard read-aloud rates.
const ACTION_WPS: f32 = 2.5;
/// Words-per-second baseline for dialogue (read aloud or subtitled).
const DIALOGUE_WPS: f32 = 2.5;
/// Floor duration in seconds for a dialogue beat — even one word lands
/// for at least this long.
const DIALOGUE_MIN_SECS: f32 = 1.2;
/// Hold duration for a slugline / scene heading.
const SCENE_HEADING_SECS: f32 = 0.4;
/// Hold duration for a transition cue.
const TRANSITION_SECS: f32 = 0.5;

/// Heuristic baseline BPM when nothing else applies.
const BASELINE_BPM: f32 = 80.0;
/// Lower clamp for proposed BPM.
const MIN_BPM: f32 = 55.0;
/// Upper clamp for proposed BPM.
const MAX_BPM: f32 = 160.0;

/// Walk a `Screenplay` and emit a `VelocityProfile`.
pub fn propose_from_screenplay(screenplay: &Screenplay) -> VelocityProfile {
    let mut anchors: Vec<Anchor> = Vec::new();
    let mut t = 0.0f32;
    let mut current_bpm = BASELINE_BPM;
    anchors.push(Anchor {
        t,
        bpm: current_bpm,
        label: Some("open".into()),
    });

    for element in &screenplay.elements {
        let (duration, target_bpm, label) = score_element(element, current_bpm);
        if duration <= 0.0 {
            continue;
        }
        // Hold the current BPM until the start of this element, then
        // ramp to the target across the element's duration.
        if (current_bpm - target_bpm).abs() > 0.5 {
            anchors.push(Anchor {
                t,
                bpm: current_bpm,
                label: None,
            });
            anchors.push(Anchor {
                t: t + duration,
                bpm: target_bpm,
                label: label.clone(),
            });
        } else {
            anchors.push(Anchor {
                t: t + duration,
                bpm: target_bpm,
                label: label.clone(),
            });
        }
        t += duration;
        current_bpm = target_bpm;
    }

    // Always anchor the final time so duration_secs == last anchor t.
    if anchors.last().map(|a| a.t).unwrap_or(0.0) < t {
        anchors.push(Anchor {
            t,
            bpm: current_bpm,
            label: Some("close".into()),
        });
    }

    let anchors = simplify_anchors(anchors);

    let mut profile = VelocityProfile {
        duration_secs: t.max(1.0),
        mean_bpm: 0.0,
        anchors,
    };
    profile.refresh_mean_bpm();
    profile
}

fn score_element(element: &Element, current_bpm: f32) -> (f32, f32, Option<String>) {
    match element {
        Element::Action { text, .. } => {
            let words = word_count(text);
            let duration = (words as f32 / ACTION_WPS).max(0.4);
            let density = words as f32 / duration.max(0.1);
            // 60 + 20 × (density - ACTION_WPS) maps a 2.5-wps paragraph
            // to 60 BPM and dense action (5 wps) to ~110 BPM.
            let bpm = (60.0 + 20.0 * (density - ACTION_WPS)).clamp(MIN_BPM, MAX_BPM);
            (duration, bpm, Some("action".into()))
        }
        Element::Dialogue {
            lines,
            is_voiceover,
            is_off_screen,
            ..
        } => {
            let total_words: usize = lines
                .iter()
                .map(|l| match l {
                    DialogueLine::Text(t) | DialogueLine::Lyric(t) => word_count(t),
                    DialogueLine::Parenthetical(_) => 0,
                })
                .sum();
            let paren_count = lines
                .iter()
                .filter(|l| matches!(l, DialogueLine::Parenthetical(_)))
                .count();
            let duration = (total_words as f32 / DIALOGUE_WPS).max(DIALOGUE_MIN_SECS);
            let mut bpm = 80.0;
            if *is_voiceover || *is_off_screen {
                bpm += 10.0;
            }
            // Parentheticals slow delivery — pull BPM down a bit.
            bpm -= (paren_count as f32) * 3.0;
            let bpm = bpm.clamp(MIN_BPM, MAX_BPM);
            let label = if *is_voiceover {
                "dialogue-vo"
            } else if *is_off_screen {
                "dialogue-os"
            } else {
                "dialogue"
            };
            (duration, bpm, Some(label.into()))
        }
        Element::Transition(t) => {
            let kick = match t.kind {
                TransitionKind::Cut => 15.0,
                TransitionKind::JumpCut => 20.0,
                TransitionKind::MatchCut | TransitionKind::SmashCut => 30.0,
                TransitionKind::WhipPan => 35.0,
                TransitionKind::Dissolve
                | TransitionKind::FadeIn
                | TransitionKind::FadeOut
                | TransitionKind::FadeTo => -10.0,
                TransitionKind::JCut | TransitionKind::LCut => 5.0,
                TransitionKind::Other => 0.0,
            };
            let bpm = (current_bpm + kick).clamp(MIN_BPM, MAX_BPM);
            let label = format!("transition-{:?}", t.kind).to_ascii_lowercase();
            (TRANSITION_SECS, bpm, Some(label))
        }
        Element::SceneHeading { .. } => (SCENE_HEADING_SECS, current_bpm, None),
        Element::PageBreak => (0.0, current_bpm, None),
        Element::Section { .. } => (0.0, current_bpm, None),
        Element::Synopsis { .. } => (0.0, current_bpm, None),
        Element::Lyric { .. } => (1.5, current_bpm, Some("lyric".into())),
    }
}

fn word_count(text: &str) -> usize {
    text.split_whitespace().filter(|w| !w.is_empty()).count()
}

/// Collapse runs of near-identical BPM into a single anchor pair. Keeps
/// the curve readable when adjacent elements happen to score the same.
fn simplify_anchors(anchors: Vec<Anchor>) -> Vec<Anchor> {
    if anchors.len() <= 2 {
        return anchors;
    }
    let mut out: Vec<Anchor> = Vec::with_capacity(anchors.len());
    for a in anchors {
        if out.len() >= 2 {
            let prev = &out[out.len() - 1];
            let prev2 = &out[out.len() - 2];
            // If three consecutive anchors share a BPM (within 0.5),
            // collapse the middle one.
            if (prev.bpm - prev2.bpm).abs() < 0.5 && (a.bpm - prev.bpm).abs() < 0.5 {
                out.pop();
            }
        }
        out.push(a);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use fountain::parse;

    #[test]
    fn empty_screenplay_proposes_baseline() {
        let screenplay = fountain::Screenplay::default();
        let v = propose_from_screenplay(&screenplay);
        assert!(!v.anchors.is_empty());
        assert!(v.anchors[0].bpm > 0.0);
    }

    #[test]
    fn action_heavy_lifts_bpm() {
        let src = r#"EXT. STREET - DAY

The chase. Cars screech. Glass shatters. Bodies fly. Sirens wail in the distance as police converge on the scene.
"#;
        let s = parse(src).unwrap();
        let v = propose_from_screenplay(&s);
        // Mean BPM should be above baseline 80.
        assert!(v.mean_bpm >= 60.0, "got {}", v.mean_bpm);
    }

    #[test]
    fn smash_cut_kicks_bpm() {
        let calm = r#"INT. ROOM - DAY

She sits.
"#;
        let smashy = r#"INT. ROOM - DAY

She sits.

SMASH CUT TO:

EXT. STREET - DAY

Chaos.
"#;
        let v_calm = propose_from_screenplay(&parse(calm).unwrap());
        let v_smashy = propose_from_screenplay(&parse(smashy).unwrap());
        assert!(
            v_smashy.anchors.iter().map(|a| a.bpm).fold(0.0_f32, f32::max)
                > v_calm.anchors.iter().map(|a| a.bpm).fold(0.0_f32, f32::max),
            "smash-cut profile should peak higher than calm profile"
        );
    }

    #[test]
    fn duration_secs_matches_last_anchor() {
        let src = r#"EXT. PARK - DAY

A bench. A pigeon lands.

NARRATOR (V.O.)
Once upon a time.
"#;
        let v = propose_from_screenplay(&parse(src).unwrap());
        let last_t = v.anchors.iter().map(|a| a.t).fold(0.0_f32, f32::max);
        assert!((v.duration_secs - last_t).abs() < 0.01);
    }

    #[test]
    fn anchors_are_monotonic_in_time() {
        let src = include_str!("../../crates/fountain/tests/fixtures/big_fish_excerpt.fountain");
        let s = parse(src).unwrap();
        let v = propose_from_screenplay(&s);
        for w in v.anchors.windows(2) {
            assert!(w[0].t <= w[1].t, "non-monotonic at {:?} → {:?}", w[0], w[1]);
        }
    }
}
