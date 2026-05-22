//! Transition classifier — read a screenplay's transitions and a
//! velocity profile, emit richer transition specs ready to drop into a
//! storyboard.
//!
//! The PRD §5.1 maps Fountain transition cues onto the wavelet transition
//! vocabulary, with velocity-driven duration choices and direction hints
//! for whip-pans. This module bakes those rules into a pure function:
//! `classify_transitions(screenplay, velocity) → Vec<ClassifiedTransition>`.

use crate::velocity::VelocityProfile;
use fountain::{Element, Screenplay, Transition, TransitionKind};
use serde::{Deserialize, Serialize};

/// Top-level wrapper — JSON-friendly shape for the CLI.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransitionClassification {
    /// Screenplay path (for round-tripping).
    pub screenplay_ref: String,
    /// Velocity profile path (for round-tripping).
    pub velocity_ref: String,
    /// One entry per Fountain transition encountered (in source order).
    pub transitions: Vec<ClassifiedTransition>,
}

/// One classified transition.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClassifiedTransition {
    /// Index of the transition element in the screenplay's elements.
    pub element_index: usize,
    /// Source-side text (e.g. "CUT TO:", "FADE IN:").
    pub source_text: String,
    /// Classified kind (carried from the Fountain parse).
    pub kind: TransitionKind,
    /// Estimated time in seconds at which this transition fires —
    /// derived by walking the screenplay using the same per-element
    /// duration heuristic the velocity proposer uses.
    pub t_secs: f32,
    /// BPM at this transition's t (sampled from the velocity profile).
    pub bpm_at: f32,
    /// Proposed duration in seconds, or None for hard cuts.
    pub duration_secs: Option<f32>,
    /// Whip-pan direction hint, if applicable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub direction: Option<String>,
    /// J/L cut audio lead/trail in seconds, if applicable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audio_lead_secs: Option<f32>,
    /// Rationale string — short note explaining why this duration/
    /// direction was chosen. Useful when the agent reviews the report.
    pub rationale: String,
}

/// Classify every transition in a screenplay using the velocity profile
/// for duration choices.
pub fn classify_transitions(
    screenplay: &Screenplay,
    velocity: &VelocityProfile,
    screenplay_ref: impl Into<String>,
    velocity_ref: impl Into<String>,
) -> TransitionClassification {
    let mut out: Vec<ClassifiedTransition> = Vec::new();
    let mut t = 0.0f32;
    let mut prev_whip_dir = "whip-pan-left";

    for (i, element) in screenplay.elements.iter().enumerate() {
        match element {
            Element::Transition(transition) => {
                let bpm = velocity.bpm_at(t);
                let classified = classify_one(
                    i,
                    transition,
                    t,
                    bpm,
                    &mut prev_whip_dir,
                );
                t += classified.duration_secs.unwrap_or(0.0).max(0.5);
                out.push(classified);
            }
            // Mirror the velocity proposer's per-element timing
            // estimates so transition `t_secs` lines up with what the
            // velocity profile saw.
            Element::Action { text, .. } => {
                let words = text.split_whitespace().count();
                t += (words as f32 / 2.5).max(0.4);
            }
            Element::Dialogue { lines, .. } => {
                let words: usize = lines
                    .iter()
                    .map(|l| match l {
                        fountain::DialogueLine::Text(t) | fountain::DialogueLine::Lyric(t) => {
                            t.split_whitespace().count()
                        }
                        fountain::DialogueLine::Parenthetical(_) => 0,
                    })
                    .sum();
                t += (words as f32 / 2.5).max(1.2);
            }
            Element::SceneHeading { .. } => {
                t += 0.4;
            }
            Element::Lyric { .. } => {
                t += 1.5;
            }
            _ => {}
        }
    }

    TransitionClassification {
        screenplay_ref: screenplay_ref.into(),
        velocity_ref: velocity_ref.into(),
        transitions: out,
    }
}

fn classify_one(
    element_index: usize,
    transition: &Transition,
    t_secs: f32,
    bpm_at: f32,
    prev_whip_dir: &mut &'static str,
) -> ClassifiedTransition {
    let kind = transition.kind;
    let (duration_secs, direction, audio_lead_secs, rationale) = match kind {
        TransitionKind::Cut
        | TransitionKind::MatchCut
        | TransitionKind::SmashCut
        | TransitionKind::JumpCut
        | TransitionKind::Other => (
            None,
            None,
            None,
            format!("hard cut ({:?}) — no duration", kind),
        ),
        TransitionKind::FadeIn => (
            Some(if bpm_at < 70.0 { 1.2 } else { 0.6 }),
            None,
            None,
            format!(
                "fade-in length set from BPM ({}): longer fade at lower tempo",
                bpm_at as i32
            ),
        ),
        TransitionKind::FadeOut | TransitionKind::FadeTo => (
            Some(if bpm_at < 70.0 { 1.2 } else { 0.8 }),
            None,
            None,
            format!(
                "fade length set from BPM ({}): longer fade at lower tempo",
                bpm_at as i32
            ),
        ),
        TransitionKind::Dissolve => (
            Some((90.0 / bpm_at.max(40.0)).clamp(0.3, 1.5)),
            None,
            None,
            format!(
                "dissolve length inversely proportional to BPM ({})",
                bpm_at as i32
            ),
        ),
        TransitionKind::WhipPan => {
            // Alternate direction so successive whips feel intentional.
            let direction = *prev_whip_dir;
            *prev_whip_dir = if direction == "whip-pan-left" {
                "whip-pan-right"
            } else {
                "whip-pan-left"
            };
            (
                Some(0.25),
                Some(direction.to_string()),
                None,
                format!("whip-pan {} — alternates direction", direction),
            )
        }
        TransitionKind::JCut => (
            None,
            None,
            Some(0.3),
            "J-cut: next shot's audio leads by 0.3s".into(),
        ),
        TransitionKind::LCut => (
            None,
            None,
            Some(0.3),
            "L-cut: previous shot's audio trails by 0.3s".into(),
        ),
    };

    ClassifiedTransition {
        element_index,
        source_text: transition.text.clone(),
        kind,
        t_secs,
        bpm_at,
        duration_secs,
        direction,
        audio_lead_secs,
        rationale,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::velocity::{Anchor, VelocityProfile};
    use fountain::parse;

    fn flat_velocity(duration: f32, bpm: f32) -> VelocityProfile {
        VelocityProfile {
            duration_secs: duration,
            mean_bpm: bpm,
            anchors: vec![
                Anchor { t: 0.0, bpm, label: None },
                Anchor { t: duration, bpm, label: None },
            ],
        }
    }

    #[test]
    fn classifies_each_transition_kind() {
        let src = r#"EXT. PARK - DAY

A bench.

CUT TO:

INT. ROOM - DAY

She enters.

DISSOLVE TO:

INT. KITCHEN - DAY

A pie.

WHIP PAN TO:

EXT. STREET - DAY

A horn honks.

FADE OUT.
"#;
        let s = parse(src).unwrap();
        let v = flat_velocity(60.0, 90.0);
        let r = classify_transitions(&s, &v, "s.fountain", "v.json");
        let kinds: Vec<TransitionKind> = r.transitions.iter().map(|c| c.kind).collect();
        assert_eq!(
            kinds,
            vec![
                TransitionKind::Cut,
                TransitionKind::Dissolve,
                TransitionKind::WhipPan,
                TransitionKind::FadeOut,
            ]
        );
        assert!(r.transitions[0].duration_secs.is_none(), "cut has no duration");
        assert!(r.transitions[1].duration_secs.is_some(), "dissolve has duration");
        assert!(r.transitions[2].direction.is_some(), "whip has direction");
        assert!(r.transitions[3].duration_secs.is_some(), "fade has duration");
    }

    #[test]
    fn whip_pan_alternates_direction() {
        let src = r#"EXT. STREET - DAY

A horn.

WHIP PAN TO:

INT. ROOM - DAY

A whisper.

WHIP PAN TO:

EXT. STREET - DAY

A scream.
"#;
        let s = parse(src).unwrap();
        let v = flat_velocity(30.0, 130.0);
        let r = classify_transitions(&s, &v, "s.fountain", "v.json");
        let directions: Vec<Option<String>> =
            r.transitions.iter().map(|c| c.direction.clone()).collect();
        assert_eq!(
            directions,
            vec![
                Some("whip-pan-left".into()),
                Some("whip-pan-right".into()),
            ]
        );
    }

    #[test]
    fn dissolve_duration_scales_with_bpm() {
        let src = r#"EXT. PARK - DAY

A bench.

DISSOLVE TO:

INT. ROOM - DAY

She enters.
"#;
        let s = parse(src).unwrap();
        let slow = classify_transitions(&s, &flat_velocity(30.0, 60.0), "s", "v");
        let fast = classify_transitions(&s, &flat_velocity(30.0, 130.0), "s", "v");
        let slow_d = slow.transitions[0].duration_secs.unwrap();
        let fast_d = fast.transitions[0].duration_secs.unwrap();
        assert!(slow_d > fast_d, "expected slow {slow_d} > fast {fast_d}");
    }

    #[test]
    fn fade_at_low_bpm_is_longer() {
        let src = "EXT. PARK - DAY\n\nA bench.\n\nFADE OUT.\n";
        let s = parse(src).unwrap();
        let calm = classify_transitions(&s, &flat_velocity(10.0, 60.0), "s", "v");
        let busy = classify_transitions(&s, &flat_velocity(10.0, 120.0), "s", "v");
        let cd = calm.transitions[0].duration_secs.unwrap();
        let bd = busy.transitions[0].duration_secs.unwrap();
        assert!(cd > bd);
    }
}
