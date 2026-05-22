//! Enums for clip-ref `kind` and `edit_kind`. Kebab-case on the wire.

use serde::{Deserialize, Serialize};

/// What a clip-ref represents.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ClipKind {
    /// Image generation (Nano Banana, generic stills).
    Still,
    /// Generated video clip (Veo, Fal Wan, Kling).
    Shot,
    /// Music generation (Lyria, ElevenLabs).
    Music,
    /// Scene-still generation — a Still scoped to a screenplay scene.
    SceneStill,
    /// Dialogue text-to-speech.
    Tts,
    /// Captions / word timing.
    Caption,
    /// One scene of a fountain screenplay.
    ScreenplayScene,
    /// Hand-authored HTML/CSS overlay (text card, lower-third, etc.).
    Overlay,
}

impl ClipKind {
    /// Lowercase kebab-cased name used in default_path. Matches the
    /// `kebab-case` serde rename so paths stay round-trip-stable with
    /// the YAML representation.
    pub fn as_kebab(self) -> &'static str {
        match self {
            ClipKind::Still => "still",
            ClipKind::Shot => "shot",
            ClipKind::Music => "music",
            ClipKind::SceneStill => "scene-still",
            ClipKind::Tts => "tts",
            ClipKind::Caption => "caption",
            ClipKind::ScreenplayScene => "screenplay-scene",
            ClipKind::Overlay => "overlay",
        }
    }
}

/// What kind of edit produced a derived clip-ref.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum EditKind {
    /// Face-refinement pass on a shot/still.
    RefineFace,
    /// Upscaler pass (e.g. SeedVR, Topaz).
    Upscale,
    /// Nano Banana inpaint/edit on a still.
    NanoBananaEdit,
    /// Re-run the same producer with the same prompt (variance sample).
    Regenerate,
    /// Hand edit by a human.
    Manual,
}
