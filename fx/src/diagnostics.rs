//! Error type for compile failures. Kept narrow on purpose — WaveletFx is a small
//! DSL and most errors fall into a handful of buckets.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum Diagnostic {
    #[error("unknown primitive: {0}")]
    UnknownPrimitive(String),

    #[error("type mismatch in {context}: expected {expected}, got {actual}")]
    TypeMismatch {
        context: String,
        expected: String,
        actual: String,
    },

    #[error("invalid composition: {0}")]
    InvalidComposition(String),

    #[error("parse error at {line}:{col}: {message}")]
    ParseError {
        line: usize,
        col: usize,
        message: String,
    },

    /// naga rejected an emitted WGSL pass. The message is naga's own
    /// formatted error (it includes line/column pointing into the WGSL
    /// string). `pass_name` is the [`crate::emit::EmittedPass::name`] so
    /// callers can identify which pass failed when the graph has many.
    #[error("naga rejected WGSL for pass '{pass_name}':\n{message}")]
    InvalidEmittedWgsl {
        pass_name: String,
        message: String,
    },

    #[error("internal: {0}")]
    Internal(String),
}
