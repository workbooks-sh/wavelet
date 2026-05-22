//! Roboflow adapters — hosted vision models via `infer.roboflow.com`.
//!
//! Roboflow's catalog complements Fal: where Fal is gen-heavy, Roboflow
//! is vision-tool-heavy. We use it for the capabilities Fal doesn't
//! expose — real CLIP embeddings, production OCR, object detection.
//!
//! Auth is a `?api_key=…` query parameter, plumbed through the shared
//! [`HttpBackendClient`] under the `QueryParam` scheme. The
//! `RoboflowClient` builder reads `ROBOFLOW_API_KEY` from env.
//!
//! ## Submodules
//!
//! - [`clip`] — [`IdentitySimilarityBackend`] impl. Two embed calls
//!   (ViT-L/14 → 768-d vector) + local cosine. ~$0.001 per check.
//! - [`doctr`] — [`OcrBackend`] impl. Returns the recognized text as a
//!   single string (no bboxes from this endpoint). ~$0.001 per call.
//! - [`face_detect`] — [`FaceDetectBackend`] impl. Hosted
//!   `face-detection-mik1i/18` (yolov8-shape, public). Returns bboxes
//!   + confidences. ~$0.001 per call. Powers the face-refine
//!   paste-back pipeline (HelloRob template). Uses
//!   `detect.roboflow.com` rather than `infer.roboflow.com`, so the
//!   adapter holds the API key directly instead of going through
//!   `RoboflowClient`.

pub mod clip;
pub mod doctr;
pub mod face_detect;

pub use clip::RoboflowClipAdapter;
pub use doctr::RoboflowDoctrOcrAdapter;
pub use face_detect::RoboflowFaceDetectAdapter;
