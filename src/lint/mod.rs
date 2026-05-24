//! `wavelet lint` — layout-walk lint rules that surface authoring
//! defects before a paid render. v1 shipped `safe-zone`; subsequent
//! drops added `glyph-clip` and `color-grade-coherence`. The design
//! doc lists `text-readability` as the next rule to land.
//! `baked-text-ocr` (feature-gated) catches AI-generated garbled text
//! in Veo video frames.

pub mod audio_presence;
pub mod baked_text_ocr;
pub mod color_grade;
pub mod glyph_clip;
pub mod hallucinated_attrs;
pub mod layout_axis;
pub mod mp4_frames;
pub mod report;
pub mod safe_zone;
pub mod safe_zones;
pub mod static_frame_trim;
pub mod text_on_subject;
pub mod text_readability;
pub mod text_readability_contrast;
