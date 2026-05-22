//! Google AI Studio adapters — Veo 3.x (txt2vid / img2vid), Nano Banana
//! 3 (reference-conditioned still gen), and Lyria 3 (music gen). These
//! are the Google-direct defaults for the `commercial` pipeline's
//! `tier_policy` (image / video / music). TTS stays on ElevenLabs — see
//! `commercial.yaml`.
//!
//! Fal does not host Veo (probed 2026-05-19; every `fal-ai/veo*` and
//! `fal-ai/google/veo*` path returned 404). The Google AI Studio
//! generative-language API hosts Veo 3.1 in the `*_preview` track:
//!
//! - `models/veo-3.1-generate-preview`
//! - `models/veo-3.1-fast-generate-preview`
//! - `models/veo-3.1-lite-generate-preview`
//!
//! Auth is a query-param API key (`?key=…`). Generation is a
//! long-running operation: POST returns `operations/<id>`; clients poll
//! `GET …/<operation>` until `done: true` and then fetch the video bytes
//! from the response URI.
//!
//! Env var: `GOOGLE_API_KEY`.

pub mod client;
pub mod gemini_tts;
pub mod lyria;
pub mod nano_banana;
pub mod veo;

pub use client::{GoogleAiClient, GOOGLE_API_KEY_ENV};
pub use gemini_tts::{
    GeminiTtsAdapter, MODEL_GEMINI_TTS, PROVIDER as GEMINI_TTS_PROVIDER,
};
pub use lyria::{
    GoogleLyriaAdapter, LyriaModel, MODEL_LYRIA_3_CLIP, MODEL_LYRIA_3_PRO,
    PRICE_PER_SEC_CLIP_USD, PRICE_PER_SEC_PRO_USD, PROVIDER_CLIP as LYRIA_PROVIDER_CLIP,
    PROVIDER_PRO as LYRIA_PROVIDER_PRO,
};
pub use nano_banana::{
    GoogleNanoBanana3Adapter, MAX_REF_IMAGES as NANO_BANANA_MAX_REFS, MODEL as NANO_BANANA_MODEL,
    PRICE_PER_CALL_USD as NANO_BANANA_PRICE_USD, PROVIDER as NANO_BANANA_PROVIDER,
};
pub use veo::{
    GoogleVeoAdapter, VeoModel, MODEL_VEO_3_1_FAST, MODEL_VEO_3_1_LITE, MODEL_VEO_3_1_STANDARD,
    PRICE_PER_SECOND_FAST_USD, PRICE_PER_SECOND_LITE_USD, PRICE_PER_SECOND_STANDARD_USD,
};
