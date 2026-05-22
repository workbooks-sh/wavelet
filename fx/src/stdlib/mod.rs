//! Stdlib — WGSL fragment bodies for each WaveletFx primitive.
//!
//! Each entry is a small chunk of WGSL that operates on an in-flight `vec4`
//! color and the standard inputs (`uv`, `iTime`, `iResolution`, etc). At emit
//! time the IR walker inlines the matching body for each AST node, threading
//! the color through.
//!
//! Split into three files mirroring the AST: sources produce a color from
//! scratch, transforms operate on one color, combinators mix two.

pub mod blur;
pub mod combinators;
pub mod sdf;
pub mod sources;
pub mod transforms;

/// Common helpers (hash, rotate2d, noise primitives, SDF distance
/// functions + smooth-min) that get prepended once per emitted shader.
/// Concatenation at compile time via `concat!` keeps the const a real
/// `&'static str` so emit.rs can pass it to `push_str` without
/// allocating. WGSL constant-folds the unused helpers, so paying the
/// bytes for the whole block per shader is fine.
pub const PRELUDE: &str = concat!(
    include_str!("prelude.wgsl"),
    "\n",
    include_str!("sdf.wgsl"),
);
