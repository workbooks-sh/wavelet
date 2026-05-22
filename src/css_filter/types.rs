//! Core types for the CSS `filter:` model — function variants,
//! lengths, parse errors, and `Display` impl.

#![allow(missing_docs)]

use std::fmt;

#[derive(Debug, Clone, PartialEq)]
pub enum FilterFn {
    /// `blur(<length>)`. Sigma in the source's coordinate space.
    Blur(Length),
    /// `brightness(<number>)`. 1.0 = identity.
    Brightness(f32),
    /// `contrast(<number>)`. 1.0 = identity.
    Contrast(f32),
    /// `saturate(<number>)`. 1.0 = identity.
    Saturate(f32),
    /// `grayscale(<number>)`. 0.0 = identity, 1.0 = full grayscale.
    Grayscale(f32),
    /// `sepia(<number>)`. 0.0 = identity, 1.0 = full sepia.
    Sepia(f32),
    /// `invert(<number>)`. 0.0 = identity, 1.0 = full invert.
    Invert(f32),
    /// `opacity(<number>)`. 1.0 = fully visible.
    Opacity(f32),
    /// `hue-rotate(<angle>)`. Always stored in degrees.
    HueRotate(f32),
    /// `drop-shadow(<offset-x> <offset-y> <blur-radius> <color>)`.
    DropShadow {
        /// Horizontal offset (positive = right).
        offset_x: Length,
        /// Vertical offset (positive = down).
        offset_y: Length,
        /// Blur radius (sigma equivalent in source coord space).
        blur_radius: Length,
        /// RGBA color, channels in 0..=1.
        color: [f32; 4],
    },
}

/// A length value plus its declared unit. Resolution to pixels happens
/// at consumption time once the viewport dimensions are known.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Length {
    /// Numeric portion of the length.
    pub value: f32,
    /// Unit suffix.
    pub unit: LengthUnit,
}

/// Length unit. Mirrors the subset of CSS units that show up in
/// `filter:` values in practice; rarer units (`pt`, `pc`, `cm`, etc.)
/// can be added if a real brief asks for them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LengthUnit {
    /// Absolute pixels. The default when no unit is present.
    Px,
    /// Em — relative to the element's font size.
    Em,
    /// Rem — relative to the root font size.
    Rem,
    /// Viewport height percentage.
    Vh,
    /// Viewport width percentage.
    Vw,
    /// Percentage of the parent.
    Percent,
}

impl Length {
    /// Resolve the length to pixels given the viewport dimensions and a
    /// font-size proxy. For `filter:` values the font size is
    /// rarely meaningful; pass the root font size or 16.0 as a default.
    pub fn to_px(self, viewport_width: f32, viewport_height: f32, font_size_px: f32) -> f32 {
        match self.unit {
            LengthUnit::Px => self.value,
            LengthUnit::Em | LengthUnit::Rem => self.value * font_size_px,
            LengthUnit::Vh => self.value * viewport_height / 100.0,
            LengthUnit::Vw => self.value * viewport_width / 100.0,
            LengthUnit::Percent => self.value * viewport_width / 100.0,
        }
    }
}

/// Parser error. Most call sites just want to know whether the parse
/// succeeded — when it doesn't, the consumer should leave the filter
/// declaration on the element and let Blitz attempt to render it
/// (which may hang, but that's no worse than skipping the hijack).
#[derive(Debug, Clone, thiserror::Error)]
pub enum FilterParseError {
    /// The value was empty after trimming.
    #[error("empty filter value")]
    Empty,
    /// A function call was not closed by a `)`.
    #[error("unterminated function call near `{0}`")]
    Unterminated(String),
    /// A function name we don't recognize / don't support.
    #[error("unknown filter function `{0}`")]
    UnknownFunction(String),
    /// A function's arguments couldn't be parsed.
    #[error("invalid argument to `{func}`: `{arg}`")]
    InvalidArgument {
        /// Function name where the bad argument appeared.
        func: String,
        /// The raw argument substring that failed to parse.
        arg: String,
    },
}

impl fmt::Display for FilterFn {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FilterFn::Blur(l) => write!(f, "blur({}{:?})", l.value, l.unit),
            FilterFn::Brightness(v) => write!(f, "brightness({v})"),
            FilterFn::Contrast(v) => write!(f, "contrast({v})"),
            FilterFn::Saturate(v) => write!(f, "saturate({v})"),
            FilterFn::Grayscale(v) => write!(f, "grayscale({v})"),
            FilterFn::Sepia(v) => write!(f, "sepia({v})"),
            FilterFn::Invert(v) => write!(f, "invert({v})"),
            FilterFn::Opacity(v) => write!(f, "opacity({v})"),
            FilterFn::HueRotate(v) => write!(f, "hue-rotate({v}deg)"),
            FilterFn::DropShadow { offset_x, offset_y, blur_radius, color } => write!(
                f,
                "drop-shadow({}{:?} {}{:?} {}{:?} rgba({},{},{},{}))",
                offset_x.value, offset_x.unit,
                offset_y.value, offset_y.unit,
                blur_radius.value, blur_radius.unit,
                color[0], color[1], color[2], color[3],
            ),
        }
    }
}
