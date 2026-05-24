//! Screenplay-stage validations that run BEFORE paid generation.
//!
//! The author surface is Fountain; the work in this module operates on
//! the parsed AST (via the `fountain` crate). Currently houses
//! `duration_fit` — the pre-flight copy-density check that refuses to
//! advance to storyboard when the script demonstrably can't fit the
//! declared spot length.

pub mod duration_fit;
