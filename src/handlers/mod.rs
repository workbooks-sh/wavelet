//! Per-subcommand CLI handlers. Each handler is the body of one
//! `ImageOp::*` (or future `VideoOp::*` / etc.) arm extracted from
//! `src/bin/wavelet.rs::run_image`. Centralizing them here keeps the
//! CLI dispatch thin and makes individual handlers independently
//! testable.

use crate::handlers::image_dispatch::run_image;
/// Shared helpers used by the per-subcommand handler modules below —
/// image-argument normalization, region parsing, structured-result
/// emission, pairwise-verdict parsing.
pub mod util;

/// Handler for the `image composite` subcommand — composite a
/// foreground (with alpha) over a background image.
pub mod image_composite;

/// Handler for the `image bg-remove` subcommand — background removal.
pub mod image_bg_remove;

/// Handler for the `image contrast` subcommand — measure text contrast.
pub mod image_contrast;

/// Handler for the `image identity-check` subcommand — CLIP similarity.
pub mod image_identity_check;

/// Handler for the `image isolate` subcommand — segment by text.
pub mod image_isolate;

/// Handler for the `image scrim` subcommand — readable-text region finder.
pub mod image_scrim;

/// Handler for the `image verify-shot` subcommand — VLM-graded verification.
pub mod image_verify_shot;
/// (auto-generated placeholder)
pub mod shot_still_variants;
/// (auto-generated placeholder)
pub mod shot_insert_into_scene;
/// (auto-generated placeholder)
pub mod shot_upscale;
/// (auto-generated placeholder)
pub mod shot_fix_from_verify;
/// (auto-generated placeholder)
pub mod shot_fix;
/// (auto-generated placeholder)
pub mod shot_txt2vid;
/// (auto-generated placeholder)
pub mod shot_still;
/// (auto-generated placeholder)
pub mod shot_search;
/// (auto-generated placeholder)
pub mod shot_edit;
/// (auto-generated placeholder)
pub mod shot_dispatch;
/// (auto-generated placeholder)
pub mod query_dispatch;
/// (auto-generated placeholder)
pub mod storyboard_dispatch;
/// (auto-generated placeholder)
pub mod image_dispatch;
/// (auto-generated placeholder)
pub mod image_ocr;
/// (auto-generated placeholder)
pub mod query_shader;
/// (auto-generated placeholder)
pub mod agent_dispatch;
/// `wavelet lint` orchestrator — runs lint rules against scenes.
pub mod lint;
