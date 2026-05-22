//! Per-cut continuity analysis.
//!
//! Emits a structured JSON report listing every cut in the storyboard
//! with the grammar findings that apply. The agent reads this and
//! decides whether to retry / reorder / accept.

use crate::grammar::{
    GrammarSeverity, MOTION_CONTINUITY_TOLERANCE_DEG, SHOT_TYPE_RUN_LIMIT,
};
use crate::storyboard::{CameraSide, ShotType, Storyboard};
use fountain::TransitionKind;
use serde::{Deserialize, Serialize};

/// Top-level continuity report.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContinuityReport {
    /// True when no Error-severity findings were emitted.
    pub ok: bool,
    /// Number of cuts examined (= shots.len().saturating_sub(1)).
    pub cuts_examined: usize,
    /// Number of Error-severity findings.
    pub errors: usize,
    /// Number of Warning-severity findings.
    pub warnings: usize,
    /// Per-cut findings, indexed by `cut_index = next_shot_index - 1`.
    pub findings: Vec<CutFinding>,
}

/// One finding tied to a specific cut between two shots.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CutFinding {
    /// Severity. `error` blocks downstream gen; `warning` is advisory.
    pub level: CutSeverity,
    /// Cut index (0-based; cut N is between shots N and N+1).
    pub cut_index: usize,
    /// Stable id of the shot the cut transitions INTO.
    pub into_shot: String,
    /// Stable id of the shot the cut transitions FROM.
    pub from_shot: String,
    /// Which grammar rule fired.
    pub rule: GrammarRule,
    /// Human-readable description.
    pub message: String,
}

/// Severity alias kept stable in the wire format.
pub type CutSeverity = GrammarSeverity;

/// Which rule produced a finding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GrammarRule {
    /// Camera crossed the action line without a sanctioned excuse.
    OneEightyRule,
    /// Motion vectors disagreed by more than the tolerance.
    MotionContinuity,
    /// Same shot type ran for more than the rhythm limit in a row.
    ShotTypeRun,
    /// Adjacent shots are both wide or both ECU — extreme jump in scale.
    ScaleJump,
    /// A non-establishing wide shot followed a close-up without a
    /// medium beat to bridge.
    MissingMediumBeat,
}

/// Run the grammar gates over a storyboard.
pub fn check_continuity(sb: &Storyboard) -> ContinuityReport {
    let mut findings: Vec<CutFinding> = Vec::new();
    let cuts_examined = sb.shots.len().saturating_sub(1);

    // Shot-type rhythm — scanned across the whole storyboard
    // (continuity isn't scene-scoped for this one).
    check_shot_type_runs(sb, &mut findings);

    // Per-cut checks.
    for (i, w) in sb.shots.windows(2).enumerate() {
        let (prev, next) = (&w[0], &w[1]);
        check_one_eighty(sb, i, prev, next, &mut findings);
        check_motion_continuity(i, prev, next, &mut findings);
        check_scale_jump(i, prev, next, &mut findings);
    }

    let errors = findings
        .iter()
        .filter(|f| f.level == GrammarSeverity::Error)
        .count();
    let warnings = findings
        .iter()
        .filter(|f| f.level == GrammarSeverity::Warning)
        .count();

    ContinuityReport {
        ok: errors == 0,
        cuts_examined,
        errors,
        warnings,
        findings,
    }
}

fn check_one_eighty(
    sb: &Storyboard,
    cut_index: usize,
    prev: &crate::storyboard::Shot,
    next: &crate::storyboard::Shot,
    findings: &mut Vec<CutFinding>,
) {
    // Only enforced within a single scene with an action_line.
    if prev.scene_index != next.scene_index {
        return;
    }
    let Some(scene) = sb.scenes.get(prev.scene_index) else { return };
    if scene.action_line.is_none() {
        return;
    }
    // Center → either side is allowed (establishing → coverage); only
    // flag a hard Left ↔ Right flip.
    let crossed = matches!(
        (prev.camera_side, next.camera_side),
        (CameraSide::Left, CameraSide::Right) | (CameraSide::Right, CameraSide::Left)
    );
    if !crossed {
        return;
    }
    // A whip-pan transition is the sanctioned "I'm crossing the line"
    // device. Smash cuts may also intentionally cross. Anything else
    // is an error.
    let excuse = next.transition_in.as_ref().map(|t| t.kind);
    let acceptable = matches!(
        excuse,
        Some(TransitionKind::WhipPan) | Some(TransitionKind::SmashCut)
    );
    let level = if acceptable {
        GrammarSeverity::Info
    } else {
        GrammarSeverity::Error
    };
    findings.push(CutFinding {
        level,
        cut_index,
        into_shot: next.id.clone(),
        from_shot: prev.id.clone(),
        rule: GrammarRule::OneEightyRule,
        message: format!(
            "camera crossed the action line ({:?} → {:?}){}",
            prev.camera_side,
            next.camera_side,
            match excuse {
                Some(k) => format!(" via {:?}", k),
                None => " with no transition".into(),
            }
        ),
    });
}

fn check_motion_continuity(
    cut_index: usize,
    prev: &crate::storyboard::Shot,
    next: &crate::storyboard::Shot,
    findings: &mut Vec<CutFinding>,
) {
    let Some(exit) = prev.motion_vector_exit else { return };
    let Some(entry) = next.motion_vector_entry else { return };
    let allow_jarring = matches!(
        next.transition_in.as_ref().map(|t| t.kind),
        Some(TransitionKind::SmashCut) | Some(TransitionKind::JumpCut)
    );
    if allow_jarring {
        return;
    }
    let dot = exit[0] * entry[0] + exit[1] * entry[1];
    let mag_a = (exit[0] * exit[0] + exit[1] * exit[1]).sqrt();
    let mag_b = (entry[0] * entry[0] + entry[1] * entry[1]).sqrt();
    if mag_a < 0.1 || mag_b < 0.1 {
        return;
    }
    let cos = dot / (mag_a * mag_b);
    let angle = cos.clamp(-1.0, 1.0).acos().to_degrees();
    if angle > MOTION_CONTINUITY_TOLERANCE_DEG {
        let level = if angle > 90.0 {
            GrammarSeverity::Error
        } else {
            GrammarSeverity::Warning
        };
        findings.push(CutFinding {
            level,
            cut_index,
            into_shot: next.id.clone(),
            from_shot: prev.id.clone(),
            rule: GrammarRule::MotionContinuity,
            message: format!(
                "motion vector turned {:.0}° (tolerance {:.0}°)",
                angle, MOTION_CONTINUITY_TOLERANCE_DEG
            ),
        });
    }
}

fn check_scale_jump(
    cut_index: usize,
    prev: &crate::storyboard::Shot,
    next: &crate::storyboard::Shot,
    findings: &mut Vec<CutFinding>,
) {
    // Scale is a coarse ordering: ECU < CU < MS < MWS < WS < EWS.
    let scale = |t: ShotType| match t {
        ShotType::Ecu => 0,
        ShotType::Cu => 1,
        ShotType::Ms => 2,
        ShotType::Mws => 3,
        ShotType::Ws => 4,
        ShotType::Ews => 5,
    };
    let diff = (scale(prev.shot_type) as i32 - scale(next.shot_type) as i32).abs();
    if diff >= 3 {
        // Smash cuts are the sanctioned scale-jump.
        let excuse = next.transition_in.as_ref().map(|t| t.kind);
        let acceptable = matches!(
            excuse,
            Some(TransitionKind::SmashCut)
                | Some(TransitionKind::MatchCut)
                | Some(TransitionKind::JumpCut)
        );
        let level = if acceptable {
            GrammarSeverity::Info
        } else {
            GrammarSeverity::Warning
        };
        findings.push(CutFinding {
            level,
            cut_index,
            into_shot: next.id.clone(),
            from_shot: prev.id.clone(),
            rule: GrammarRule::ScaleJump,
            message: format!(
                "shot scale jumped from {:?} to {:?} (Δ{} steps)",
                prev.shot_type, next.shot_type, diff
            ),
        });
        // ECU → WS (or reverse) without a medium beat is louder than a
        // generic scale jump — surface that explicitly.
        if (prev.shot_type == ShotType::Ecu
            && matches!(next.shot_type, ShotType::Ws | ShotType::Ews))
            || (next.shot_type == ShotType::Ecu
                && matches!(prev.shot_type, ShotType::Ws | ShotType::Ews))
        {
            findings.push(CutFinding {
                level: GrammarSeverity::Warning,
                cut_index,
                into_shot: next.id.clone(),
                from_shot: prev.id.clone(),
                rule: GrammarRule::MissingMediumBeat,
                message: "ECU↔WS without an intermediate MS beat — consider bridging".into(),
            });
        }
    }
}

fn check_shot_type_runs(sb: &Storyboard, findings: &mut Vec<CutFinding>) {
    if sb.shots.is_empty() {
        return;
    }
    let mut run = 1usize;
    let mut run_start = 0usize;
    for i in 1..sb.shots.len() {
        if sb.shots[i].shot_type == sb.shots[i - 1].shot_type {
            run += 1;
            if run == SHOT_TYPE_RUN_LIMIT + 1 {
                findings.push(CutFinding {
                    level: GrammarSeverity::Warning,
                    cut_index: i - 1,
                    into_shot: sb.shots[i].id.clone(),
                    from_shot: sb.shots[i - 1].id.clone(),
                    rule: GrammarRule::ShotTypeRun,
                    message: format!(
                        "{:?} run of {} starting at shot {} — shot variety expected",
                        sb.shots[i].shot_type, run, run_start
                    ),
                });
            }
        } else {
            run = 1;
            run_start = i;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storyboard::*;

    fn make_shot(id: &str, idx: usize, scene: usize, kind: ShotType, side: CameraSide) -> Shot {
        Shot {
            id: id.into(),
            shot_index: idx,
            scene_index: scene,
            screenplay_element_index: idx,
            start_secs: idx as f32,
            duration_secs: 1.0,
            shot_type: kind,
            framing: None,
            camera_movement: CameraMovement::Static,
            camera_side: side,
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

    fn make_sb(shots: Vec<Shot>, scenes: Vec<SceneAnnotation>) -> Storyboard {
        Storyboard {
            version: 1,
            duration_secs: shots.iter().map(|s| s.duration_secs).sum::<f32>().max(0.1),
            fps: 30,
            resolution: [1920, 1080],
            screenplay_ref: "x".into(),
            velocity_ref: "v".into(),
            voices_ref: None,
            style_bible_ref: None,
            scenes,
            shots,
        }
    }

    #[test]
    fn empty_storyboard_is_ok() {
        let sb = make_sb(vec![], vec![]);
        let r = check_continuity(&sb);
        assert!(r.ok);
        assert_eq!(r.cuts_examined, 0);
    }

    #[test]
    fn one_eighty_crossing_without_excuse_is_error() {
        let shots = vec![
            make_shot("a", 0, 0, ShotType::Cu, CameraSide::Left),
            make_shot("b", 1, 0, ShotType::Cu, CameraSide::Right),
        ];
        let scenes = vec![SceneAnnotation {
            scene_index: 0,
            slugline: "INT. ROOM - DAY".into(),
            first_shot: 0,
            shot_count: 2,
            action_line: Some(ActionLine {
                from: [0.2, 0.5],
                to: [0.8, 0.5],
                labels: vec!["A".into(), "B".into()],
            }),
        }];
        let r = check_continuity(&make_sb(shots, scenes));
        assert!(!r.ok);
        assert!(r.findings.iter().any(|f| f.rule == GrammarRule::OneEightyRule
            && f.level == GrammarSeverity::Error));
    }

    #[test]
    fn one_eighty_with_whip_pan_is_info_only() {
        let mut shots = vec![
            make_shot("a", 0, 0, ShotType::Cu, CameraSide::Left),
            make_shot("b", 1, 0, ShotType::Cu, CameraSide::Right),
        ];
        shots[1].transition_in = Some(ShotTransition {
            kind: TransitionKind::WhipPan,
            duration_secs: Some(0.25),
            direction: Some("whip-pan-left".into()),
            audio_lead_secs: None,
        });
        let scenes = vec![SceneAnnotation {
            scene_index: 0,
            slugline: "INT. ROOM - DAY".into(),
            first_shot: 0,
            shot_count: 2,
            action_line: Some(ActionLine {
                from: [0.2, 0.5],
                to: [0.8, 0.5],
                labels: vec!["A".into(), "B".into()],
            }),
        }];
        let r = check_continuity(&make_sb(shots, scenes));
        assert!(r.ok, "expected ok report, got {r:#?}");
        assert!(r.findings.iter().any(|f| f.rule == GrammarRule::OneEightyRule
            && f.level == GrammarSeverity::Info));
    }

    #[test]
    fn motion_reversal_without_excuse_is_error() {
        let mut shots = vec![
            make_shot("a", 0, 0, ShotType::Ms, CameraSide::Center),
            make_shot("b", 1, 0, ShotType::Ms, CameraSide::Center),
        ];
        shots[0].motion_vector_exit = Some([10.0, 0.0]);
        shots[1].motion_vector_entry = Some([-10.0, 0.0]); // 180°
        let scenes = vec![SceneAnnotation {
            scene_index: 0,
            slugline: "INT. ROOM - DAY".into(),
            first_shot: 0,
            shot_count: 2,
            action_line: None,
        }];
        let r = check_continuity(&make_sb(shots, scenes));
        assert!(r.findings.iter().any(|f| f.rule == GrammarRule::MotionContinuity
            && f.level == GrammarSeverity::Error));
    }

    #[test]
    fn shot_type_run_of_three_warns() {
        let shots = vec![
            make_shot("a", 0, 0, ShotType::Cu, CameraSide::Center),
            make_shot("b", 1, 0, ShotType::Cu, CameraSide::Center),
            make_shot("c", 2, 0, ShotType::Cu, CameraSide::Center),
        ];
        let scenes = vec![SceneAnnotation {
            scene_index: 0,
            slugline: "INT. ROOM - DAY".into(),
            first_shot: 0,
            shot_count: 3,
            action_line: None,
        }];
        let r = check_continuity(&make_sb(shots, scenes));
        assert!(r.findings.iter().any(|f| f.rule == GrammarRule::ShotTypeRun));
    }

    #[test]
    fn ecu_to_ws_without_excuse_warns_with_missing_medium_beat() {
        let shots = vec![
            make_shot("a", 0, 0, ShotType::Ecu, CameraSide::Center),
            make_shot("b", 1, 0, ShotType::Ws, CameraSide::Center),
        ];
        let scenes = vec![SceneAnnotation {
            scene_index: 0,
            slugline: "INT. ROOM - DAY".into(),
            first_shot: 0,
            shot_count: 2,
            action_line: None,
        }];
        let r = check_continuity(&make_sb(shots, scenes));
        assert!(r.findings.iter().any(|f| f.rule == GrammarRule::ScaleJump));
        assert!(r.findings.iter().any(|f| f.rule == GrammarRule::MissingMediumBeat));
    }

    #[test]
    fn smash_cut_excuses_scale_jump() {
        let mut shots = vec![
            make_shot("a", 0, 0, ShotType::Ecu, CameraSide::Center),
            make_shot("b", 1, 0, ShotType::Ws, CameraSide::Center),
        ];
        shots[1].transition_in = Some(ShotTransition {
            kind: TransitionKind::SmashCut,
            duration_secs: None,
            direction: None,
            audio_lead_secs: None,
        });
        let scenes = vec![SceneAnnotation {
            scene_index: 0,
            slugline: "INT. ROOM - DAY".into(),
            first_shot: 0,
            shot_count: 2,
            action_line: None,
        }];
        let r = check_continuity(&make_sb(shots, scenes));
        let scale_jump = r
            .findings
            .iter()
            .find(|f| f.rule == GrammarRule::ScaleJump)
            .expect("scale jump finding");
        assert_eq!(scale_jump.level, GrammarSeverity::Info);
    }
}
