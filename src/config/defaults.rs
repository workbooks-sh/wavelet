//! Hardcoded fallback values for the config cascade.
//!
//! These are the last layer — workdir `wavelet.config.toml` and the
//! user-global `~/.wavelet/config.toml` both override them, and an
//! explicit CLI flag overrides everything.
//!
//! Picks per wb-e90g: Google-direct on image + video where it's
//! available; ElevenLabs for voice. MusicGen / Suno are intentionally
//! not the default (licensing).

/// Default text-to-video and image-to-video backend.
pub const VIDEO_BACKEND: &str = "veo";

/// Default reference-conditioned music backend.
pub const MUSIC_BACKEND: &str = "lyria-pro";

/// Default text-to-image backend.
pub const IMAGE_BACKEND: &str = "google-nano-banana-3";

/// Default text-to-speech backend.
pub const TTS_BACKEND: &str = "elevenlabs";

/// Default aspect ratio for new video/image generations.
pub const ASPECT: &str = "16:9";

/// Default clip duration in seconds for new video generations.
pub const DURATION_SECS: f32 = 5.0;
