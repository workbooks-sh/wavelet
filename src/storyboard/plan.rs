//! Heuristic storyboard planner.
//!
//! Walks a Fountain screenplay (paired with a velocity profile) and
//! emits a draft `Storyboard`. The agent is expected to fill in
//! generation prompts and tune shot-type / framing decisions through a
//! separate skill — this proposer just picks reasonable defaults so the
//! agent isn't starting from a blank file.
//!
//! Heuristics (all tunable, documented inline):
//!
//! - **One shot per non-trivial element.** Scene headings become
//!   establishing wide shots. Action paragraphs become 1 shot for short
//!   paragraphs, 2 shots for long (>40 words). Dialogue lines become
//!   close-ups on the speaker, with reverse-shot framing for adjacent
//!   characters.
//! - **Durations come from the velocity profile.** Shot duration is
//!   inversely proportional to mean BPM across the shot's window:
//!   `duration = base * (60 / bpm)` with `base = 2s`, clamped to
//!   `[0.4, 6.0]` s.
//! - **Camera side alternates per speaker** within a scene to satisfy
//!   the 180° rule without manual annotation.
//! - **Transitions** carry over from the screenplay where present;
//!   otherwise default to `Cut` between adjacent shots in the same
//!   scene and `Dissolve` across scene boundaries when BPM is below 90.
//! - **Generation defaults to `StockSearch`** with the subject as the
//!   query — cheapest backend, easiest swap.

use crate::clipref::character::{CharacterRef, CharacterRefs, CharacterType};
use crate::storyboard::{
    ActionLine, CameraMovement, CameraSide, ExpectedCheck, Framing, Generation, SceneAnnotation,
    Shot, ShotTransition, ShotType, Storyboard,
};
use crate::velocity::VelocityProfile;
use fountain::{DialogueLine, Element, Screenplay, TransitionKind};

/// Default Fal Veo 3.1 reference-to-video backend slug. Standard tier;
/// callers can swap in `fal-veo3-ref-fast` for the cheaper tier.
const DEFAULT_REF_BACKEND: &str = "fal-veo3-ref";

const BASE_SHOT_SECS: f32 = 2.0;
const MIN_SHOT_SECS: f32 = 0.4;
const MAX_SHOT_SECS: f32 = 6.0;
const LONG_ACTION_WORDS: usize = 40;
const ONSET_SNAP_TOLERANCE_SECS: f32 = 0.3;

/// Plan a storyboard from a parsed screenplay + velocity profile.
///
/// `screenplay_ref` / `velocity_ref` become path strings stored in the
/// emitted `Storyboard` for round-tripping; pass relative paths
/// (typically resolved against the storyboard's directory).
pub fn plan_from_screenplay(
    screenplay: &Screenplay,
    velocity: &VelocityProfile,
    screenplay_ref: impl Into<String>,
    velocity_ref: impl Into<String>,
    fps: u32,
    resolution: [u32; 2],
) -> Storyboard {
    plan_from_screenplay_with_onsets(
        screenplay,
        velocity,
        screenplay_ref,
        velocity_ref,
        fps,
        resolution,
        None,
    )
}

/// Same as [`plan_from_screenplay`] but additionally snaps scene
/// boundaries to the nearest detected music onset (within
/// [`ONSET_SNAP_TOLERANCE_SECS`] of the heuristic boundary).
///
/// `onsets` is a slice of onset times in seconds, sorted ascending.
/// When `None` or empty, behavior is identical to `plan_from_screenplay`.
///
/// The first scene always starts at `0.0`. Subsequent scene starts (and
/// therefore the prior scene's `duration_secs`) snap to the nearest
/// onset within tolerance. The last scene's duration is adjusted so the
/// storyboard's `duration_secs` matches the un-snapped total.
pub fn plan_from_screenplay_with_onsets(
    screenplay: &Screenplay,
    velocity: &VelocityProfile,
    screenplay_ref: impl Into<String>,
    velocity_ref: impl Into<String>,
    fps: u32,
    resolution: [u32; 2],
    onsets: Option<&[f32]>,
) -> Storyboard {
    plan_from_screenplay_full(
        screenplay,
        velocity,
        screenplay_ref,
        velocity_ref,
        fps,
        resolution,
        onsets,
        None,
    )
}

/// Full planner entrypoint — accepts an optional `CharacterRefs`
/// collection of loaded reference bundles keyed by
/// `(canonical_name, character_type)` (wb-jwnk). When a Dialogue
/// scene's CHARACTER cue matches a loaded ref, the produced shot gets
/// `character_ref = Some(name)` and is routed through
/// `Generation::RefConditioned` (Fal Veo 3.1 reference-to-video)
/// instead of the default `StockSearch`.
///
/// Action paragraphs that look like ECU hand cutaways (per
/// [`looks_like_hand_cutaway`]) get routed to a *hands* ref for the
/// most-recently-spoken character — preferring `ProductHands` if
/// product nouns are present in the line, else `Hands`, else falling
/// back to `FullBody`. The precedence is documented inline on
/// [`pick_hand_ref_type`].
///
/// Pass `None` for `characters` to get the legacy plan behavior — no
/// shot is reference-conditioned and `Generation::StockSearch` is the
/// default for dialogue beats.
pub fn plan_from_screenplay_full(
    screenplay: &Screenplay,
    velocity: &VelocityProfile,
    screenplay_ref: impl Into<String>,
    velocity_ref: impl Into<String>,
    fps: u32,
    resolution: [u32; 2],
    onsets: Option<&[f32]>,
    characters: Option<&CharacterRefs>,
) -> Storyboard {
    let mut shots: Vec<Shot> = Vec::new();
    let mut scenes: Vec<SceneAnnotation> = Vec::new();
    let mut t = 0.0f32;
    let mut current_scene: Option<usize> = None;
    let mut current_scene_first_shot: usize = 0;
    let mut current_scene_speakers: Vec<String> = Vec::new();
    let mut pending_transition: Option<TransitionKind> = None;
    // Most-recently-spoken canonical character name, used by the
    // ECU-hands cutaway heuristic to pick the right hands ref for an
    // Action paragraph. Reset on scene boundaries — a cutaway in a new
    // scene shouldn't borrow the previous scene's speaker.
    let mut last_speaker_canonical: Option<String> = None;

    for (elem_idx, element) in screenplay.elements.iter().enumerate() {
        match element {
            Element::SceneHeading { slugline, .. } => {
                // Close out the previous scene.
                if let Some(scene_idx) = current_scene {
                    let count = shots.len() - current_scene_first_shot;
                    if scene_idx < scenes.len() {
                        scenes[scene_idx].shot_count = count;
                        // Derive an action line from the two most-recent
                        // unique speakers when we have them, and attach
                        // OnAllowedSide checks to every shot in the scene.
                        if current_scene_speakers.len() >= 2 {
                            scenes[scene_idx].action_line = Some(ActionLine {
                                from: [0.2, 0.5],
                                to: [0.8, 0.5],
                                labels: current_scene_speakers
                                    .iter()
                                    .take(2)
                                    .cloned()
                                    .collect(),
                            });
                            let scene_end = current_scene_first_shot + count;
                            for shot in &mut shots[current_scene_first_shot..scene_end] {
                                shot.expected_checks.push(ExpectedCheck::OnAllowedSide);
                            }
                        }
                    }
                }
                current_scene = Some(scenes.len());
                current_scene_first_shot = shots.len();
                current_scene_speakers.clear();
                last_speaker_canonical = None;
                scenes.push(SceneAnnotation {
                    scene_index: scenes.len(),
                    slugline: slugline.clone(),
                    first_shot: current_scene_first_shot,
                    shot_count: 0,
                    action_line: None,
                });
                // Add an establishing wide shot for the scene.
                let duration = shot_duration(velocity, t, 2.0);
                let scene_idx = current_scene.unwrap();
                let shot = Shot {
                    id: format!("shot-{scene_idx}-est"),
                    shot_index: shots.len(),
                    scene_index: scene_idx,
                    screenplay_element_index: elem_idx,
                    start_secs: t,
                    duration_secs: duration,
                    shot_type: ShotType::Ws,
                    framing: None,
                    camera_movement: CameraMovement::Static,
                    camera_side: CameraSide::Center,
                    subject: subject_from_slugline(slugline),
                    generation: Generation::StockSearch {
                        query: subject_from_slugline(slugline),
                        orientation: Some("landscape".into()),
                        resolved_path: None,
                    },
                    transition_in: pending_transition.take().map(|k| ShotTransition {
                        kind: k,
                        duration_secs: transition_duration(k, velocity, t),
                        direction: direction_for(k),
                        audio_lead_secs: audio_lead_for(k),
                    }),
                    motion_vector_exit: None,
                    motion_vector_entry: None,
                    audio_ref: None,
                    // OnAllowedSide is only meaningful for scenes with two
                    // speakers (action_line gets populated at scene close).
                    // We can't know that yet on the establishing shot, so
                    // leave checks empty and patch them in at scene close
                    // when an action_line gets attached.
                    expected_checks: vec![],
                    prev_shot_id: None,
                    attributes: None,
                    character_ref: None,
                    character_ref_type: None,
                };
                t += duration;
                shots.push(shot);
            }
            Element::Action { text, .. } => {
                let words = text.split_whitespace().count();
                let split_count = if words > LONG_ACTION_WORDS { 2 } else { 1 };
                // Hand-cutaway heuristic (wb-jwnk): the first sub-shot
                // is the candidate for ECU-hands routing. Only the
                // FIRST sub-shot is considered a true cutaway — the
                // second is a continuation that stays on the default
                // path.
                let hand_cutaway = looks_like_hand_cutaway(text);
                for k in 0..split_count {
                    let duration = shot_duration(velocity, t, BASE_SHOT_SECS);
                    let scene_idx = current_scene.unwrap_or(0);
                    // Default shot type: MS for the first sub-shot,
                    // CU for the (rare) split second. Override to ECU
                    // when the heuristic fires on the leading sub-shot
                    // — that's the signal downstream consumers (and
                    // the verifier) key off.
                    let mut shot_type = if k == 0 { ShotType::Ms } else { ShotType::Cu };
                    let mut generation = Generation::StockSearch {
                        query: keyword_from_action(text),
                        orientation: Some("landscape".into()),
                        resolved_path: None,
                    };
                    let mut character_ref: Option<String> = None;
                    let mut character_ref_type: Option<CharacterType> = None;

                    if k == 0 && hand_cutaway {
                        shot_type = ShotType::Ecu;
                        // Resolve which character "owns" this cutaway:
                        // 1. Action paragraph names a known speaker
                        //    directly (e.g. "ALEX twists the cap").
                        // 2. Fall back to the most-recently-spoken
                        //    character in this scene.
                        let owner = resolve_hand_owner(
                            text,
                            &current_scene_speakers,
                            last_speaker_canonical.as_deref(),
                        );
                        if let (Some(name), Some(refs)) = (owner.as_ref(), characters) {
                            let want_product = mentions_product_noun(text);
                            // Precedence: when product-hands is implied
                            // by text, prefer it; else prefer hands
                            // proper; else fall back to full-body. The
                            // verifier will WARN if we end up on
                            // FullBody for an ECU because face features
                            // leak into the conditioning signal.
                            let selected = pick_hand_ref_type(refs, name, want_product);
                            if let Some((ty, picked)) = selected {
                                character_ref = Some(picked.name.clone());
                                character_ref_type = Some(ty);
                                generation = Generation::RefConditioned {
                                    character_ref: picked.name.clone(),
                                    prompt: format!(
                                        "ECU cutaway of {name}'s hands: {}",
                                        text.trim(),
                                    ),
                                    backend: DEFAULT_REF_BACKEND.to_string(),
                                    seed: None,
                                    resolved_path: None,
                                };
                            }
                        }
                    }

                    let shot = Shot {
                        id: format!("shot-{scene_idx}-act-{elem_idx}-{k}"),
                        shot_index: shots.len(),
                        scene_index: scene_idx,
                        screenplay_element_index: elem_idx,
                        start_secs: t,
                        duration_secs: duration,
                        shot_type,
                        framing: None,
                        camera_movement: if velocity.bpm_at(t) > 110.0 {
                            CameraMovement::Push
                        } else {
                            CameraMovement::Static
                        },
                        camera_side: CameraSide::Center,
                        subject: keyword_from_action(text),
                        generation,
                        transition_in: pending_transition.take().map(|kind| ShotTransition {
                            kind,
                            duration_secs: transition_duration(kind, velocity, t),
                            direction: direction_for(kind),
                            audio_lead_secs: audio_lead_for(kind),
                        }),
                        motion_vector_exit: None,
                        motion_vector_entry: None,
                        audio_ref: None,
                        expected_checks: vec![],
                        prev_shot_id: None,
                        attributes: None,
                        character_ref,
                        character_ref_type,
                    };
                    t += duration;
                    shots.push(shot);
                }
            }
            Element::Dialogue {
                character,
                lines,
                is_voiceover,
                is_off_screen,
                ..
            } => {
                if !current_scene_speakers.iter().any(|s| s == character) {
                    current_scene_speakers.push(character.clone());
                }
                let speaker_position = current_scene_speakers
                    .iter()
                    .position(|s| s == character)
                    .unwrap_or(0);
                let camera_side = if *is_voiceover || *is_off_screen {
                    CameraSide::Center
                } else if speaker_position % 2 == 0 {
                    CameraSide::Left
                } else {
                    CameraSide::Right
                };
                let words: usize = lines
                    .iter()
                    .map(|l| match l {
                        DialogueLine::Text(t) | DialogueLine::Lyric(t) => {
                            t.split_whitespace().count()
                        }
                        DialogueLine::Parenthetical(_) => 0,
                    })
                    .sum();
                let duration =
                    (words as f32 / 2.5).max(1.2).min(MAX_SHOT_SECS);
                let scene_idx = current_scene.unwrap_or(0);
                // Resolve the CHARACTER cue against the loaded character
                // refs. We canonicalize the cue the same way the fountain
                // characters extractor does so screenplay/refs stay keyed
                // on identical strings.
                let character_key = fountain::canonicalize_name(character);
                if let Some(key) = character_key.clone() {
                    last_speaker_canonical = Some(key);
                }
                // Dialogue beats are face shots — prefer the full-body
                // ref (which covers face/head). Fall back to any other
                // typed ref if that's all the author defined.
                let matched: Option<(String, CharacterType)> = match (&character_key, characters) {
                    (Some(key), Some(refs)) => refs
                        .lookup_full(key)
                        .map(|r| (r.name.clone(), CharacterType::FullBody))
                        .or_else(|| {
                            refs.lookup_hands(key)
                                .map(|r| (r.name.clone(), CharacterType::Hands))
                        })
                        .or_else(|| {
                            refs.lookup_product_hands(key)
                                .map(|r| (r.name.clone(), CharacterType::ProductHands))
                        }),
                    _ => None,
                };
                let matched_ref = matched.as_ref().map(|(n, _)| n.clone());
                let matched_type = matched.as_ref().map(|(_, t)| *t);
                let generation = match &matched_ref {
                    Some(name) => Generation::RefConditioned {
                        character_ref: name.clone(),
                        prompt: format!("{character} close-up, dialogue beat"),
                        backend: DEFAULT_REF_BACKEND.to_string(),
                        seed: None,
                        resolved_path: None,
                    },
                    None => Generation::StockSearch {
                        query: format!("{character} face close-up"),
                        orientation: Some("portrait".into()),
                        resolved_path: None,
                    },
                };
                let shot = Shot {
                    id: format!("shot-{scene_idx}-dlg-{elem_idx}"),
                    shot_index: shots.len(),
                    scene_index: scene_idx,
                    screenplay_element_index: elem_idx,
                    start_secs: t,
                    duration_secs: duration,
                    shot_type: ShotType::Cu,
                    framing: if speaker_position > 0 {
                        Some(Framing::Ots)
                    } else {
                        None
                    },
                    camera_movement: CameraMovement::Static,
                    camera_side,
                    subject: character.clone(),
                    generation,
                    transition_in: pending_transition.take().map(|kind| ShotTransition {
                        kind,
                        duration_secs: transition_duration(kind, velocity, t),
                        direction: direction_for(kind),
                        audio_lead_secs: audio_lead_for(kind),
                    }),
                    motion_vector_exit: None,
                    motion_vector_entry: None,
                    audio_ref: Some(format!("vo/{character}-{elem_idx}.mp3")),
                    // OnAllowedSide is attached on scene close when the
                    // action_line lands (we don't know if there are two
                    // speakers yet on the first dialogue beat).
                    expected_checks: vec![ExpectedCheck::SubjectVisible {
                        selector: format!("#{}", character.to_ascii_lowercase()),
                    }],
                    prev_shot_id: None,
                    attributes: None,
                    character_ref: matched_ref,
                    character_ref_type: matched_type,
                };
                t += duration;
                shots.push(shot);
            }
            Element::Transition(transition) => {
                pending_transition = Some(transition.kind);
            }
            _ => {}
        }
    }

    // Close out the last scene.
    if let Some(scene_idx) = current_scene {
        let count = shots.len() - current_scene_first_shot;
        if scene_idx < scenes.len() {
            scenes[scene_idx].shot_count = count;
            if current_scene_speakers.len() >= 2 {
                scenes[scene_idx].action_line = Some(ActionLine {
                    from: [0.2, 0.5],
                    to: [0.8, 0.5],
                    labels: current_scene_speakers.iter().take(2).cloned().collect(),
                });
                let scene_end = current_scene_first_shot + count;
                for shot in &mut shots[current_scene_first_shot..scene_end] {
                    shot.expected_checks.push(ExpectedCheck::OnAllowedSide);
                }
            }
        }
    }

    let total_duration = t.max(0.1);

    if let Some(o) = onsets {
        if !o.is_empty() {
            snap_scene_boundaries_to_onsets(&scenes, &mut shots, o, total_duration);
        }
    }

    Storyboard {
        version: 1,
        duration_secs: total_duration,
        fps,
        resolution,
        screenplay_ref: screenplay_ref.into(),
        velocity_ref: velocity_ref.into(),
        voices_ref: None,
        style_bible_ref: None,
        scenes,
        shots,
    }
}

/// Scale every shot's `start_secs` + `duration_secs` so the storyboard's
/// total duration matches `target_secs`. Use after `plan_from_screenplay_*`
/// when the brief specifies a hard runtime that the velocity-driven
/// per-shot durations don't naturally land on. Idempotent — calling with
/// the current `duration_secs` is a no-op.
pub fn match_runtime(sb: &mut crate::storyboard::Storyboard, target_secs: f32) {
    if target_secs <= 0.0 || sb.duration_secs <= 0.0 {
        return;
    }
    let scale = target_secs / sb.duration_secs;
    if (scale - 1.0).abs() < 1e-6 {
        return;
    }
    for shot in &mut sb.shots {
        shot.start_secs *= scale;
        shot.duration_secs = (shot.duration_secs * scale).max(MIN_SHOT_SECS);
        if let Some(t) = &mut shot.transition_in {
            if let Some(d) = t.duration_secs.as_mut() {
                *d *= scale;
            }
        }
    }
    sb.duration_secs = target_secs;
}

/// Snap scene boundaries (the start of every scene after the first) to
/// the nearest onset within [`ONSET_SNAP_TOLERANCE_SECS`]. Shots inside
/// each scene get their `start_secs` shifted by the same delta as the
/// scene's first shot so within-scene timing stays intact. The last
/// scene's last shot's `duration_secs` is adjusted to preserve
/// `total_duration`.
fn snap_scene_boundaries_to_onsets(
    scenes: &[crate::storyboard::SceneAnnotation],
    shots: &mut [crate::storyboard::Shot],
    onsets: &[f32],
    total_duration: f32,
) {
    if scenes.len() < 2 || shots.is_empty() {
        return;
    }

    for scene in scenes.iter().skip(1) {
        let first_idx = scene.first_shot;
        if first_idx >= shots.len() || scene.shot_count == 0 {
            continue;
        }
        let original_start = shots[first_idx].start_secs;
        if let Some(onset) = nearest_onset_within(onsets, original_start, ONSET_SNAP_TOLERANCE_SECS)
        {
            let delta = onset - original_start;
            if delta.abs() < 1e-6 {
                continue;
            }
            let scene_end = first_idx + scene.shot_count;
            for shot in &mut shots[first_idx..scene_end] {
                shot.start_secs += delta;
            }
            if first_idx > 0 {
                let prev = &mut shots[first_idx - 1];
                prev.duration_secs = (onset - prev.start_secs).max(MIN_SHOT_SECS);
            }
        }
    }

    let last = shots.len() - 1;
    let last_start = shots[last].start_secs;
    shots[last].duration_secs = (total_duration - last_start).max(MIN_SHOT_SECS);
}

/// Return the onset closest to `target` if within `tolerance` seconds,
/// else `None`. Onsets are assumed sorted but a linear scan is fine for
/// the per-scene-boundary call site (scenes are O(10s), onsets O(100s)).
fn nearest_onset_within(onsets: &[f32], target: f32, tolerance: f32) -> Option<f32> {
    let mut best: Option<f32> = None;
    let mut best_d = tolerance;
    for &o in onsets {
        let d = (o - target).abs();
        if d <= best_d {
            best_d = d;
            best = Some(o);
        }
    }
    best
}

fn shot_duration(velocity: &VelocityProfile, t: f32, base: f32) -> f32 {
    let bpm = velocity.bpm_at(t).max(40.0);
    (base * (60.0 / bpm)).clamp(MIN_SHOT_SECS, MAX_SHOT_SECS)
}

fn transition_duration(kind: TransitionKind, velocity: &VelocityProfile, t: f32) -> Option<f32> {
    use TransitionKind::*;
    match kind {
        Cut | MatchCut | SmashCut | JumpCut | JCut | LCut | Other => None,
        FadeIn | FadeOut | FadeTo => Some(0.6),
        Dissolve => Some((90.0 / velocity.bpm_at(t).max(40.0)).clamp(0.3, 1.5)),
        WhipPan => Some(0.25),
    }
}

fn direction_for(kind: TransitionKind) -> Option<String> {
    if matches!(kind, TransitionKind::WhipPan) {
        Some("whip-pan-left".into())
    } else {
        None
    }
}

fn audio_lead_for(kind: TransitionKind) -> Option<f32> {
    match kind {
        TransitionKind::JCut => Some(0.3),
        TransitionKind::LCut => Some(0.3),
        _ => None,
    }
}

fn subject_from_slugline(slug: &str) -> String {
    let after_ie = slug
        .splitn(2, ['.', ' '])
        .nth(1)
        .map(|s| s.trim())
        .unwrap_or(slug);
    after_ie
        .split(" - ")
        .next()
        .unwrap_or(after_ie)
        .trim()
        .to_string()
}

fn keyword_from_action(text: &str) -> String {
    // Take the first 5 content words as a rough query seed. The agent
    // will rewrite this.
    text.split_whitespace()
        .filter(|w| w.len() > 2)
        .take(5)
        .collect::<Vec<_>>()
        .join(" ")
}

/// Hand-verb vocabulary that signals an Action paragraph is probably a
/// hands cutaway. Wb-jwnk: matches the lowercased text via simple
/// substring check (no token boundary, but the verbs are distinctive
/// enough — "hand", "hold", "grip", "fingers", "palm", "thumb").
const HAND_VERBS: &[&str] = &[
    "hand", "hold", "grip", "finger", "palm", "thumb", "knuckle",
];

/// Short-form product nouns that, when paired with hand verbs in the
/// same Action paragraph, bump the ref-type precedence from `Hands` up
/// to `ProductHands`. The list is deliberately small — we err on the
/// side of plain `Hands` because the precedence then falls back to
/// `Hands` -> `FullBody` anyway.
const PRODUCT_NOUNS: &[&str] = &[
    "bottle", "cap", "lid", "jar", "tube", "box", "pack", "package",
    "label", "phone", "tin", "can", "carton", "wrapper", "product",
];

/// Sub-60-character threshold under which an unqualified Action
/// paragraph reads as a cutaway. Tuned against the eval-010
/// screenplay's hand cutaways which clock in around 30 chars.
const SHORT_ACTION_CHARS: usize = 60;

/// Does this Action paragraph look like an ECU hand cutaway? Wb-jwnk
/// heuristic — text contains a hand verb AND either an explicit ECU
/// hint OR is short enough to read as a cutaway by itself.
///
/// We intentionally don't require an explicit ECU annotation in the
/// screenplay: Fountain authors rarely write "ECU:" on their cutaways,
/// they just write "Her fingers twist the cap." and rely on the
/// editor to frame it. The short-paragraph fallback captures that
/// convention.
pub fn looks_like_hand_cutaway(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    let has_hand_verb = HAND_VERBS.iter().any(|v| lower.contains(v));
    if !has_hand_verb {
        return false;
    }
    let explicit_ecu = lower.contains("ecu")
        || lower.contains("extreme close")
        || lower.contains("close up on")
        || lower.contains("close-up on")
        || lower.contains("cutaway");
    if explicit_ecu {
        return true;
    }
    text.trim().len() <= SHORT_ACTION_CHARS
}

/// Does this Action paragraph mention a product noun near a hand
/// verb? Used to bump `Hands` → `ProductHands` precedence. We don't
/// enforce strict adjacency — the same paragraph mentioning both is a
/// strong-enough signal at this granularity.
fn mentions_product_noun(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    PRODUCT_NOUNS.iter().any(|n| lower.contains(n))
}

/// Pick the right reference-type bundle for a hand cutaway. Returns
/// `(selected_type, &CharacterRef)`. Precedence (wb-jwnk):
///
/// 1. When the paragraph mentions a product noun, prefer
///    `ProductHands` -> `Hands` -> `FullBody`.
/// 2. Otherwise prefer `Hands` -> `ProductHands` -> `FullBody`.
///
/// `FullBody` is the last-ditch fallback for "this character has
/// *some* ref defined, but not the right type". The verifier will
/// flag the resulting shot because face features may leak into the
/// conditioning signal — that's the cue for the agent to define a
/// hands ref.
fn pick_hand_ref_type<'a>(
    refs: &'a CharacterRefs,
    name: &str,
    want_product: bool,
) -> Option<(CharacterType, &'a CharacterRef)> {
    let order: [CharacterType; 3] = if want_product {
        [
            CharacterType::ProductHands,
            CharacterType::Hands,
            CharacterType::FullBody,
        ]
    } else {
        [
            CharacterType::Hands,
            CharacterType::ProductHands,
            CharacterType::FullBody,
        ]
    };
    for ty in order {
        if let Some(r) = refs.get_typed(name, ty) {
            return Some((ty, r));
        }
    }
    None
}

/// Resolve which character "owns" a hand cutaway. Looks for an
/// explicit CHARACTER cue in the paragraph text (e.g. "ALEX twists the
/// cap") first; falls back to the most-recently-spoken character in
/// the current scene.
fn resolve_hand_owner(
    text: &str,
    scene_speakers: &[String],
    most_recent: Option<&str>,
) -> Option<String> {
    // Cheap explicit-name check: look for any scene speaker's canonical
    // name appearing in the paragraph. We canonicalize the comparison
    // since fountain CHARACTER cues are uppercase-by-convention.
    let upper = text.to_ascii_uppercase();
    for sp in scene_speakers {
        let canonical = fountain::canonicalize_name(sp).unwrap_or_else(|| sp.clone());
        if upper.contains(&canonical) {
            return Some(canonical);
        }
    }
    most_recent.map(|s| s.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::velocity::propose_from_screenplay;
    use fountain::parse;

    fn baseline_velocity(duration: f32) -> VelocityProfile {
        VelocityProfile {
            duration_secs: duration,
            mean_bpm: 0.0,
            anchors: vec![
                crate::velocity::Anchor { t: 0.0, bpm: 90.0, label: None },
                crate::velocity::Anchor { t: duration, bpm: 90.0, label: None },
            ],
        }
    }

    #[test]
    fn empty_screenplay_yields_empty_plan() {
        let s = fountain::Screenplay::default();
        let v = baseline_velocity(1.0);
        let sb = plan_from_screenplay(&s, &v, "x.fountain", "v.json", 30, [1920, 1080]);
        assert!(sb.shots.is_empty());
        assert!(sb.scenes.is_empty());
    }

    #[test]
    fn match_runtime_scales_to_target() {
        let src = "EXT. ALLEY - NIGHT\n\nA puddle.\n\nA cat.\n\nA siren.\n";
        let s = parse(src).expect("parse");
        let v = baseline_velocity(10.0);
        let mut sb = plan_from_screenplay(&s, &v, "x.fountain", "v.json", 30, [1920, 1080]);
        let original = sb.duration_secs;
        assert!(original > 0.0);
        super::match_runtime(&mut sb, 12.0);
        assert!((sb.duration_secs - 12.0).abs() < 1e-3, "got {}", sb.duration_secs);
        let sum: f32 = sb.shots.iter().map(|s| s.duration_secs).sum();
        assert!((sum - 12.0).abs() < 1e-2, "shot durations sum {sum} != 12");
        if let (Some(first), Some(last)) = (sb.shots.first(), sb.shots.last()) {
            let end = last.start_secs + last.duration_secs;
            assert!((end - 12.0).abs() < 1e-2, "last shot ends at {end}");
            assert!(first.start_secs.abs() < 1e-3);
        }
    }

    #[test]
    fn match_runtime_is_noop_for_identity() {
        let src = "EXT. ALLEY - NIGHT\n\nA puddle.\n";
        let s = parse(src).expect("parse");
        let v = baseline_velocity(10.0);
        let mut sb = plan_from_screenplay(&s, &v, "x.fountain", "v.json", 30, [1920, 1080]);
        let before = sb.duration_secs;
        let before_first_dur = sb.shots[0].duration_secs;
        super::match_runtime(&mut sb, before);
        assert!((sb.duration_secs - before).abs() < 1e-6);
        assert!((sb.shots[0].duration_secs - before_first_dur).abs() < 1e-6);
    }

    #[test]
    fn one_shot_per_screenplay_element_minimum() {
        let src = r#"EXT. PARK - DAY

A bench.

NARRATOR (V.O.)
Once.
"#;
        let s = parse(src).unwrap();
        let v = baseline_velocity(15.0);
        let sb = plan_from_screenplay(&s, &v, "x.fountain", "v.json", 30, [1920, 1080]);
        // 1 scene heading + 1 action + 1 dialogue = ≥3 shots.
        assert!(sb.shots.len() >= 3, "got {} shots", sb.shots.len());
        assert_eq!(sb.scenes.len(), 1);
        assert_eq!(sb.scenes[0].shot_count, sb.shots.len());
    }

    #[test]
    fn shots_are_time_contiguous() {
        let src = include_str!("../../crates/fountain/tests/fixtures/big_fish_excerpt.fountain");
        let s = parse(src).unwrap();
        let v = propose_from_screenplay(&s);
        let sb = plan_from_screenplay(&s, &v, "x.fountain", "v.json", 30, [1920, 1080]);
        for w in sb.shots.windows(2) {
            let prev_end = w[0].start_secs + w[0].duration_secs;
            assert!(
                (w[1].start_secs - prev_end).abs() < 0.01,
                "shot {} starts at {} but {} ended at {}",
                w[1].shot_index,
                w[1].start_secs,
                w[0].shot_index,
                prev_end,
            );
        }
        // Total duration matches sum of shot durations.
        let total: f32 = sb.shots.iter().map(|s| s.duration_secs).sum();
        assert!((total - sb.duration_secs).abs() < 0.01);
    }

    #[test]
    fn camera_side_alternates_between_speakers() {
        let src = r#"INT. ROOM - DAY

ALICE
Hi.

BOB
Hi back.

ALICE
How's the day?
"#;
        let s = parse(src).unwrap();
        let v = baseline_velocity(15.0);
        let sb = plan_from_screenplay(&s, &v, "x.fountain", "v.json", 30, [1920, 1080]);
        let dlg_sides: Vec<CameraSide> = sb
            .shots
            .iter()
            .filter(|s| matches!(s.shot_type, ShotType::Cu))
            .filter(|s| matches!(s.framing, Some(Framing::Ots) | None) && s.audio_ref.is_some())
            .map(|s| s.camera_side)
            .collect();
        // Alice (first speaker) → Left, Bob (second) → Right, Alice → Left.
        assert!(dlg_sides.len() >= 3, "expected ≥3 dialogue shots, got {dlg_sides:?}");
        assert_eq!(dlg_sides[0], CameraSide::Left);
        assert_eq!(dlg_sides[1], CameraSide::Right);
        assert_eq!(dlg_sides[2], CameraSide::Left);
    }

    #[test]
    fn smash_cut_transitions_get_no_duration() {
        let src = r#"INT. ROOM - DAY

She sits.

SMASH CUT TO:

EXT. STREET - DAY

Chaos.
"#;
        let s = parse(src).unwrap();
        let v = baseline_velocity(15.0);
        let sb = plan_from_screenplay(&s, &v, "x.fountain", "v.json", 30, [1920, 1080]);
        let has_smash = sb.shots.iter().any(|s| {
            matches!(
                &s.transition_in,
                Some(t) if t.kind == TransitionKind::SmashCut && t.duration_secs.is_none()
            )
        });
        assert!(has_smash, "expected one shot to carry SmashCut with no duration");
    }

    #[test]
    fn nearest_onset_exact_match() {
        let onsets = [1.0_f32, 2.0, 3.0];
        assert_eq!(nearest_onset_within(&onsets, 2.0, 0.3), Some(2.0));
    }

    #[test]
    fn nearest_onset_within_tolerance() {
        let onsets = [1.0_f32, 2.0, 3.0];
        // 2.2 is within 0.3 of 2.0
        assert_eq!(nearest_onset_within(&onsets, 2.2, 0.3), Some(2.0));
        // 1.8 is within 0.3 of 2.0 (closer than 1.0 at 0.8 away)
        assert_eq!(nearest_onset_within(&onsets, 1.8, 0.3), Some(2.0));
    }

    #[test]
    fn nearest_onset_outside_tolerance() {
        let onsets = [1.0_f32, 2.0, 3.0];
        // 2.5 is 0.5 from both 2.0 and 3.0 → outside 0.3
        assert_eq!(nearest_onset_within(&onsets, 2.5, 0.3), None);
        // empty list
        assert_eq!(nearest_onset_within(&[], 1.0, 0.3), None);
    }

    #[test]
    fn empty_onsets_falls_back_to_heuristic() {
        let src = r#"INT. ROOM - DAY

She sits.

INT. STREET - DAY

Chaos.
"#;
        let s = parse(src).unwrap();
        let v = baseline_velocity(15.0);
        let sb_plain = plan_from_screenplay(&s, &v, "x.fountain", "v.json", 30, [1920, 1080]);
        let sb_empty = plan_from_screenplay_with_onsets(
            &s,
            &v,
            "x.fountain",
            "v.json",
            30,
            [1920, 1080],
            Some(&[]),
        );
        assert_eq!(sb_plain.shots.len(), sb_empty.shots.len());
        for (a, b) in sb_plain.shots.iter().zip(sb_empty.shots.iter()) {
            assert!((a.start_secs - b.start_secs).abs() < 1e-5);
            assert!((a.duration_secs - b.duration_secs).abs() < 1e-5);
        }
    }

    #[test]
    fn single_onset_snaps_one_scene_boundary() {
        // Two scenes — second scene's heuristic boundary lands somewhere
        // around the first scene's total. We pick an onset ~0.2s off to
        // force a snap.
        let src = r#"INT. ROOM - DAY

She sits.

INT. STREET - DAY

Chaos.
"#;
        let s = parse(src).unwrap();
        let v = baseline_velocity(15.0);
        let sb_plain = plan_from_screenplay(&s, &v, "x.fountain", "v.json", 30, [1920, 1080]);
        let scene2_first = sb_plain.scenes[1].first_shot;
        let original = sb_plain.shots[scene2_first].start_secs;
        let onset = original + 0.15;
        let sb_snapped = plan_from_screenplay_with_onsets(
            &s,
            &v,
            "x.fountain",
            "v.json",
            30,
            [1920, 1080],
            Some(&[onset]),
        );
        let new_start = sb_snapped.shots[scene2_first].start_secs;
        assert!(
            (new_start - onset).abs() < 1e-4,
            "scene-2 start should snap to onset {onset}, got {new_start}",
        );
        // Total duration preserved.
        assert!((sb_snapped.duration_secs - sb_plain.duration_secs).abs() < 1e-4);
    }

    #[test]
    fn onsets_too_sparse_leaves_unmatched_scenes_alone() {
        let src = r#"INT. ROOM - DAY

She sits.

INT. STREET - DAY

Chaos.

INT. BAR - NIGHT

Loud.
"#;
        let s = parse(src).unwrap();
        let v = baseline_velocity(20.0);
        let sb_plain = plan_from_screenplay(&s, &v, "x.fountain", "v.json", 30, [1920, 1080]);
        // Place a single onset near scene 1's boundary only.
        let scene2_first = sb_plain.scenes[1].first_shot;
        let onset = sb_plain.shots[scene2_first].start_secs + 0.05;
        let sb_snapped = plan_from_screenplay_with_onsets(
            &s,
            &v,
            "x.fountain",
            "v.json",
            30,
            [1920, 1080],
            Some(&[onset]),
        );
        // Scene-1→2 boundary should snap.
        assert!(
            (sb_snapped.shots[scene2_first].start_secs - onset).abs() < 1e-4,
        );
        // Scene-2→3 boundary should NOT have moved (no onset within tolerance).
        let scene3_first = sb_plain.scenes[2].first_shot;
        let plain_scene3_start = sb_plain.shots[scene3_first].start_secs;
        let snapped_scene3_start = sb_snapped.shots[scene3_first].start_secs;
        // The delta from scene-2 snap propagates to all scene-2 shots (which
        // includes the establishing shot at scene_3.first_shot - 1? No —
        // scene-3 has its own shots starting at scene3_first). Scene-3 shots
        // are NOT shifted because we only shift shots inside the snapped
        // scene's range. So scene-3 start equals plain scene-3 start.
        assert!(
            (snapped_scene3_start - plain_scene3_start).abs() < 1e-4,
            "scene-3 should be untouched: plain={plain_scene3_start} snapped={snapped_scene3_start}",
        );
    }

    #[test]
    fn scene_with_two_speakers_gets_action_line() {
        let src = r#"INT. ROOM - DAY

ALICE
Hi.

BOB
Hi back.
"#;
        let s = parse(src).unwrap();
        let v = baseline_velocity(10.0);
        let sb = plan_from_screenplay(&s, &v, "x.fountain", "v.json", 30, [1920, 1080]);
        let al = sb.scenes[0].action_line.as_ref().expect("action line");
        assert_eq!(al.labels, vec!["ALICE".to_string(), "BOB".to_string()]);
    }

    /// Load-bearing wb-cx08 wiring test: a screenplay with two
    /// characters (ALEX + DANA) plus a `CharacterRefs` that only
    /// defines ALEX should produce exactly one shot with `character_ref
    /// = Some("ALEX")` routed through `Generation::RefConditioned`. The
    /// DANA dialogue beat stays on `StockSearch`.
    #[test]
    fn planner_routes_matched_character_through_ref_conditioned() {
        use crate::clipref::character::{CharacterRef, CharacterRefs, CharacterType};

        let src = r#"INT. KITCHEN - DAY

ALEX
Coffee?

DANA
Please.
"#;
        let s = parse(src).unwrap();
        let v = baseline_velocity(15.0);
        let mut characters = CharacterRefs::new();
        characters.insert(CharacterRef {
            name: "ALEX".to_string(),
            reference_images: vec!["./alex.jpg".to_string()],
            character_type: CharacterType::FullBody,
        });
        let sb = plan_from_screenplay_full(
            &s,
            &v,
            "x.fountain",
            "v.json",
            30,
            [1920, 1080],
            None,
            Some(&characters),
        );

        // Collect dialogue shots — the ones with audio_ref set.
        let alex_shots: Vec<_> = sb
            .shots
            .iter()
            .filter(|sh| sh.audio_ref.as_deref().map(|a| a.contains("ALEX")).unwrap_or(false))
            .collect();
        assert_eq!(alex_shots.len(), 1, "expected one ALEX dialogue shot");
        let alex = alex_shots[0];
        assert_eq!(alex.character_ref.as_deref(), Some("ALEX"));
        match &alex.generation {
            Generation::RefConditioned {
                character_ref,
                backend,
                ..
            } => {
                assert_eq!(character_ref, "ALEX");
                assert_eq!(backend, "fal-veo3-ref");
            }
            other => panic!("expected RefConditioned, got {other:?}"),
        }

        let dana_shots: Vec<_> = sb
            .shots
            .iter()
            .filter(|sh| sh.audio_ref.as_deref().map(|a| a.contains("DANA")).unwrap_or(false))
            .collect();
        assert_eq!(dana_shots.len(), 1, "expected one DANA dialogue shot");
        let dana = dana_shots[0];
        assert!(
            dana.character_ref.is_none(),
            "DANA isn't in the refs map, should not be ref-conditioned: {:?}",
            dana.character_ref,
        );
        assert!(
            matches!(dana.generation, Generation::StockSearch { .. }),
            "unmatched character should stay on StockSearch",
        );
    }

    /// Load-bearing wb-jwnk test: a screenplay with one Dialogue line
    /// and one ECU-hands cutaway, with both `FullBody` and `Hands`
    /// refs defined for ALEX. Dialogue must route through `FullBody`;
    /// the ECU Action must route through `Hands`.
    #[test]
    fn planner_routes_ecu_hand_cutaway_to_hands_ref() {
        use crate::clipref::character::{CharacterRef, CharacterRefs, CharacterType};

        let src = r#"INT. KITCHEN - DAY

ALEX
Try it.

ALEX's hands twist the cap off.
"#;
        let s = parse(src).unwrap();
        let v = baseline_velocity(15.0);
        let mut characters = CharacterRefs::new();
        characters.insert(CharacterRef {
            name: "ALEX".to_string(),
            reference_images: vec!["./alex-face.jpg".to_string()],
            character_type: CharacterType::FullBody,
        });
        characters.insert(CharacterRef {
            name: "ALEX".to_string(),
            reference_images: vec!["./alex-hands.jpg".to_string()],
            character_type: CharacterType::Hands,
        });
        let sb = plan_from_screenplay_full(
            &s,
            &v,
            "x.fountain",
            "v.json",
            30,
            [1920, 1080],
            None,
            Some(&characters),
        );

        // Dialogue shot — `audio_ref` is set for dialogue.
        let dlg = sb
            .shots
            .iter()
            .find(|sh| sh.audio_ref.is_some())
            .expect("expected one dialogue shot");
        assert_eq!(dlg.character_ref.as_deref(), Some("ALEX"));
        assert_eq!(dlg.character_ref_type, Some(CharacterType::FullBody));
        match &dlg.generation {
            Generation::RefConditioned { backend, .. } => {
                assert_eq!(backend, "fal-veo3-ref");
            }
            other => panic!("dialogue should be RefConditioned, got {other:?}"),
        }

        // ECU-hands shot — find it by shot_type==Ecu.
        let ecu = sb
            .shots
            .iter()
            .find(|sh| sh.shot_type == ShotType::Ecu)
            .expect("expected one ECU shot");
        assert_eq!(ecu.character_ref.as_deref(), Some("ALEX"));
        assert_eq!(
            ecu.character_ref_type,
            Some(CharacterType::Hands),
            "ECU cutaway should pick the Hands ref, not the FullBody ref",
        );
        match &ecu.generation {
            Generation::RefConditioned {
                character_ref,
                backend,
                ..
            } => {
                assert_eq!(character_ref, "ALEX");
                assert_eq!(backend, "fal-veo3-ref");
            }
            other => panic!("ECU should be RefConditioned, got {other:?}"),
        }
    }

    #[test]
    fn ecu_hand_cutaway_falls_back_to_full_body_when_hands_missing() {
        use crate::clipref::character::{CharacterRef, CharacterRefs, CharacterType};

        let src = r#"INT. KITCHEN - DAY

ALEX
Try it.

ALEX's hands twist the cap off.
"#;
        let s = parse(src).unwrap();
        let v = baseline_velocity(15.0);
        let mut characters = CharacterRefs::new();
        // Only full-body defined — no hands ref.
        characters.insert(CharacterRef {
            name: "ALEX".to_string(),
            reference_images: vec!["./alex-face.jpg".to_string()],
            character_type: CharacterType::FullBody,
        });
        let sb = plan_from_screenplay_full(
            &s,
            &v,
            "x.fountain",
            "v.json",
            30,
            [1920, 1080],
            None,
            Some(&characters),
        );
        let ecu = sb.shots.iter().find(|sh| sh.shot_type == ShotType::Ecu).unwrap();
        // Should still route to ALEX, but via FullBody — the verifier
        // is responsible for warning the agent to add a hands ref.
        assert_eq!(ecu.character_ref.as_deref(), Some("ALEX"));
        assert_eq!(ecu.character_ref_type, Some(CharacterType::FullBody));
    }

    #[test]
    fn product_noun_bumps_precedence_to_product_hands() {
        use crate::clipref::character::{CharacterRef, CharacterRefs, CharacterType};

        // Paragraph mentions "bottle" → ProductHands wins over Hands.
        let src = r#"INT. KITCHEN - DAY

DANA
Watch.

DANA's fingers grip the bottle cap.
"#;
        let s = parse(src).unwrap();
        let v = baseline_velocity(15.0);
        let mut characters = CharacterRefs::new();
        characters.insert(CharacterRef {
            name: "DANA".to_string(),
            reference_images: vec!["./dana-hands.jpg".to_string()],
            character_type: CharacterType::Hands,
        });
        characters.insert(CharacterRef {
            name: "DANA".to_string(),
            reference_images: vec!["./dana-product-hands.jpg".to_string()],
            character_type: CharacterType::ProductHands,
        });
        let sb = plan_from_screenplay_full(
            &s,
            &v,
            "x.fountain",
            "v.json",
            30,
            [1920, 1080],
            None,
            Some(&characters),
        );
        let ecu = sb.shots.iter().find(|sh| sh.shot_type == ShotType::Ecu).unwrap();
        assert_eq!(ecu.character_ref_type, Some(CharacterType::ProductHands));
    }

    #[test]
    fn looks_like_hand_cutaway_heuristic() {
        assert!(looks_like_hand_cutaway("She twists the cap with her fingers."));
        assert!(looks_like_hand_cutaway("ECU: hand on the wheel."));
        assert!(looks_like_hand_cutaway("Cutaway: he grips the door."));
        // Long, narrative paragraph without explicit ECU cue → not a
        // cutaway even if it mentions "hand".
        let long = "He waved his hand absently while the parade rolled past the storefront and the children chased after the trumpet player, whose tassels swayed.";
        assert!(!looks_like_hand_cutaway(long));
        // No hand verb at all → no cutaway.
        assert!(!looks_like_hand_cutaway("She walks to the door."));
    }

    #[test]
    fn planner_without_characters_keeps_legacy_behavior() {
        // Sanity: passing None for characters must produce exactly the
        // same plan as the legacy `plan_from_screenplay` entrypoint.
        let src = r#"INT. ROOM - DAY

ALEX
Hello.
"#;
        let s = parse(src).unwrap();
        let v = baseline_velocity(10.0);
        let sb_legacy = plan_from_screenplay(&s, &v, "x.fountain", "v.json", 30, [1920, 1080]);
        let sb_full = plan_from_screenplay_full(
            &s,
            &v,
            "x.fountain",
            "v.json",
            30,
            [1920, 1080],
            None,
            None,
        );
        assert_eq!(sb_legacy.shots.len(), sb_full.shots.len());
        for shot in &sb_full.shots {
            assert!(shot.character_ref.is_none());
            assert!(
                !matches!(shot.generation, Generation::RefConditioned { .. }),
                "no character refs → no RefConditioned shots",
            );
        }
    }
}
