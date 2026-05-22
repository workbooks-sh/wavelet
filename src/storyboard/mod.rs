//! Storyboard — the per-shot plan that sits between screenplay+velocity
//! and the final `comp.json` render. Encodes shot vocabulary, generation
//! manifests, continuity annotations, and expected verification gates.
//!
//! Phase 4 of the screenplay-to-MP4 epic (wb-iv3c → wb-ybpq). See
//! `docs/research/screenplay-to-mp4-prd.md` §5–6.
//!
//! The format is plain JSON. `wavelet storyboard plan` produces a draft
//! heuristically (no LLM); the agent fills in the gaps via a separate
//! skill. `wavelet storyboard verify` runs structural lint and (in deep
//! mode) every per-shot verification gate.

use serde::{Deserialize, Serialize};

pub mod attributes;
pub mod plan;
pub mod verify;

pub use attributes::{ShotAttributes, ValidationError};
pub use plan::{plan_from_screenplay, plan_from_screenplay_with_onsets};
pub use verify::{verify_storyboard, StoryboardFinding, StoryboardLevel};

/// Top-level storyboard. Holds refs to upstream artifacts (screenplay,
/// velocity, voices, style-bible) by path so the storyboard remains
/// readable and the artifacts stay single-source-of-truth.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Storyboard {
    /// Schema version. Bump on breaking changes.
    pub version: u32,
    /// Total duration in seconds.
    pub duration_secs: f32,
    /// Frame rate the eventual render targets. Used by downstream
    /// orchestrators to derive frame-accurate ranges from `start_secs`.
    pub fps: u32,
    /// Output resolution (e.g. `[1920, 1080]`).
    pub resolution: [u32; 2],
    /// Path to the Fountain screenplay (or its AST JSON dump). Relative
    /// to the storyboard file by convention.
    pub screenplay_ref: String,
    /// Path to the velocity profile JSON.
    pub velocity_ref: String,
    /// Path to the voices/casting JSON (Phase 3 artifact). Optional.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub voices_ref: Option<String>,
    /// Path to the style-bible TOML (palette, fonts, motion language).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub style_bible_ref: Option<String>,
    /// Scene-level annotations (one per screenplay scene heading).
    pub scenes: Vec<SceneAnnotation>,
    /// Per-shot plan. Order is render order.
    pub shots: Vec<Shot>,
}

/// Scene-level metadata. Carries the 180°-rule action line and the
/// originating slugline.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SceneAnnotation {
    /// Sequential 0-based scene index in the screenplay.
    pub scene_index: usize,
    /// The slugline as written (e.g. `INT. KITCHEN - DAY`).
    pub slugline: String,
    /// First shot's index in `Storyboard.shots` that belongs to this
    /// scene. Used to range-scan continuity across the scene.
    pub first_shot: usize,
    /// Count of shots in this scene.
    pub shot_count: usize,
    /// 180° action-line annotation, when applicable (e.g. a dialogue
    /// scene with two characters). Used by the continuity gate.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action_line: Option<ActionLine>,
}

/// Two-character action line for the 180° rule. `from`/`to` are
/// normalized to the frame coordinate system (0..1 across width, 0..1
/// down height).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActionLine {
    /// Source point of the line (typically character A's position).
    pub from: [f32; 2],
    /// Sink point of the line (typically character B's position).
    pub to: [f32; 2],
    /// Optional labels for the two endpoints (character names).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub labels: Vec<String>,
}

/// One shot in the storyboard.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Shot {
    /// Stable shot id. Format: `shot-<scene>-<index-in-scene>` by
    /// convention (so it stays stable when shots are reordered within a
    /// scene).
    pub id: String,
    /// 0-based index in the storyboard's ordered shot list.
    pub shot_index: usize,
    /// Which scene this shot belongs to (index into `Storyboard.scenes`).
    pub scene_index: usize,
    /// Index of the originating screenplay element in the AST's
    /// `elements` array. Lets the verifier cross-check shot↔script.
    pub screenplay_element_index: usize,
    /// Start time in seconds from composition start.
    pub start_secs: f32,
    /// Duration in seconds.
    pub duration_secs: f32,
    /// Shot-type vocabulary entry.
    pub shot_type: ShotType,
    /// Optional camera framing (over-shoulder, point-of-view, etc.).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub framing: Option<Framing>,
    /// Camera movement vocabulary.
    pub camera_movement: CameraMovement,
    /// Which side of the action line the camera sits on. Required for
    /// 180°-rule checks; `Center` is allowed for establishing shots.
    pub camera_side: CameraSide,
    /// Free-form subject label — typically a character name or a
    /// concrete noun ("car", "skyline"). Used in img2vid prompts and
    /// in continuity output.
    pub subject: String,
    /// How this shot's footage is produced.
    pub generation: Generation,
    /// Transition INTO this shot (i.e. how the previous shot ends).
    /// `None` for shot 0.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transition_in: Option<ShotTransition>,
    /// Motion vector at shot exit, in pixels/second. Used by the
    /// continuity gate to score 180°-friendly cuts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub motion_vector_exit: Option<[f32; 2]>,
    /// Motion vector at shot entry, in pixels/second.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub motion_vector_entry: Option<[f32; 2]>,
    /// Optional audio asset path — typically a VO line.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audio_ref: Option<String>,
    /// List of render-query gates the verifier runs against this shot
    /// after render. The CLI translates these into `wavelet query` calls.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub expected_checks: Vec<ExpectedCheck>,
    /// Predecessor shot id for **inter-shot frame chaining** (wb-6msu).
    /// When set, the executor extracts the prev shot's last frame and
    /// feeds it as `start_image_url` to this shot's i2v call (default
    /// chain-from-end semantics). Spans the cut between shots so the
    /// transition stops looking like a fresh universe.
    ///
    /// Only meaningful for `Generation::Img2Vid` shots on backends that
    /// support image-conditioned generation (Kling O1, future Veo 3.1).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prev_shot_id: Option<String>,
    /// L-Storyboard structured attributes (arXiv 2505.12237). When
    /// `Some`, `shot_prompt_fragment` calls `attributes.to_prompt()`
    /// instead of the legacy `Generation`-payload + shot-type-label
    /// assembly. `None` keeps the legacy path so existing storyboard
    /// JSON keeps working unchanged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attributes: Option<ShotAttributes>,
}

/// Shot-type vocabulary. Matches industry convention.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ShotType {
    /// Extreme close-up — eye / detail.
    Ecu,
    /// Close-up — face / object.
    Cu,
    /// Medium shot — waist up.
    Ms,
    /// Medium-wide shot — full body.
    Mws,
    /// Wide shot — subject + environment.
    Ws,
    /// Extreme wide shot — subject tiny in landscape.
    Ews,
}

/// Optional framing modifier on top of the shot type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Framing {
    /// Over-the-shoulder.
    Ots,
    /// Point-of-view.
    Pov,
}

/// Camera-movement vocabulary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CameraMovement {
    /// Camera fixed.
    Static,
    /// Push in.
    Push,
    /// Pull out.
    Pull,
    /// Lateral truck.
    Dolly,
    /// Whip pan.
    Whip,
    /// Crane up/down.
    Crane,
    /// Handheld jitter.
    Handheld,
}

/// Which side of the action line the camera sits on, for 180°-rule
/// verification.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CameraSide {
    /// Camera on the "left" half of the action line.
    Left,
    /// Camera on the "right" half.
    Right,
    /// Camera dead-center (establishing / overhead / closeups without
    /// directional commitment).
    Center,
}

/// How the shot is produced. Each variant carries its own manifest.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Generation {
    /// Stock-footage lookup (e.g. Pexels).
    StockSearch {
        /// Search query.
        query: String,
        /// Optional preferred orientation / aspect.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        orientation: Option<String>,
        /// Resolved asset path after a download. None until lookup runs.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        resolved_path: Option<String>,
    },
    /// Image-to-video (e.g. Runway).
    Img2Vid {
        /// Source still path.
        still: String,
        /// Motion prompt describing intended movement.
        motion_prompt: String,
        /// Backend identifier (`runway`, `kling`, `pika`, …).
        backend: String,
        /// Random seed; bumps reproducibility.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        seed: Option<u64>,
        /// Resolved output path after gen runs.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        resolved_path: Option<String>,
    },
    /// Text-to-video (e.g. Veo, Sora, Kling).
    Txt2Vid {
        /// Generation prompt.
        prompt: String,
        /// Backend identifier.
        backend: String,
        /// Random seed; bumps reproducibility.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        seed: Option<u64>,
        /// Resolved output path after gen runs.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        resolved_path: Option<String>,
    },
    /// ControlNet-style image gen + later animation. Phase 5 detail.
    Controlnet {
        /// Text prompt.
        prompt: String,
        /// ControlNet conditioning kind (`canny`, `pose`, `depth`, …).
        condition_kind: String,
        /// Path to the conditioning image.
        condition_image: String,
        /// Backend identifier (Replicate / Fal.ai / local SD).
        backend: String,
        /// Random seed.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        seed: Option<u64>,
        /// Resolved output path after gen runs.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        resolved_path: Option<String>,
    },
    /// Native render — HTML scene driven through wavelet's renderer.
    Native {
        /// Path to the scene's HTML file.
        html: String,
    },
}

/// Shot-level transition, lifted from the Fountain transition vocab.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShotTransition {
    /// Transition kind. Reuses the Fountain transition vocabulary so a
    /// storyboard maps cleanly back to the screenplay.
    pub kind: fountain::TransitionKind,
    /// Duration in seconds for non-cut transitions. None for hard cuts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_secs: Option<f32>,
    /// Optional direction hint (e.g. `whip-pan-left` or `whip-pan-right`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub direction: Option<String>,
    /// Per-J/L-cut lead/trail audio seconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audio_lead_secs: Option<f32>,
}

/// A render-query gate the verifier runs against this shot once it's
/// rendered. Maps onto `wavelet query` flags.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "gate", rename_all = "snake_case")]
pub enum ExpectedCheck {
    /// Subject must be visible (non-zero pixels in the expected area)
    /// at the mid-frame of the shot.
    SubjectVisible {
        /// CSS selector that should resolve to the subject element.
        selector: String,
    },
    /// Text overlay must be present and OCR-readable.
    TextVisible {
        /// Expected text content.
        text: String,
        /// Optional selector to crop OCR (much faster).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        within_selector: Option<String>,
    },
    /// Subject must lie inside the title-safe area.
    InSafeArea {
        /// CSS selector for the element to check.
        selector: String,
        /// Safe-area inset fraction. Default 0.1 (10%).
        #[serde(default = "default_safe_inset")]
        inset: f32,
    },
    /// Mean color of an element should be near a target hex.
    ColorIn {
        /// CSS selector.
        selector: String,
        /// Target hex (e.g. `#ffffff`).
        target_hex: String,
        /// Max ΔE allowed. Default 5.
        #[serde(default = "default_max_de")]
        max_de: f32,
    },
    /// Camera-side must match the scene's action line (180° rule).
    OnAllowedSide,
    /// Motion vector must continue from the previous shot within
    /// `+/- tolerance_degrees`.
    MotionContinuous {
        /// Allowed deviation in degrees. Default 30.
        #[serde(default = "default_motion_tol")]
        tolerance_degrees: f32,
    },
}

fn default_safe_inset() -> f32 {
    0.1
}
fn default_max_de() -> f32 {
    5.0
}
fn default_motion_tol() -> f32 {
    30.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_through_json() {
        let sb = Storyboard {
            version: 1,
            duration_secs: 30.0,
            fps: 30,
            resolution: [1920, 1080],
            screenplay_ref: "script.fountain".into(),
            velocity_ref: "velocity.json".into(),
            voices_ref: None,
            style_bible_ref: None,
            scenes: vec![SceneAnnotation {
                scene_index: 0,
                slugline: "EXT. ROAD - DAY".into(),
                first_shot: 0,
                shot_count: 2,
                action_line: Some(ActionLine {
                    from: [0.2, 0.5],
                    to: [0.8, 0.5],
                    labels: vec!["ALICE".into(), "BOB".into()],
                }),
            }],
            shots: vec![Shot {
                id: "shot-0-0".into(),
                shot_index: 0,
                scene_index: 0,
                screenplay_element_index: 1,
                start_secs: 0.0,
                duration_secs: 3.0,
                shot_type: ShotType::Ws,
                framing: None,
                camera_movement: CameraMovement::Static,
                camera_side: CameraSide::Left,
                subject: "ALICE".into(),
                generation: Generation::StockSearch {
                    query: "highway desert".into(),
                    orientation: Some("landscape".into()),
                    resolved_path: None,
                },
                transition_in: None,
                motion_vector_exit: Some([10.0, 0.0]),
                motion_vector_entry: None,
                audio_ref: None,
                expected_checks: vec![
                    ExpectedCheck::SubjectVisible {
                        selector: "#alice".into(),
                    },
                    ExpectedCheck::OnAllowedSide,
                ],
                prev_shot_id: None,
                attributes: None,
            }],
        };

        let json = serde_json::to_string(&sb).unwrap();
        let back: Storyboard = serde_json::from_str(&json).unwrap();
        assert_eq!(back.shots.len(), 1);
        assert_eq!(back.shots[0].shot_type, ShotType::Ws);
        assert_eq!(back.shots[0].expected_checks.len(), 2);
    }

    #[test]
    fn prev_shot_id_round_trips() {
        let body = r#"{
            "id": "shot-0-1",
            "shot_index": 1,
            "scene_index": 0,
            "screenplay_element_index": 2,
            "start_secs": 3.0,
            "duration_secs": 3.0,
            "shot_type": "ms",
            "camera_movement": "static",
            "camera_side": "right",
            "subject": "BOB",
            "generation": {"kind": "img2_vid", "still": "b.png", "motion_prompt": "push in", "backend": "fal-kling-o1"},
            "prev_shot_id": "shot-0-0"
        }"#;
        let shot: Shot = serde_json::from_str(body).unwrap();
        assert_eq!(shot.prev_shot_id.as_deref(), Some("shot-0-0"));
        let json = serde_json::to_string(&shot).unwrap();
        assert!(json.contains("\"prev_shot_id\":\"shot-0-0\""));
    }

    #[test]
    fn prev_shot_id_omitted_when_none() {
        let body = r#"{
            "id": "shot-0-0",
            "shot_index": 0,
            "scene_index": 0,
            "screenplay_element_index": 0,
            "start_secs": 0.0,
            "duration_secs": 3.0,
            "shot_type": "ws",
            "camera_movement": "static",
            "camera_side": "center",
            "subject": "ALICE",
            "generation": {"kind": "native", "html": "scene.html"}
        }"#;
        let shot: Shot = serde_json::from_str(body).unwrap();
        assert!(shot.prev_shot_id.is_none());
        assert!(shot.attributes.is_none());
        let json = serde_json::to_string(&shot).unwrap();
        assert!(!json.contains("prev_shot_id"));
        assert!(!json.contains("attributes"));
    }

    #[test]
    fn legacy_shot_json_without_attributes_still_parses() {
        let body = r#"{
            "id": "shot-0-0",
            "shot_index": 0,
            "scene_index": 0,
            "screenplay_element_index": 0,
            "start_secs": 0.0,
            "duration_secs": 3.0,
            "shot_type": "ws",
            "camera_movement": "static",
            "camera_side": "center",
            "subject": "ALICE",
            "generation": {"kind": "txt2_vid", "prompt": "a wide shot", "backend": "veo"}
        }"#;
        let shot: Shot = serde_json::from_str(body).unwrap();
        assert!(shot.attributes.is_none());
    }

    #[test]
    fn shot_with_attributes_round_trips() {
        let attrs = ShotAttributes {
            subject: "the product".into(),
            action: "rotates slowly".into(),
            scene: "on a black turntable".into(),
            camera: "MS 100mm macro, eye level".into(),
            lens: "shallow DoF, sharp falloff".into(),
            lighting: "key right, soft fill, rim from behind".into(),
            style: "luxury product, glossy".into(),
        };
        let body = serde_json::json!({
            "id": "shot-0-0",
            "shot_index": 0,
            "scene_index": 0,
            "screenplay_element_index": 0,
            "start_secs": 0.0,
            "duration_secs": 3.0,
            "shot_type": "ms",
            "camera_movement": "static",
            "camera_side": "center",
            "subject": "product",
            "generation": {"kind": "txt2_vid", "prompt": "p", "backend": "veo"},
            "attributes": attrs,
        });
        let shot: Shot = serde_json::from_value(body).unwrap();
        assert_eq!(shot.attributes.as_ref().unwrap(), &attrs);
        let json = serde_json::to_string(&shot).unwrap();
        assert!(json.contains("\"attributes\""));
        let back: Shot = serde_json::from_str(&json).unwrap();
        assert_eq!(back.attributes.unwrap(), attrs);
    }

    #[test]
    fn generation_variants_round_trip() {
        for g in [
            Generation::StockSearch {
                query: "a".into(),
                orientation: None,
                resolved_path: None,
            },
            Generation::Img2Vid {
                still: "s.jpg".into(),
                motion_prompt: "push in".into(),
                backend: "runway".into(),
                seed: Some(42),
                resolved_path: None,
            },
            Generation::Txt2Vid {
                prompt: "p".into(),
                backend: "veo".into(),
                seed: None,
                resolved_path: None,
            },
            Generation::Controlnet {
                prompt: "p".into(),
                condition_kind: "canny".into(),
                condition_image: "c.png".into(),
                backend: "replicate".into(),
                seed: None,
                resolved_path: None,
            },
            Generation::Native { html: "scene.html".into() },
        ] {
            let json = serde_json::to_string(&g).unwrap();
            let _: Generation = serde_json::from_str(&json).unwrap();
        }
    }
}
