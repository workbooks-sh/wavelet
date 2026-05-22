//! Pond5 stock-search adapter — stub.
//!
//! Pond5 is a paid stock-footage marketplace (RF + RM licensing). The
//! prompting shape matches Pexels (keyword + orientation + duration),
//! so it lives in the same `StockSearch` cluster trait. The adapter is
//! a stub so the agent can structurally tell a Pond5 backend is
//! known-but-not-wired (and so the cluster has a documented fallback
//! position).
//!
//! Wiring requires a Pond5 API agreement; defer until needed.

use crate::backends::stock::{StockSearchBackend, StockSearchRequest, StockSearchResult};
use crate::backends::{
    BackendCallOutcome, BackendError, CostEstimate, RunMode,
};

/// Provider id.
pub const PROVIDER: &str = "pond5";

/// Stub adapter. Constructed cheaply; every `search` call returns
/// `BackendError::Unimplemented`.
#[derive(Debug, Clone, Default)]
pub struct Pond5Adapter;

impl StockSearchBackend for Pond5Adapter {
    fn name(&self) -> &'static str {
        PROVIDER
    }

    fn estimate_cost(&self, _: &StockSearchRequest) -> CostEstimate {
        // Pond5 RF clips are typically $20-200 each; we don't have a
        // pricing endpoint, so the estimate is a placeholder agents can
        // see in dry-run mode.
        CostEstimate {
            provider: PROVIDER.into(),
            cost_usd: 0.0,
            explanation: "Pond5 adapter not wired — estimate unavailable.".into(),
        }
    }

    fn search(
        &self,
        _request: &StockSearchRequest,
        _mode: RunMode,
    ) -> Result<BackendCallOutcome<StockSearchResult>, BackendError> {
        Err(BackendError::Unimplemented(PROVIDER))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stub_returns_unimplemented() {
        let adapter = Pond5Adapter::default();
        let req = StockSearchRequest::new("ocean");
        let err = adapter.search(&req, RunMode::DryRun).unwrap_err();
        assert!(matches!(err, BackendError::Unimplemented("pond5")));
    }

    #[test]
    fn name_is_stable() {
        assert_eq!(Pond5Adapter::default().name(), "pond5");
    }
}
