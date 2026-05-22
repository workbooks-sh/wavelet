//! L-Storyboard structured shot attributes.
//!
//! Source: arXiv 2505.12237 "L-Storyboard: Long-form Storyboard
//! Generation with LLMs." The paper argues that asking an LLM to write
//! free-form shot prose is brittle; constraining it to a fixed set of
//! typed slots produces prompts that are more accurate, more critique-
//! able per axis, and can be VLM-graded slot-by-slot.
//!
//! This module is the schema layer only — assembly + validation. The
//! LLM-as-creative-director step (wb-epk3, next wave) fills the slots
//! via JSON-schema-constrained generation; `to_prompt()` joins them
//! into the model-facing string.
//!
//! Back-compat: `Shot::attributes` is `Option`. `None` keeps the
//! pre-existing prompt-assembly path; `Some(_)` switches to L-Storyboard
//! assembly.
//!
//! # Slot vocabulary (all required when `Some`)
//!
//! | Slot     | Captures                                |
//! |----------|-----------------------------------------|
//! | subject  | what the shot is OF                     |
//! | action   | what's happening                        |
//! | scene    | where it is                             |
//! | camera   | shot framing (ECU, lens mm, angle)      |
//! | lens     | optical character (DoF, anamorphic, …) |
//! | lighting | direction + quality of light            |
//! | style    | aesthetic register                      |
//!
//! Unknown slots are explicitly filled with the literal token
//! `"unspecified"` rather than left empty — empty strings are a
//! validation error so the director can't silently drop slots.

use serde::{Deserialize, Serialize};

/// L-Storyboard structured shot attributes (arXiv 2505.12237).
///
/// All seven slots are required. The LLM director must explicitly mark
/// unknown slots `"unspecified"` rather than leaving them blank — see
/// [`ShotAttributes::validate`].
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ShotAttributes {
    /// What the shot is OF — concrete subject noun phrase.
    pub subject: String,
    /// What's happening within the frame — verb phrase, not setting.
    pub action: String,
    /// Where it is — location, time of day, environment.
    pub scene: String,
    /// Framing + viewpoint — shot type, focal length, angle.
    pub camera: String,
    /// Optical character — DoF, anamorphic, fringe, distortion.
    pub lens: String,
    /// Light direction + quality.
    pub lighting: String,
    /// Aesthetic register — film stock, color grade, reference world.
    pub style: String,
}

/// Validation outcomes for [`ShotAttributes::validate`].
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ValidationError {
    /// One of the seven slots was empty / whitespace-only.
    #[error("ShotAttributes slot `{slot}` is empty (use \"unspecified\" instead)")]
    EmptySlot {
        /// Name of the offending slot.
        slot: &'static str,
    },
}

impl ShotAttributes {
    /// Assemble the seven slots into the model-facing prompt string.
    ///
    /// Join order is fixed
    /// (subject → action → scene → camera → lens → lighting → style)
    /// so identical attribute sets produce identical prompts across
    /// runs — important for cache hits at the backend and for
    /// regression tests.
    ///
    /// The template reads as a real prompt rather than a tag dump:
    /// `"{subject}. {action}. {scene}. Shot: {camera}, {lens}.
    ///   Lighting: {lighting}. Style: {style}."`
    pub fn to_prompt(&self) -> String {
        format!(
            "{subject}. {action}. {scene}. Shot: {camera}, {lens}. \
             Lighting: {lighting}. Style: {style}.",
            subject = self.subject.trim(),
            action = self.action.trim(),
            scene = self.scene.trim(),
            camera = self.camera.trim(),
            lens = self.lens.trim(),
            lighting = self.lighting.trim(),
            style = self.style.trim(),
        )
    }

    /// Reject empty slots. The director must spell out `"unspecified"`
    /// when it doesn't know — silent gaps degrade the prompt without
    /// surfacing the omission.
    pub fn validate(&self) -> Result<(), ValidationError> {
        for (slot, value) in self.slots() {
            if value.trim().is_empty() {
                return Err(ValidationError::EmptySlot { slot });
            }
        }
        Ok(())
    }

    fn slots(&self) -> [(&'static str, &str); 7] {
        [
            ("subject", &self.subject),
            ("action", &self.action),
            ("scene", &self.scene),
            ("camera", &self.camera),
            ("lens", &self.lens),
            ("lighting", &self.lighting),
            ("style", &self.style),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cinematic() -> ShotAttributes {
        ShotAttributes {
            subject: "a 1968 Porsche 911 GT3 in racing yellow".into(),
            action: "idle, parked, engine off".into(),
            scene: "on a wet asphalt track at dawn".into(),
            camera: "ECU 50mm, low angle, 3/4 front".into(),
            lens: "anamorphic, shallow DoF, slight chromatic fringe".into(),
            lighting: "backlit by rising sun, mist-diffused".into(),
            style: "cinematic, A24-flavored, restrained color".into(),
        }
    }

    fn brutalist() -> ShotAttributes {
        ShotAttributes {
            subject: "a raw concrete monolith".into(),
            action: "stands motionless".into(),
            scene: "in an empty plaza at noon".into(),
            camera: "EWS 24mm, eye-level, centered".into(),
            lens: "deep DoF, rectilinear, no fringe".into(),
            lighting: "harsh overhead sun, sharp shadow".into(),
            style: "brutalist, desaturated, large-format stills".into(),
        }
    }

    fn editorial() -> ShotAttributes {
        ShotAttributes {
            subject: "a model in a wool overcoat".into(),
            action: "turns toward the lens".into(),
            scene: "in a tiled corridor under fluorescent strips".into(),
            camera: "MS 85mm, slight high angle".into(),
            lens: "creamy bokeh, fast prime, controlled vignette".into(),
            lighting: "key from camera-left, soft fill, cool kicker".into(),
            style: "editorial fashion, magazine print, Kodak Portra".into(),
        }
    }

    #[test]
    fn to_prompt_cinematic_snapshot() {
        let p = cinematic().to_prompt();
        assert_eq!(
            p,
            "a 1968 Porsche 911 GT3 in racing yellow. idle, parked, engine off. \
             on a wet asphalt track at dawn. Shot: ECU 50mm, low angle, 3/4 front, \
             anamorphic, shallow DoF, slight chromatic fringe. \
             Lighting: backlit by rising sun, mist-diffused. \
             Style: cinematic, A24-flavored, restrained color."
        );
    }

    #[test]
    fn to_prompt_brutalist_snapshot() {
        let p = brutalist().to_prompt();
        assert_eq!(
            p,
            "a raw concrete monolith. stands motionless. in an empty plaza at noon. \
             Shot: EWS 24mm, eye-level, centered, deep DoF, rectilinear, no fringe. \
             Lighting: harsh overhead sun, sharp shadow. \
             Style: brutalist, desaturated, large-format stills."
        );
    }

    #[test]
    fn to_prompt_editorial_snapshot() {
        let p = editorial().to_prompt();
        assert_eq!(
            p,
            "a model in a wool overcoat. turns toward the lens. \
             in a tiled corridor under fluorescent strips. \
             Shot: MS 85mm, slight high angle, creamy bokeh, fast prime, controlled vignette. \
             Lighting: key from camera-left, soft fill, cool kicker. \
             Style: editorial fashion, magazine print, Kodak Portra."
        );
    }

    #[test]
    fn to_prompt_join_order_is_stable() {
        let a = cinematic().to_prompt();
        let b = cinematic().to_prompt();
        assert_eq!(a, b);
    }

    #[test]
    fn validate_passes_when_all_slots_filled() {
        assert!(cinematic().validate().is_ok());
    }

    #[test]
    fn validate_catches_each_empty_slot() {
        let cases: [(&str, fn(&mut ShotAttributes)); 7] = [
            ("subject", |a| a.subject.clear()),
            ("action", |a| a.action.clear()),
            ("scene", |a| a.scene.clear()),
            ("camera", |a| a.camera.clear()),
            ("lens", |a| a.lens.clear()),
            ("lighting", |a| a.lighting.clear()),
            ("style", |a| a.style.clear()),
        ];
        for (slot, mutate) in cases {
            let mut a = cinematic();
            mutate(&mut a);
            let err = a.validate().unwrap_err();
            assert_eq!(err, ValidationError::EmptySlot { slot });
        }
    }

    #[test]
    fn validate_rejects_whitespace_only_slot() {
        let mut a = cinematic();
        a.lighting = "   \t\n".into();
        assert_eq!(
            a.validate().unwrap_err(),
            ValidationError::EmptySlot { slot: "lighting" }
        );
    }

    #[test]
    fn validate_accepts_unspecified_marker() {
        let mut a = cinematic();
        a.lens = "unspecified".into();
        assert!(a.validate().is_ok());
    }

    #[test]
    fn json_round_trips() {
        let a = cinematic();
        let s = serde_json::to_string(&a).unwrap();
        let back: ShotAttributes = serde_json::from_str(&s).unwrap();
        assert_eq!(a, back);
    }
}
