//! ABI types — Rust mirrors of the WGSL contract in `ABI.md`.
//!
//! `AssertionResult` is bytemuck Pod with the exact storage-buffer layout
//! every assertion shader writes into binding 4. `AssertionOutcome` is the
//! caller-facing decoded form (evidence truncated to `evidence_count`,
//! reason code resolved to a string).

use std::path::PathBuf;

use bytemuck::{Pod, Zeroable};

/// Bounded inline evidence array length, matches WGSL `array<f32, 64>`.
pub const EVIDENCE_CAPACITY: usize = 64;

/// Maximum uniform buffer bytes the dispatcher writes for `Params`. Sized
/// to the portable WebGPU `maxUniformBufferBindingSize / 256` floor so any
/// shader using this slot loads on any backend.
pub const PARAMS_MAX_BYTES: usize = 256;

/// Raw layout of the `AssertionResult` storage buffer (binding 4). Read
/// back verbatim via `bytemuck::from_bytes` after each dispatch.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct AssertionResult {
    /// 0 = failed, 1 = passed.
    pub passed: u32,
    /// Stable code; see `ReasonCode` for the cross-validator vocabulary.
    pub reason_code: i32,
    /// Number of valid floats in `evidence` (0..=EVIDENCE_CAPACITY).
    pub evidence_count: u32,
    /// Inline diagnostic floats. Interpretation is shader-specific.
    pub evidence: [f32; EVIDENCE_CAPACITY],
}

impl AssertionResult {
    /// Total byte size of the storage buffer the dispatcher allocates.
    pub const SIZE: usize = std::mem::size_of::<Self>();
}

/// Decoded outcome the dispatcher returns to callers. `evidence` is
/// truncated to the first `evidence_count` floats; `reason` is the
/// human-readable mapping of `reason_code`.
#[derive(Clone, Debug)]
pub struct AssertionOutcome {
    /// Pass/fail decision.
    pub passed: bool,
    /// Raw code from the shader, retained for programmatic consumers.
    pub reason_code: i32,
    /// Human-readable label for `reason_code`.
    pub reason: String,
    /// Inline evidence floats, truncated to the valid prefix.
    pub evidence: Vec<f32>,
}

/// Stable reason-code → string mapping. Codes 0..=4 are the cross-validator
/// vocabulary defined in `ABI.md`; shaders may emit higher codes with
/// validator-specific meaning — those fall through to a generic label and
/// the caller is expected to interpret them from the shader header comment.
#[derive(Clone, Copy, Debug)]
pub enum ReasonCode {
    /// `passed = 1` success, or `passed = 0` with unspecified failure.
    Ok,
    /// Metric fell outside the assertion's accepted range.
    OutOfBounds,
    /// Region under test was empty or unmatched.
    RegionNotFound,
    /// Signal too weak to draw a conclusion (e.g. flat region).
    InsufficientSignal,
    /// NaN / Inf encountered during reduction.
    NumericalIssue,
    /// Shader-defined code outside the shared vocabulary.
    ValidatorSpecific(i32),
}

impl ReasonCode {
    /// Decode a raw `i32` reason code from the storage buffer.
    pub fn from_raw(code: i32) -> Self {
        match code {
            0 => Self::Ok,
            1 => Self::OutOfBounds,
            2 => Self::RegionNotFound,
            3 => Self::InsufficientSignal,
            4 => Self::NumericalIssue,
            other => Self::ValidatorSpecific(other),
        }
    }

    /// Human-readable label for the reason.
    pub fn as_str(self) -> String {
        match self {
            Self::Ok => "ok".into(),
            Self::OutOfBounds => "metric out of bounds".into(),
            Self::RegionNotFound => "region not found / empty mask".into(),
            Self::InsufficientSignal => "insufficient signal".into(),
            Self::NumericalIssue => "numerical issue (NaN / Inf)".into(),
            Self::ValidatorSpecific(c) => format!("validator-specific reason ({c})"),
        }
    }
}

/// Frame the validator runs over. `Texture` is the production path —
/// dispatcher consumes whatever the render kernel just produced. `PngPath`
/// is the test + CLI path: read a PNG from disk, upload it as the color
/// texture. `Rgba8` is the in-memory equivalent for callers that already
/// have decoded pixels.
pub enum FrameSource {
    /// Pre-allocated wgpu texture; dispatcher reads `width()`/`height()`.
    Texture(wgpu::Texture),
    /// On-disk PNG; dispatcher decodes via the `png` crate.
    PngPath(PathBuf),
    /// In-memory tightly-packed RGBA8 pixels.
    Rgba8 {
        /// Frame width in pixels.
        width: u32,
        /// Frame height in pixels.
        height: u32,
        /// `width * height * 4` bytes, row-major, no padding.
        pixels: Vec<u8>,
    },
}
