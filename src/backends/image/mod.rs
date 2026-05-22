//! Image-processing clusters — operations that take an image and emit
//! a modified image.
//!
//! - **`BgRemove`** — universal foreground/background split (birefnet).
//! - **`SegmentByText`** — text-prompted segmentation.
//! - **`Txt2Img`** — generate stills.
//! - **`RefConditionedImgGen`** — reference-conditioned stills.
//!
//! Plus `compose` — pure-local apply-mask + composite-over helpers.

#![allow(missing_docs)]

pub mod compose;
pub use compose::{apply_mask, composite_over};

pub mod vision_verify;
pub mod segment_by_text;
pub mod txt2img;
pub mod ref_conditioned;
pub mod bg_remove;
pub mod identity_check;
pub mod ocr;
pub mod instruction_edit;
pub mod upscale;
pub mod face_detect;
pub mod low_denoise;
pub mod pairwise_compare;

pub use vision_verify::*;
pub use segment_by_text::*;
pub use txt2img::*;
pub use ref_conditioned::*;
pub use bg_remove::*;
pub use identity_check::*;
pub use ocr::*;
pub use instruction_edit::*;
pub use upscale::*;
pub use face_detect::*;
pub use low_denoise::*;
pub use pairwise_compare::*;

/// Cluster identifier for `BgRemove` — used in cache keys.
pub const CLUSTER_BG_REMOVE: &str = "image_bg_remove";

/// Cluster identifier for text-prompted segmentation.
pub const CLUSTER_SEGMENT: &str = "image_segment_by_text";

/// Cluster identifier for txt2img.
pub const CLUSTER_TXT2IMG: &str = "image_txt2img";

/// Cluster identifier for reference-conditioned image gen.
pub const CLUSTER_REF_IMG_GEN: &str = "image_ref_conditioned_gen";

/// Cluster identifier for identity-similarity verification.
pub const CLUSTER_IDENTITY_CHECK: &str = "image_identity_check";

/// Cluster identifier for vision-verify (pre-render brief-match check).
pub const CLUSTER_VISION_VERIFY: &str = "image_vision_verify";

/// Cluster identifier for VISTA-style pairwise comparison — used in
pub const CLUSTER_PAIRWISE_COMPARE: &str = "image_pairwise_compare";

/// Cluster identifier for OCR (baked-text detection).
pub const CLUSTER_OCR: &str = "image_ocr";

/// Cluster identifier for instruction-driven surgical edits (Flux Kontext).
pub const CLUSTER_INSTRUCTION_EDIT: &str = "instruction-edit";

/// Cluster identifier for face detection (yolov8-shaped bbox detectors).
pub const CLUSTER_FACE_DETECT: &str = "image_face_detect";

/// Cluster identifier for low-denoise img2img refinement — the inner
pub const CLUSTER_LOW_DENOISE_IMG2IMG: &str = "image_low_denoise_img2img";

/// Cluster identifier for final-pass image upscale (single still polish —
pub const CLUSTER_UPSCALE_IMAGE: &str = "image_upscale";

/// Cluster identifier for final-pass video upscale (the last stage of
pub const CLUSTER_UPSCALE_VIDEO: &str = "video_upscale";

