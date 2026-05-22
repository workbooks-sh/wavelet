//! Fluent Rust API for building WaveletFx compositions without going through the
//! text parser. This is the v0 entry point — exercise IR and emit end-to-end
//! before investing in parser ergonomics.
//!
//! Every numeric parameter accepts `impl Into<Value>`, so both
//! constants and Animato tweens compose naturally:
//!
//! ```ignore
//! use wavelet_fx::{noise, src, Tween, Easing};
//!
//! let pulse = Tween::new(0.0_f32, 0.1)
//!     .duration(2.0)
//!     .easing(Easing::EaseInOutSine)
//!     .build();
//!
//! let comp = src(0)
//!     .modulate(noise(4.0, 0.0), pulse)   // <-- tween drives modulation depth
//!     .contrast(1.1)
//!     .output();
//! ```

use crate::ast::{Combinator, Node, Output, Source, Transform};
use crate::value::{UniformRef, Value};

#[derive(Debug, Clone)]
pub struct Chain(pub(crate) Node);

#[derive(Debug, Clone)]
pub struct Composition {
    pub outputs: Vec<Output>,
}

// ---- sources -----------------------------------------------------------

pub fn osc(frequency: impl Into<Value>) -> Chain {
    Chain(Node::Source(Source::Osc {
        frequency: frequency.into(),
        sync: Value::Const(0.1),
        offset: Value::Const(0.0),
    }))
}

pub fn noise(scale: impl Into<Value>, offset: impl Into<Value>) -> Chain {
    Chain(Node::Source(Source::Noise {
        scale: scale.into(),
        offset: offset.into(),
    }))
}

pub fn voronoi(
    scale: impl Into<Value>,
    speed: impl Into<Value>,
    blending: impl Into<Value>,
) -> Chain {
    Chain(Node::Source(Source::Voronoi {
        scale: scale.into(),
        speed: speed.into(),
        blending: blending.into(),
    }))
}

pub fn gradient(speed: impl Into<Value>) -> Chain {
    Chain(Node::Source(Source::Gradient { speed: speed.into() }))
}

pub fn solid(
    r: impl Into<Value>,
    g: impl Into<Value>,
    b: impl Into<Value>,
    a: impl Into<Value>,
) -> Chain {
    Chain(Node::Source(Source::Solid {
        r: r.into(),
        g: g.into(),
        b: b.into(),
        a: a.into(),
    }))
}

pub fn shape(
    sides: u32,
    radius: impl Into<Value>,
    smoothing: impl Into<Value>,
) -> Chain {
    Chain(Node::Source(Source::Shape {
        sides,
        radius: radius.into(),
        smoothing: smoothing.into(),
    }))
}

/// 2D circle SDF rendered as a soft-edged disc. Math sourced from
/// `sdfu` / Inigo Quilez's SDF catalog — see `stdlib::sdf`.
pub fn sphere(radius: impl Into<Value>, smoothing: impl Into<Value>) -> Chain {
    Chain(Node::Source(Source::Sphere {
        radius: radius.into(),
        smoothing: smoothing.into(),
    }))
}

/// 2D axis-aligned box SDF. `width` and `height` are half-extents in
/// uv-units. Math sourced from `sdfu` / IQ.
pub fn box_sdf(
    width: impl Into<Value>,
    height: impl Into<Value>,
    smoothing: impl Into<Value>,
) -> Chain {
    Chain(Node::Source(Source::BoxSdf {
        width: width.into(),
        height: height.into(),
        smoothing: smoothing.into(),
    }))
}

/// 2D annulus / ring SDF. `radius` is the centerline, `thickness` is
/// half the band width. Math sourced from `sdfu` / IQ.
pub fn torus(
    radius: impl Into<Value>,
    thickness: impl Into<Value>,
    smoothing: impl Into<Value>,
) -> Chain {
    Chain(Node::Source(Source::Torus {
        radius: radius.into(),
        thickness: thickness.into(),
        smoothing: smoothing.into(),
    }))
}

pub fn src(channel: u32) -> Chain {
    Chain(Node::Source(Source::Src { channel }))
}

// ---- dynamic-uniform Values --------------------------------------------
//
// Any builder/transform parameter takes `impl Into<Value>`, so these
// helpers slot into the chain alongside numeric literals and tweens:
//
// ```ignore
// src(0).contrast(audio_rms())            // contrast tied to running RMS
// src(0).modulate(noise(prop("--energy")), 0.05)  // CSS var drives scale
// ```
//
// Multiple references to the same source share one uniform slot in the
// emitted Uniforms struct (dedup is done at lower time).

/// Running RMS of the consumer's audio mixer. Slot: `u_audio_rms`.
pub fn audio_rms() -> Value {
    Value::Uniform(UniformRef::AudioRms)
}

/// Energy of FFT bin `n`. Slot: `u_audio_fft_<n>`.
pub fn audio_fft(n: u32) -> Value {
    Value::Uniform(UniformRef::AudioFftBin(n))
}

/// Current beat phase in `[0, 1)`, or -1 when there's no beat track. Slot:
/// `u_beat`.
pub fn time_beat() -> Value {
    Value::Uniform(UniformRef::Beat)
}

/// Deterministic per-frame seed (`comp_hash ^ frame_index` cast to f32).
/// Slot: `u_seed`.
pub fn seed() -> Value {
    Value::Uniform(UniformRef::Seed)
}

/// Value of a CSS custom property on the host element. Slot:
/// `u_prop_<sanitized>` (leading `--` stripped, dashes mapped to `_`).
pub fn prop(name: impl Into<String>) -> Value {
    Value::Uniform(UniformRef::CssProp(name.into()))
}

/// Read from a named intermediate buffer (declared earlier in the
/// composition via `output_to(name)`).
pub fn from_buffer(name: impl Into<String>) -> Chain {
    Chain(Node::Source(Source::Buffer { name: name.into() }))
}

/// Read the previous frame of the current pass's output buffer. Only valid
/// inside a chain that ends in `output_to(name)`; using `prev()` inside a
/// chain that terminates with `output()` is a lowering error.
pub fn prev() -> Chain {
    Chain(Node::Source(Source::Prev))
}

// ---- transforms (chained) ----------------------------------------------

impl Chain {
    fn wrap(self, op: Transform) -> Chain {
        Chain(Node::Transform {
            input: Box::new(self.0),
            op,
        })
    }

    pub fn rotate(self, angle: impl Into<Value>, speed: impl Into<Value>) -> Chain {
        self.wrap(Transform::Rotate {
            angle: angle.into(),
            speed: speed.into(),
        })
    }

    pub fn scale(self, amount: impl Into<Value>) -> Chain {
        let amount = amount.into();
        self.wrap(Transform::Scale {
            amount,
            x: Value::Const(1.0),
            y: Value::Const(1.0),
        })
    }

    pub fn color(
        self,
        r: impl Into<Value>,
        g: impl Into<Value>,
        b: impl Into<Value>,
        a: impl Into<Value>,
    ) -> Chain {
        self.wrap(Transform::Color {
            r: r.into(),
            g: g.into(),
            b: b.into(),
            a: a.into(),
        })
    }

    pub fn brightness(self, amount: impl Into<Value>) -> Chain {
        self.wrap(Transform::Brightness { amount: amount.into() })
    }

    pub fn contrast(self, amount: impl Into<Value>) -> Chain {
        self.wrap(Transform::Contrast { amount: amount.into() })
    }

    pub fn invert(self, amount: impl Into<Value>) -> Chain {
        self.wrap(Transform::Invert { amount: amount.into() })
    }

    pub fn scroll(
        self,
        x: impl Into<Value>,
        y: impl Into<Value>,
        speed_x: impl Into<Value>,
        speed_y: impl Into<Value>,
    ) -> Chain {
        self.wrap(Transform::Scroll {
            x: x.into(),
            y: y.into(),
            speed_x: speed_x.into(),
            speed_y: speed_y.into(),
        })
    }

    pub fn pixelate(self, x: impl Into<Value>, y: impl Into<Value>) -> Chain {
        self.wrap(Transform::Pixelate {
            x: x.into(),
            y: y.into(),
        })
    }

    pub fn repeat(
        self,
        x: impl Into<Value>,
        y: impl Into<Value>,
        offset_x: impl Into<Value>,
        offset_y: impl Into<Value>,
    ) -> Chain {
        self.wrap(Transform::Repeat {
            x: x.into(),
            y: y.into(),
            offset_x: offset_x.into(),
            offset_y: offset_y.into(),
        })
    }
}

// ---- combinators (chained) ---------------------------------------------

impl Chain {
    fn combine(self, rhs: Chain, op: Combinator) -> Chain {
        Chain(Node::Combine {
            lhs: Box::new(self.0),
            rhs: Box::new(rhs.0),
            op,
        })
    }

    pub fn add(self, rhs: Chain, amount: impl Into<Value>) -> Chain {
        self.combine(rhs, Combinator::Add { amount: amount.into() })
    }

    pub fn mult(self, rhs: Chain, amount: impl Into<Value>) -> Chain {
        self.combine(rhs, Combinator::Mult { amount: amount.into() })
    }

    /// Linear interpolation between two chains: `mix(self, rhs, amount)`.
    /// Use for crossfades and any "blend N% of B into A" effect.
    ///
    /// `amount` is the weight of `rhs`. `0.0` = pure `self`; `1.0` = pure
    /// `rhs`. Animate it via `prop("progress")` for a transition.
    ///
    /// **Not what this is**: a perceptual blur, a soft mask, or a stylized
    /// dissolve. For those:
    /// - For Gaussian blur, use `.blur(radius)`.
    /// - For a luma-driven dissolve, use `.mask` with a noise/gradient
    ///   source (and ideally a smoothstep — see `RECIPES.md`).
    /// - For a chromatic-aberration glitch, see the `chroma_shift` recipe.
    pub fn blend(self, rhs: Chain, amount: impl Into<Value>) -> Chain {
        self.combine(rhs, Combinator::Blend { amount: amount.into() })
    }

    /// UV-displace this chain by the per-pixel `(R, G)` of `rhs`, scaled by
    /// `amount`. For every pixel `p`, sample `self` at `p + amount * (rhs(p).rg - 0.5)`
    /// rather than at `p`. Strong with low-frequency noise + small amounts
    /// (`noise(2..4)` + `amount ≤ 0.05`) for organic wobble or watery
    /// distortion.
    ///
    /// **Common foot-gun**: high-frequency noise + large amounts (e.g.
    /// `noise(12)` + `amount = 0.1`) produces jagged speckled displacement,
    /// not blur. If you want blur, use `.blur(radius)`. If you want a
    /// believable warp for a transition, keep `amount` ≤ 0.05 and apply a
    /// bell-curve envelope (`sin(progress * π)`) on the amount so it peaks
    /// mid-transition.
    pub fn modulate(self, rhs: Chain, amount: impl Into<Value>) -> Chain {
        self.combine(rhs, Combinator::Modulate { amount: amount.into() })
    }

    /// True multi-tap Gaussian blur. Radius is in pixels (1..=32).
    ///
    /// Implements a 9-tap separable approximation that's cheap enough for
    /// per-frame use at 1080p on M-series CPU (~0.5ms). For an artistic
    /// "out of focus" feel use `radius` 4..12; for a heavy glassmorphism
    /// blur use 16..32.
    ///
    /// Unlike `.modulate(noise, ...)` which UV-displaces, `.blur` actually
    /// averages neighboring samples — the result is smooth, not jagged.
    pub fn blur(self, radius: impl Into<Value>) -> Chain {
        self.wrap(Transform::Blur {
            radius: radius.into(),
        })
    }

    pub fn modulate_scale(
        self,
        rhs: Chain,
        multiple: impl Into<Value>,
        offset: impl Into<Value>,
    ) -> Chain {
        self.combine(
            rhs,
            Combinator::ModulateScale {
                multiple: multiple.into(),
                offset: offset.into(),
            },
        )
    }

    pub fn modulate_rotate(
        self,
        rhs: Chain,
        multiple: impl Into<Value>,
        offset: impl Into<Value>,
    ) -> Chain {
        self.combine(
            rhs,
            Combinator::ModulateRotate {
                multiple: multiple.into(),
                offset: offset.into(),
            },
        )
    }

    pub fn diff(self, rhs: Chain) -> Chain {
        self.combine(rhs, Combinator::Diff)
    }

    pub fn mask(self, rhs: Chain) -> Chain {
        self.combine(rhs, Combinator::Mask)
    }

    /// IQ-style smooth-min union of two SDF-shaped chains. Bigger `k`
    /// = wider blend zone. Works on the rendered grayscale masks of
    /// wavelet_fx's SDF sources (sphere / box_sdf / torus); for non-SDF
    /// sources the result is still defined (smooth-min of any two
    /// colors), but the visual reads best on SDF inputs. Math sourced
    /// from `sdfu` / Inigo Quilez.
    pub fn smooth_union(self, rhs: Chain, k: impl Into<Value>) -> Chain {
        self.combine(rhs, Combinator::SmoothUnion { k: k.into() })
    }

    /// IQ-style smooth-max intersect, symmetric counterpart of
    /// `smooth_union`.
    pub fn smooth_intersect(self, rhs: Chain, k: impl Into<Value>) -> Chain {
        self.combine(rhs, Combinator::SmoothIntersect { k: k.into() })
    }
}

// ---- terminators -------------------------------------------------------

impl Chain {
    pub fn output(self) -> Composition {
        Composition {
            outputs: vec![Output {
                chain: self.0,
                buffer: None,
            }],
        }
    }

    pub fn output_to(self, buffer: impl Into<String>) -> Composition {
        Composition {
            outputs: vec![Output {
                chain: self.0,
                buffer: Some(buffer.into()),
            }],
        }
    }
}

impl Composition {
    /// Chain this composition with another so the resulting composition has
    /// both sets of outputs, evaluated in the order they were appended. Use
    /// this to author multi-pass pipelines:
    ///
    /// ```ignore
    /// let pipeline = osc(3.0).output_to("smear")
    ///     .and_then(src(0).modulate(from_buffer("smear"), 0.1).output());
    /// ```
    ///
    /// Buffer references must be backwards-only: a pass can read any buffer
    /// that was declared (via `output_to`) earlier in the chain. Lowering
    /// rejects forward references.
    pub fn and_then(mut self, next: Composition) -> Composition {
        self.outputs.extend(next.outputs);
        self
    }
}
