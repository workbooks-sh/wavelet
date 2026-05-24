//! Clip-file utilities — operations on raw `.mp4` clip outputs that
//! aren't part of the structured clip-ref subsystem under `clipref/`.
//!
//! `trim_static` detects and removes the leading / trailing freeze
//! frames that Veo (and other AI video generators) commonly produce
//! when the model "boots up" before motion begins.

pub mod trim_static;
