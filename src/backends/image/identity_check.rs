//! Identity Check backend types.

#![allow(missing_docs)]

use crate::backends::{BackendCallOutcome, BackendError, CostEstimate, RunMode};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use crate::handlers::util::image_arg_to_url;

/// Identity-similarity verification request — compute embedding-space
/// similarity between a master reference of the subject and a generated
/// candidate still. Used as the gate after every `SceneStill` gen to
/// detect drift (e.g. Seedream returning a different car make/watch
/// face/sneaker model than the references).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IdentityCheckRequest {
    /// Public URL (or `data:` URL) of the master reference image of the
    /// subject — the ground-truth identity to verify against.
    pub reference_url: String,
    /// Public URL (or `data:` URL) of the generated still to verify.
    pub candidate_url: String,
}

impl IdentityCheckRequest {
    /// Build a minimum-viable request.
    pub fn new(
        reference_url: impl Into<String>,
        candidate_url: impl Into<String>,
    ) -> Self {
        Self {
            reference_url: reference_url.into(),
            candidate_url: candidate_url.into(),
        }
    }
}

/// Outcome of an identity-similarity check.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IdentityCheckResult {
    /// Provider identifier (`fal-clip-similarity`).
    pub provider: String,
    /// Cosine similarity in `[0.0, 1.0]`. Higher is more similar.
    pub similarity: f32,
    /// True when `similarity >= threshold`.
    pub passes_threshold: bool,
    /// Threshold used at decision time. Echoed back so the caller can
    /// reason about borderline scores without re-deriving the cutoff.
    pub threshold: f32,
}

/// Cluster trait shared by every identity-similarity adapter. Adapters
/// either return a similarity score directly (CLIP-similarity-style
/// endpoint) or fetch embeddings for both images and cosine-sim them
/// locally — the trait hides which path is taken.
pub trait IdentitySimilarityBackend {
    /// Provider name.
    fn name(&self) -> &'static str;
    /// Cost estimate.
    fn estimate_cost(&self, request: &IdentityCheckRequest) -> CostEstimate;
    /// Compare candidate against reference. `threshold` controls the
    /// `passes_threshold` flag on the result; the raw `similarity` is
    /// always returned so the caller can apply a softer policy if needed.
    fn check(
        &self,
        request: &IdentityCheckRequest,
        threshold: f32,
        mode: RunMode,
    ) -> Result<BackendCallOutcome<IdentityCheckResult>, BackendError>;
}

