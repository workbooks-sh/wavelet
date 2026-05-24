//! Per-element shaped-glyph ink bounds, captured from Blitz's
//! `inline_layout_data` (a `parley::Layout`). This is what the
//! `glyph-clip` lint rule uses to fact-check from the visualization,
//! not from the layout bbox.
//!
//! Layout-bbox-based checks miss real visible clipping:
//!   - italic side-bearing overrun (the lean past the advance edge)
//!   - ascender / descender ink that exceeds line-height
//!   - kerning offsets that push glyphs outside the run's nominal box
//!
//! Mirrors the canonical Blitz read path:
//!   - `vendor/blitz-paint/src/render.rs::draw_inline_layout`
//!     (how `inline_layout_data.as_ref()` is consumed)
//!   - `vendor/blitz-paint/src/render/background_clip_text.rs`
//!     (how glyph metrics are queried via skrifa for paint-side geometry)

use blitz_dom::node::TextBrush;
use parley::{Layout, PositionedLayoutItem};
use skrifa::{
    FontRef, MetadataProvider,
    instance::{LocationRef, NormalizedCoord, Size},
    raw::types::{F2Dot14, GlyphId},
};

/// One shaped, positioned glyph plus its actual ink bounds — what
/// would paint on the GPU. Coordinates are in the element's local
/// inline-layout space (relative to the element's content-box origin,
/// which the lint rule treats as the element's bbox origin).
#[derive(Debug, Clone, Copy)]
pub struct GlyphInk {
    /// Glyph's pen position (left edge of advance) along the baseline.
    pub pen_x: f32,
    /// Glyph's pen position along the baseline (screen-space y).
    pub pen_y: f32,
    /// Left edge of the glyph's actual ink, in element-local
    /// screen-space coordinates. Computed from skrifa's
    /// `GlyphMetrics::bounds()` for the run's font size, then flipped
    /// from font-space y-up to screen y-down and translated by pen
    /// position. Includes italic side-bearings and ascender/descender
    /// overrun.
    pub ink_min_x: f32,
    /// Top edge of the glyph's ink in element-local screen-space.
    pub ink_min_y: f32,
    /// Right edge of the glyph's ink in element-local screen-space.
    pub ink_max_x: f32,
    /// Bottom edge of the glyph's ink in element-local screen-space.
    pub ink_max_y: f32,
}

/// Per-element shaped-glyph collection.
#[derive(Debug, Clone, Default)]
pub struct GlyphRunData {
    /// Every positioned glyph the element painted, in document order.
    pub glyphs: Vec<GlyphInk>,
}

impl GlyphRunData {
    /// Build a `GlyphRunData` by walking a parley `Layout` and querying
    /// skrifa for each positioned glyph's ink bbox. Returns `None` when
    /// the layout has zero glyphs (e.g. whitespace-only or font load
    /// failures across the board).
    pub fn from_layout(layout: &Layout<TextBrush>) -> Option<Self> {
        let mut out = Vec::new();

        for line in layout.lines() {
            for item in line.items() {
                let PositionedLayoutItem::GlyphRun(glyph_run) = item else {
                    continue;
                };
                let run = glyph_run.run();
                let font_data = run.font();
                let font_size = run.font_size();
                let Ok(font_ref) =
                    FontRef::from_index(font_data.data.as_ref(), font_data.index)
                else {
                    continue;
                };

                let raw_coords = run.normalized_coords();
                let norm_coords: Vec<NormalizedCoord> = raw_coords
                    .iter()
                    .map(|c| F2Dot14::from_bits(*c))
                    .collect();
                let location = LocationRef::new(&norm_coords);
                let metrics = font_ref.glyph_metrics(Size::new(font_size), location);

                for glyph in glyph_run.positioned_glyphs() {
                    let Some(bb) = metrics.bounds(GlyphId::new(glyph.id)) else {
                        continue;
                    };
                    // skrifa returns BoundingBox<f32> in font y-up coordinates
                    // already scaled by font_size / units_per_em. Flip y to
                    // match parley/screen (y-down).
                    let ink_min_x = glyph.x + bb.x_min;
                    let ink_max_x = glyph.x + bb.x_max;
                    let ink_min_y = glyph.y - bb.y_max;
                    let ink_max_y = glyph.y - bb.y_min;
                    out.push(GlyphInk {
                        pen_x: glyph.x,
                        pen_y: glyph.y,
                        ink_min_x,
                        ink_min_y,
                        ink_max_x,
                        ink_max_y,
                    });
                }
            }
        }

        if out.is_empty() {
            None
        } else {
            Some(Self { glyphs: out })
        }
    }
}
