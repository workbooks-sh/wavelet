//! External-backend adapters — wraps third-party APIs (stock search,
//! music gen, TTS, img2vid, lip-sync, voice match) behind cluster-shaped
//! traits.
//!
//! Phase 2/3/5/6 of the screenplay-to-MP4 epic (wb-iv3c).
//!
//! ## Design — cluster traits, not category traits
//!
//! Providers cluster by *prompting shape*, not by category. For
//! example, Runway Gen-3 and Kling both take `still + motion_prompt` →
//! one trait (`Img2VidGen`) with two impls. Suno and Udio share
//! "structured text + section markers" → one trait. This makes the
//! agent's reasoning structural: "this run has provider X but not Y, so
//! capability Z is unavailable" is mechanical, not per-provider.
//!
//! See `feedback_cluster_backends_by_prompt_shape` in
//! `~/.claude/projects/.../memory/`.
//!
//! ## Common machinery
//!
//! - **`RunMode`** — `DryRun` (emit request spec, no API call) vs
//!   `Live { max_cost_usd }` (gated by a per-run budget).
//! - **`CostEstimate`** — every backend exposes a `estimate_cost(req)`
//!   call so the CLI can preview spend.
//! - **`BackendError`** — uniform error surface.
//! - **`cache`** — content-addressed cache keyed by request hash.
//!   Re-running the same request returns the cached response;
//!   prevents accidental re-bills.

use serde::{Deserialize, Serialize};
use thiserror::Error;

pub mod cache;
pub mod captions;
pub mod clipref_emit;
pub mod elevenlabs;
pub mod fal;
pub mod google;
pub mod http_client;
pub mod image;
pub mod music;
pub mod pexels;
pub mod pond5;
pub mod replicate;
pub mod roboflow;
pub mod stock;
pub mod tts;
pub mod udio;
pub mod util;
pub mod video;

pub use captions::{
    CaptionsBackend, CaptionsRequest, CaptionsResult, OverlayConfig, OverlayStyle, WordTimestamp,
};
pub use stock::{
    Orientation, StockItem, StockSearchBackend, StockSearchRequest, StockSearchResult,
};
pub use tts::{TtsRequest, TtsResult, VoiceIdTtsBackend};

/// Whether an adapter actually hits the API or just emits the request
/// spec it *would* send.
#[derive(Debug, Clone, Copy)]
pub enum RunMode {
    /// Don't call the backend. Adapters return a synthetic response
    /// that includes the request shape they would have sent — useful
    /// for cost preview and for tests.
    DryRun,
    /// Hit the backend, but refuse if the estimated cost exceeds
    /// `max_cost_usd`. A budget of `0.0` means "free clusters only" —
    /// adapters whose estimated cost is `> 0` must refuse.
    Live {
        /// Maximum spend permitted on this single call, in USD.
        max_cost_usd: f32,
    },
}

impl RunMode {
    /// True iff the mode would actually make a network request.
    pub fn is_live(self) -> bool {
        matches!(self, RunMode::Live { .. })
    }

    /// Returns the budget when in live mode, else `0.0`.
    pub fn max_cost_usd(self) -> f32 {
        match self {
            RunMode::DryRun => 0.0,
            RunMode::Live { max_cost_usd } => max_cost_usd,
        }
    }
}

/// One backend's estimate of what a request will cost.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CostEstimate {
    /// Backend identifier (`pexels`, `suno`, `runway`, …).
    pub provider: String,
    /// Estimated cost in USD. `0.0` for free-tier / unlimited providers
    /// like Pexels.
    pub cost_usd: f32,
    /// Short human-readable explanation of how the estimate was
    /// produced (so the agent can reason about it).
    pub explanation: String,
}

/// Uniform error surface across every backend adapter.
#[derive(Debug, Error)]
pub enum BackendError {
    /// The backend isn't wired yet — known cluster, no live adapter.
    #[error("backend `{0}` is not implemented yet")]
    Unimplemented(&'static str),
    /// A required environment variable (typically an API key) wasn't
    /// present. The error message names the variable.
    #[error("missing credential env var: {0}")]
    MissingCredential(String),
    /// The caller's budget was lower than the backend's estimate.
    #[error("estimated cost ${estimate:.4} exceeds budget ${budget:.4}")]
    OverBudget {
        /// Backend's cost estimate, USD.
        estimate: f32,
        /// Caller-provided ceiling, USD.
        budget: f32,
    },
    /// Network-layer or transport-level failure.
    #[error("transport: {0}")]
    Transport(String),
    /// The backend returned an HTTP error response.
    #[error("http {status}: {body}")]
    HttpStatus {
        /// HTTP status code.
        status: u16,
        /// Response body (truncated to first ~512 chars).
        body: String,
    },
    /// Couldn't parse the backend's response into the expected shape.
    #[error("decode: {0}")]
    Decode(String),
    /// On-disk cache I/O failure.
    #[error("cache: {0}")]
    Cache(String),
    /// Backend rejected the request as malformed.
    #[error("invalid request: {0}")]
    InvalidRequest(String),
}

/// Standard wrapper around a backend call's outcome — includes the
/// request hash, whether the response came from cache, and the cost
/// estimate. Used uniformly so CLI output looks the same shape across
/// every cluster.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackendCallOutcome<R> {
    /// The actual response payload.
    pub response: R,
    /// Provider identifier.
    pub provider: String,
    /// Stable hash of the request — same input = same hash. Used as
    /// the cache key.
    pub request_hash: String,
    /// True when the response was served from the local cache instead
    /// of hitting the backend.
    pub cached: bool,
    /// Cost estimate at request time. Cache hits report `0.0` (the cost
    /// was incurred at original fetch time and the manifest carries it).
    pub cost_estimate_usd: f32,
    /// `"dry-run"` or `"live"` — mirrors the `RunMode` used.
    pub mode: &'static str,
}

/// Format a `RunMode` as one of the two canonical strings the wire
/// format uses.
pub(crate) fn mode_label(mode: RunMode) -> &'static str {
    match mode {
        RunMode::DryRun => "dry-run",
        RunMode::Live { .. } => "live",
    }
}

/// Map a `BackendError` to a CLI exit code per the wavelet convention:
///
/// - `0` = success
/// - `1` = generic runtime error (HTTP, transport, decode, I/O)
/// - `2` = clap arg parse error (reserved — never returned from here)
/// - `3` = post-parse hard fail (missing credential, over-budget,
///         invalid-request gate, unimplemented backend)
///
/// Exit `2` is reserved for clap; any error originating after arg
/// parse (an HTTP 4xx, a missing credential env var, a cost-gate trip)
/// must NOT collide with that. Eval drivers rely on this split to
/// distinguish "I called the tool wrong" from "the tool ran but the
/// backend pushed back".
///
/// Shared across every CLI handler (`shot txt2vid`, `music gen`,
/// `dialogue tts`, `lipsync`, ...) so a single convention is enforced.
pub fn exit_for_backend_error(err: &BackendError) -> std::process::ExitCode {
    use std::process::ExitCode;
    match err {
        BackendError::MissingCredential(_)
        | BackendError::OverBudget { .. }
        | BackendError::InvalidRequest(_)
        | BackendError::Unimplemented(_) => ExitCode::from(3),
        BackendError::Transport(_)
        | BackendError::HttpStatus { .. }
        | BackendError::Decode(_)
        | BackendError::Cache(_) => ExitCode::from(1),
    }
}

/// Centralized budget gate used by every cluster trait. Dry-run mode
/// always passes (no actual spend); live mode rejects when the estimate
/// exceeds the configured budget.
pub(crate) fn check_budget(estimate: &CostEstimate, mode: RunMode) -> Result<(), BackendError> {
    match mode {
        RunMode::DryRun => Ok(()),
        RunMode::Live { max_cost_usd } => {
            if estimate.cost_usd > max_cost_usd + 1e-6 {
                Err(BackendError::OverBudget {
                    estimate: estimate.cost_usd,
                    budget: max_cost_usd,
                })
            } else {
                Ok(())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn run_mode_helpers() {
        assert!(!RunMode::DryRun.is_live());
        assert!(RunMode::Live { max_cost_usd: 0.10 }.is_live());
        assert_eq!(RunMode::DryRun.max_cost_usd(), 0.0);
        assert!((RunMode::Live { max_cost_usd: 0.10 }.max_cost_usd() - 0.10).abs() < 1e-6);
    }

    #[test]
    fn over_budget_is_useful_error() {
        let err = BackendError::OverBudget {
            estimate: 0.5,
            budget: 0.1,
        };
        let msg = format!("{err}");
        assert!(msg.contains("0.5000"));
        assert!(msg.contains("0.1000"));
    }

    #[test]
    fn mode_label_is_canonical() {
        assert_eq!(mode_label(RunMode::DryRun), "dry-run");
        assert_eq!(mode_label(RunMode::Live { max_cost_usd: 0.0 }), "live");
    }

    #[test]
    fn dry_run_bypasses_budget_gate() {
        let est = CostEstimate {
            provider: "x".into(),
            cost_usd: 999.0,
            explanation: "expensive".into(),
        };
        assert!(check_budget(&est, RunMode::DryRun).is_ok());
    }

    #[test]
    fn live_mode_respects_budget() {
        let est = CostEstimate {
            provider: "x".into(),
            cost_usd: 0.50,
            explanation: "test".into(),
        };
        assert!(check_budget(&est, RunMode::Live { max_cost_usd: 1.0 }).is_ok());
        assert!(check_budget(&est, RunMode::Live { max_cost_usd: 0.1 }).is_err());
    }

    /// Cross-handler regression: `exit_for_backend_error` must never
    /// return exit 2 (clap collision) for any `BackendError` variant.
    /// Music, dialogue, lipsync and shot-txt2vid all route through
    /// this single helper — the test locks the convention in one place
    /// so adding a new variant without updating the mapping breaks
    /// the build (non-exhaustive match + this case-list test).
    #[test]
    fn exit_for_backend_error_never_returns_two() {
        use std::process::ExitCode;
        let two = format!("{:?}", ExitCode::from(2));
        let cases = [
            BackendError::Unimplemented("x"),
            BackendError::MissingCredential("KEY".into()),
            BackendError::OverBudget {
                estimate: 1.0,
                budget: 0.5,
            },
            BackendError::Transport("net".into()),
            BackendError::HttpStatus {
                status: 500,
                body: "".into(),
            },
            BackendError::Decode("d".into()),
            BackendError::Cache("c".into()),
            BackendError::InvalidRequest("i".into()),
        ];
        for err in &cases {
            let code = exit_for_backend_error(err);
            assert_ne!(
                format!("{:?}", code),
                two,
                "BackendError {err:?} routed to exit 2 (reserved for clap parse errors)",
            );
        }
    }

    #[test]
    fn exit_for_backend_error_hard_fail_variants_route_to_three() {
        use std::process::ExitCode;
        let three = format!("{:?}", ExitCode::from(3));
        let cases = [
            BackendError::MissingCredential("ELEVENLABS_API_KEY".into()),
            BackendError::OverBudget {
                estimate: 0.10,
                budget: 0.00,
            },
            BackendError::InvalidRequest("prompt empty".into()),
            BackendError::Unimplemented("google-lyria-3-pro"),
        ];
        for err in &cases {
            let code = exit_for_backend_error(err);
            assert_eq!(
                format!("{:?}", code),
                three,
                "expected exit 3 for {err:?} (post-parse hard fail)",
            );
        }
    }

    #[test]
    fn exit_for_backend_error_runtime_variants_route_to_one() {
        use std::process::ExitCode;
        let one = format!("{:?}", ExitCode::from(1));
        let cases = [
            BackendError::Transport("connection refused".into()),
            BackendError::HttpStatus {
                status: 503,
                body: "lyria queue down".into(),
            },
            BackendError::Decode("malformed lyria response".into()),
            BackendError::Cache("disk full".into()),
        ];
        for err in &cases {
            let code = exit_for_backend_error(err);
            assert_eq!(
                format!("{:?}", code),
                one,
                "expected exit 1 for {err:?} (generic runtime)",
            );
        }
    }

    /// Music gen regression: the eval-010-v5 trace showed
    /// `wavelet music gen --backend google-lyria-3-pro --max-cost 0.10`
    /// returning exit 2 after a 29-second API call. With the helper,
    /// the OverBudget gate trip pre-call routes to 3, and any HTTP
    /// pushback routes to 1 — never 2.
    #[test]
    fn music_gen_over_budget_maps_to_three() {
        use std::process::ExitCode;
        let err = BackendError::OverBudget {
            estimate: 0.40,
            budget: 0.10,
        };
        assert_eq!(
            format!("{:?}", exit_for_backend_error(&err)),
            format!("{:?}", ExitCode::from(3)),
        );
    }

    #[test]
    fn music_gen_http_status_maps_to_one() {
        use std::process::ExitCode;
        let err = BackendError::HttpStatus {
            status: 429,
            body: "rate limited".into(),
        };
        assert_eq!(
            format!("{:?}", exit_for_backend_error(&err)),
            format!("{:?}", ExitCode::from(1)),
        );
    }
}
