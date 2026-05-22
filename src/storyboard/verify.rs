//! Structural verification of a `Storyboard`.
//!
//! This is the cheap, no-render lint pass that runs before any
//! generation. Catches inconsistencies that would otherwise compound
//! through Phase 5+ — bad shot ranges, dangling scene refs, 180°
//! violations, motion-vector discontinuities, missing assets.
//!
//! Deep verification (rendering each shot's mid-frame and running the
//! `expected_checks`) is intentionally separate — that pass lives
//! downstream once the shot assets exist.

use crate::storyboard::{
    ActionLine, CameraSide, ExpectedCheck, ShotTransition, Storyboard,
};
use fountain::TransitionKind;
use serde::{Deserialize, Serialize};

/// One structural finding from `verify_storyboard`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoryboardFinding {
    /// Severity.
    pub level: StoryboardLevel,
    /// Subject the finding pertains to.
    pub origin: String,
    /// Human-readable description.
    pub message: String,
}

/// Severity level for a finding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StoryboardLevel {
    /// Blocks downstream generation.
    Error,
    /// Advisory; downstream can proceed but the user should look.
    Warning,
}

/// Walk a storyboard and return every structural finding. Empty when
/// the storyboard is clean.
pub fn verify_storyboard(sb: &Storyboard) -> Vec<StoryboardFinding> {
    let mut findings: Vec<StoryboardFinding> = Vec::new();

    if sb.version != 1 {
        findings.push(err(
            "storyboard",
            format!("unsupported version {}, expected 1", sb.version),
        ));
    }
    if sb.duration_secs <= 0.0 {
        findings.push(err(
            "storyboard",
            format!("duration_secs must be > 0, got {}", sb.duration_secs),
        ));
    }
    if sb.fps == 0 {
        findings.push(err("storyboard", "fps must be > 0".to_string()));
    }
    if sb.resolution[0] == 0 || sb.resolution[1] == 0 {
        findings.push(err(
            "storyboard",
            format!("resolution must be positive, got {:?}", sb.resolution),
        ));
    }

    check_scene_invariants(sb, &mut findings);
    check_shot_ordering_and_coverage(sb, &mut findings);
    check_shot_scene_consistency(sb, &mut findings);
    check_transitions(sb, &mut findings);
    check_camera_side_invariant(sb, &mut findings);
    check_motion_continuity(sb, &mut findings);
    check_generation_manifests(sb, &mut findings);
    check_expected_check_targets(sb, &mut findings);

    findings
}

fn err(origin: impl Into<String>, message: impl Into<String>) -> StoryboardFinding {
    StoryboardFinding {
        level: StoryboardLevel::Error,
        origin: origin.into(),
        message: message.into(),
    }
}

fn warn(origin: impl Into<String>, message: impl Into<String>) -> StoryboardFinding {
    StoryboardFinding {
        level: StoryboardLevel::Warning,
        origin: origin.into(),
        message: message.into(),
    }
}

fn check_scene_invariants(sb: &Storyboard, findings: &mut Vec<StoryboardFinding>) {
    for (i, scene) in sb.scenes.iter().enumerate() {
        if scene.scene_index != i {
            findings.push(err(
                format!("scenes[{i}]"),
                format!(
                    "scene_index {} doesn't match position {i}",
                    scene.scene_index
                ),
            ));
        }
        let first = scene.first_shot;
        let end = first + scene.shot_count;
        if end > sb.shots.len() {
            findings.push(err(
                format!("scenes[{i}]"),
                format!(
                    "first_shot+shot_count={end} but storyboard has only {} shots",
                    sb.shots.len()
                ),
            ));
        }
        if scene.shot_count == 0 {
            findings.push(warn(
                format!("scenes[{i}]"),
                "scene has zero shots — likely unintentional".to_string(),
            ));
        }
        if let Some(al) = &scene.action_line {
            check_action_line(al, &format!("scenes[{i}].action_line"), findings);
        }
    }
}

fn check_action_line(al: &ActionLine, origin: &str, findings: &mut Vec<StoryboardFinding>) {
    for (name, p) in [("from", &al.from), ("to", &al.to)] {
        if !(0.0..=1.0).contains(&p[0]) || !(0.0..=1.0).contains(&p[1]) {
            findings.push(err(
                origin,
                format!("{name} {:?} is outside [0,1]², action lines use normalized coords", p),
            ));
        }
    }
    let dx = al.to[0] - al.from[0];
    let dy = al.to[1] - al.from[1];
    if (dx * dx + dy * dy).sqrt() < 0.05 {
        findings.push(warn(
            origin,
            "action line is degenerate (from ≈ to) — 180° checks won't work".to_string(),
        ));
    }
}

fn check_shot_ordering_and_coverage(sb: &Storyboard, findings: &mut Vec<StoryboardFinding>) {
    let mut covered_until = 0.0f32;
    for (i, shot) in sb.shots.iter().enumerate() {
        if shot.shot_index != i {
            findings.push(err(
                &shot.id,
                format!(
                    "shot_index {} doesn't match position {i}",
                    shot.shot_index
                ),
            ));
        }
        if shot.duration_secs <= 0.0 {
            findings.push(err(
                &shot.id,
                format!("duration_secs must be > 0, got {}", shot.duration_secs),
            ));
        }
        if shot.start_secs < covered_until - 0.001 {
            findings.push(err(
                &shot.id,
                format!(
                    "starts at {:.3}s but previous shot ended at {:.3}s — overlap",
                    shot.start_secs, covered_until
                ),
            ));
        } else if shot.start_secs > covered_until + 0.001 {
            findings.push(warn(
                &shot.id,
                format!(
                    "starts at {:.3}s but previous shot ended at {:.3}s — gap",
                    shot.start_secs, covered_until
                ),
            ));
        }
        covered_until = shot.start_secs + shot.duration_secs;

        if covered_until > sb.duration_secs + 0.001 {
            findings.push(err(
                &shot.id,
                format!(
                    "ends at {:.3}s but storyboard duration is {:.3}s",
                    covered_until, sb.duration_secs
                ),
            ));
        }
    }
}

fn check_shot_scene_consistency(sb: &Storyboard, findings: &mut Vec<StoryboardFinding>) {
    for shot in &sb.shots {
        if shot.scene_index >= sb.scenes.len() {
            findings.push(err(
                &shot.id,
                format!(
                    "scene_index {} out of range (have {} scenes)",
                    shot.scene_index,
                    sb.scenes.len()
                ),
            ));
        }
    }
}

fn check_transitions(sb: &Storyboard, findings: &mut Vec<StoryboardFinding>) {
    if let Some(first) = sb.shots.first() {
        if first.transition_in.is_some() {
            findings.push(warn(
                &first.id,
                "first shot has transition_in — typically the open is a hard cut".to_string(),
            ));
        }
    }
    for shot in &sb.shots {
        if let Some(t) = &shot.transition_in {
            validate_transition(&shot.id, t, findings);
        }
    }
}

fn validate_transition(
    origin: &str,
    t: &ShotTransition,
    findings: &mut Vec<StoryboardFinding>,
) {
    match (t.kind, t.duration_secs) {
        (TransitionKind::Cut, Some(_))
        | (TransitionKind::MatchCut, Some(_))
        | (TransitionKind::SmashCut, Some(_))
        | (TransitionKind::JumpCut, Some(_)) => {
            findings.push(warn(
                origin,
                format!("{:?} carries duration_secs but is a hard cut", t.kind),
            ));
        }
        (TransitionKind::FadeIn, None)
        | (TransitionKind::FadeOut, None)
        | (TransitionKind::FadeTo, None)
        | (TransitionKind::Dissolve, None)
        | (TransitionKind::WhipPan, None) => {
            findings.push(err(
                origin,
                format!("{:?} requires duration_secs but none was supplied", t.kind),
            ));
        }
        _ => {}
    }
    if matches!(t.kind, TransitionKind::WhipPan) && t.direction.is_none() {
        findings.push(warn(origin, "whip-pan with no direction hint".to_string()));
    }
    if let Some(d) = t.duration_secs {
        if d <= 0.0 {
            findings.push(err(origin, format!("transition duration {d} must be > 0")));
        }
        if d > 3.0 {
            findings.push(warn(
                origin,
                format!("transition duration {d}s is unusually long"),
            ));
        }
    }
}

fn check_camera_side_invariant(sb: &Storyboard, findings: &mut Vec<StoryboardFinding>) {
    // For each scene that has an action_line, every shot in the scene
    // must be on one of its two sides (`Left` / `Right`) or `Center`.
    // We don't yet have the geometry to verify side correctness against
    // the action line; we just check that the shots aren't all `Center`
    // when the scene has multiple speakers — that's almost certainly a
    // missed framing decision.
    for scene in &sb.scenes {
        if scene.action_line.is_none() {
            continue;
        }
        let range_end = (scene.first_shot + scene.shot_count).min(sb.shots.len());
        let range = &sb.shots[scene.first_shot..range_end];
        let center_count = range.iter().filter(|s| s.camera_side == CameraSide::Center).count();
        if !range.is_empty() && center_count == range.len() {
            findings.push(warn(
                format!("scenes[{}]", scene.scene_index),
                "scene has an action_line but every shot is camera_side=center"
                    .to_string(),
            ));
        }
    }
}

fn check_motion_continuity(sb: &Storyboard, findings: &mut Vec<StoryboardFinding>) {
    for w in sb.shots.windows(2) {
        let prev = &w[0];
        let next = &w[1];
        // If transition is a hard cut and both shots declare motion
        // vectors, they should be within ±30° of each other unless the
        // transition is deliberately jarring (SmashCut / JumpCut).
        let Some(exit) = prev.motion_vector_exit else { continue };
        let Some(entry) = next.motion_vector_entry else { continue };
        let allow_jarring = matches!(
            next.transition_in.as_ref().map(|t| t.kind),
            Some(TransitionKind::SmashCut) | Some(TransitionKind::JumpCut)
        );
        if allow_jarring {
            continue;
        }
        let dot = exit[0] * entry[0] + exit[1] * entry[1];
        let mag_a = (exit[0] * exit[0] + exit[1] * exit[1]).sqrt();
        let mag_b = (entry[0] * entry[0] + entry[1] * entry[1]).sqrt();
        if mag_a < 0.1 || mag_b < 0.1 {
            continue;
        }
        let cos = dot / (mag_a * mag_b);
        let angle_deg = cos.clamp(-1.0, 1.0).acos().to_degrees();
        if angle_deg > 30.0 {
            findings.push(warn(
                &next.id,
                format!(
                    "motion vector discontinuity from previous shot: {:.0}° (>30°)",
                    angle_deg
                ),
            ));
        }
    }
}

fn check_generation_manifests(sb: &Storyboard, findings: &mut Vec<StoryboardFinding>) {
    for shot in &sb.shots {
        match &shot.generation {
            crate::storyboard::Generation::StockSearch { query, .. } => {
                if query.trim().is_empty() {
                    findings.push(err(&shot.id, "stock-search query is empty"));
                }
            }
            crate::storyboard::Generation::Img2Vid {
                still,
                motion_prompt,
                backend,
                ..
            } => {
                if still.trim().is_empty() {
                    findings.push(err(&shot.id, "img2vid still path is empty"));
                }
                if motion_prompt.trim().is_empty() {
                    findings.push(warn(&shot.id, "img2vid motion_prompt is empty"));
                }
                if backend.trim().is_empty() {
                    findings.push(err(&shot.id, "img2vid backend is empty"));
                }
            }
            crate::storyboard::Generation::Txt2Vid { prompt, backend, .. } => {
                if prompt.trim().is_empty() {
                    findings.push(err(&shot.id, "txt2vid prompt is empty"));
                }
                if backend.trim().is_empty() {
                    findings.push(err(&shot.id, "txt2vid backend is empty"));
                }
            }
            crate::storyboard::Generation::Controlnet {
                prompt,
                condition_kind,
                condition_image,
                backend,
                ..
            } => {
                if prompt.trim().is_empty() {
                    findings.push(err(&shot.id, "controlnet prompt is empty"));
                }
                if condition_kind.trim().is_empty() {
                    findings.push(err(&shot.id, "controlnet condition_kind is empty"));
                }
                if condition_image.trim().is_empty() {
                    findings.push(err(&shot.id, "controlnet condition_image is empty"));
                }
                if backend.trim().is_empty() {
                    findings.push(err(&shot.id, "controlnet backend is empty"));
                }
            }
            crate::storyboard::Generation::Native { html } => {
                if html.trim().is_empty() {
                    findings.push(err(&shot.id, "native html path is empty"));
                }
            }
        }
    }
}

fn check_expected_check_targets(sb: &Storyboard, findings: &mut Vec<StoryboardFinding>) {
    for shot in &sb.shots {
        for (i, check) in shot.expected_checks.iter().enumerate() {
            let origin = format!("{}.expected_checks[{i}]", shot.id);
            match check {
                ExpectedCheck::SubjectVisible { selector }
                | ExpectedCheck::InSafeArea { selector, .. }
                | ExpectedCheck::ColorIn { selector, .. } => {
                    if selector.trim().is_empty() {
                        findings.push(err(origin, "selector is empty"));
                    }
                }
                ExpectedCheck::TextVisible { text, .. } => {
                    if text.trim().is_empty() {
                        findings.push(err(origin, "expected text is empty"));
                    }
                }
                ExpectedCheck::OnAllowedSide => {
                    // Requires the scene to have an action_line.
                    let scene = sb.scenes.get(shot.scene_index);
                    if scene.map(|s| s.action_line.is_none()).unwrap_or(true) {
                        findings.push(warn(
                            origin,
                            "OnAllowedSide check requires the scene to define an action_line"
                                .to_string(),
                        ));
                    }
                }
                ExpectedCheck::MotionContinuous { .. } => {
                    if shot.motion_vector_entry.is_none() {
                        findings.push(warn(
                            origin,
                            "MotionContinuous requires motion_vector_entry on this shot"
                                .to_string(),
                        ));
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storyboard::*;

    fn shot(id: &str, idx: usize, start: f32, dur: f32) -> Shot {
        Shot {
            id: id.into(),
            shot_index: idx,
            scene_index: 0,
            screenplay_element_index: idx,
            start_secs: start,
            duration_secs: dur,
            shot_type: ShotType::Ms,
            framing: None,
            camera_movement: CameraMovement::Static,
            camera_side: CameraSide::Center,
            subject: "x".into(),
            generation: Generation::StockSearch {
                query: "x".into(),
                orientation: None,
                resolved_path: None,
            },
            transition_in: None,
            motion_vector_exit: None,
            motion_vector_entry: None,
            audio_ref: None,
            expected_checks: vec![],
            prev_shot_id: None,
            attributes: None,
        }
    }

    fn empty_storyboard() -> Storyboard {
        Storyboard {
            version: 1,
            duration_secs: 5.0,
            fps: 30,
            resolution: [1920, 1080],
            screenplay_ref: "s.fountain".into(),
            velocity_ref: "v.json".into(),
            voices_ref: None,
            style_bible_ref: None,
            scenes: vec![SceneAnnotation {
                scene_index: 0,
                slugline: "INT. ROOM - DAY".into(),
                first_shot: 0,
                shot_count: 1,
                action_line: None,
            }],
            shots: vec![shot("a", 0, 0.0, 5.0)],
        }
    }

    #[test]
    fn clean_storyboard_has_no_findings() {
        let sb = empty_storyboard();
        let f = verify_storyboard(&sb);
        assert!(f.is_empty(), "expected no findings, got {f:?}");
    }

    #[test]
    fn overlapping_shots_are_errors() {
        let mut sb = empty_storyboard();
        sb.shots.push(shot("b", 1, 3.0, 5.0)); // overlaps prior at t=3 < 5
        sb.scenes[0].shot_count = 2;
        let f = verify_storyboard(&sb);
        assert!(f.iter().any(|x| x.message.contains("overlap")));
    }

    #[test]
    fn shot_after_duration_is_error() {
        let mut sb = empty_storyboard();
        sb.shots[0].duration_secs = 100.0;
        let f = verify_storyboard(&sb);
        assert!(f.iter().any(|x| x.message.contains("storyboard duration")));
    }

    #[test]
    fn dangling_scene_index_is_error() {
        let mut sb = empty_storyboard();
        sb.shots[0].scene_index = 42;
        let f = verify_storyboard(&sb);
        assert!(f.iter().any(|x| x.message.contains("out of range")));
    }

    #[test]
    fn fade_without_duration_is_error() {
        let mut sb = empty_storyboard();
        sb.shots.push(Shot {
            transition_in: Some(ShotTransition {
                kind: TransitionKind::FadeIn,
                duration_secs: None,
                direction: None,
                audio_lead_secs: None,
            }),
            ..shot("b", 1, 5.0, 1.0)
        });
        sb.duration_secs = 6.0;
        sb.scenes[0].shot_count = 2;
        let f = verify_storyboard(&sb);
        assert!(
            f.iter().any(|x| x.message.contains("requires duration_secs")),
            "got {f:?}",
        );
    }

    #[test]
    fn cut_with_duration_is_warning() {
        let mut sb = empty_storyboard();
        sb.shots.push(Shot {
            transition_in: Some(ShotTransition {
                kind: TransitionKind::Cut,
                duration_secs: Some(0.5),
                direction: None,
                audio_lead_secs: None,
            }),
            ..shot("b", 1, 5.0, 1.0)
        });
        sb.duration_secs = 6.0;
        sb.scenes[0].shot_count = 2;
        let f = verify_storyboard(&sb);
        assert!(f.iter().any(|x| x.level == StoryboardLevel::Warning
            && x.message.contains("hard cut")));
    }

    #[test]
    fn motion_discontinuity_warns() {
        let mut sb = empty_storyboard();
        sb.shots[0].motion_vector_exit = Some([10.0, 0.0]);
        sb.shots.push(Shot {
            motion_vector_entry: Some([-10.0, 0.0]),
            ..shot("b", 1, 5.0, 1.0)
        });
        sb.duration_secs = 6.0;
        sb.scenes[0].shot_count = 2;
        let f = verify_storyboard(&sb);
        assert!(f.iter().any(|x| x.message.contains("discontinuity")));
    }

    #[test]
    fn smash_cut_excuses_motion_discontinuity() {
        let mut sb = empty_storyboard();
        sb.shots[0].motion_vector_exit = Some([10.0, 0.0]);
        sb.shots.push(Shot {
            motion_vector_entry: Some([-10.0, 0.0]),
            transition_in: Some(ShotTransition {
                kind: TransitionKind::SmashCut,
                duration_secs: None,
                direction: None,
                audio_lead_secs: None,
            }),
            ..shot("b", 1, 5.0, 1.0)
        });
        sb.duration_secs = 6.0;
        sb.scenes[0].shot_count = 2;
        let f = verify_storyboard(&sb);
        assert!(!f.iter().any(|x| x.message.contains("discontinuity")));
    }

    #[test]
    fn empty_query_in_stocksearch_is_error() {
        let mut sb = empty_storyboard();
        sb.shots[0].generation = Generation::StockSearch {
            query: "".into(),
            orientation: None,
            resolved_path: None,
        };
        let f = verify_storyboard(&sb);
        assert!(f.iter().any(|x| x.message.contains("query is empty")));
    }
}
