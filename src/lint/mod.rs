//! `wavelet lint` — layout-walk lint rules that surface authoring
//! defects before a paid render. v1 ships one rule (`safe-zone`); the
//! design doc lists three more (glyph-clip, text-readability,
//! color-grade-coherence) shipped in follow-ups.

pub mod glyph_clip;
pub mod report;
pub mod safe_zone;
pub mod safe_zones;
