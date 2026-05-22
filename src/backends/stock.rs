//! Stock-search cluster — keyword + filters → list of stock clips.
//!
//! Providers in this cluster share a prompting shape: a free-text
//! query plus orientation/duration filters. The trait surfaces that
//! shape; per-provider adapters translate it to their wire format.
//!
//! Members: **Pexels** (primary), **Pond5** (stub — adapter not wired).

use crate::backends::{BackendCallOutcome, BackendError, CostEstimate, RunMode};
use serde::{Deserialize, Serialize};

/// Cluster identifier — used in cache keys + manifests so cached entries
/// can be associated with their cluster, not just the specific provider.
pub const CLUSTER: &str = "stock_search";

/// Aspect/orientation filter the agent can request from stock APIs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Orientation {
    /// Wider than tall (typical video).
    Landscape,
    /// Taller than wide (mobile/social).
    Portrait,
    /// 1:1.
    Square,
}

/// One stock-search request. Shape is shared across providers.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StockSearchRequest {
    /// Free-text query (e.g. "highway desert sunset").
    pub query: String,
    /// Optional orientation filter.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub orientation: Option<Orientation>,
    /// Optional minimum clip duration in seconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_duration_secs: Option<u32>,
    /// Optional maximum clip duration in seconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_duration_secs: Option<u32>,
    /// Page size. Providers vary on the max; adapters clamp as needed.
    #[serde(default = "default_per_page")]
    pub per_page: u32,
    /// 1-based page index.
    #[serde(default = "default_page")]
    pub page: u32,
}

fn default_per_page() -> u32 {
    15
}
fn default_page() -> u32 {
    1
}

impl StockSearchRequest {
    /// Construct a request with the minimum useful fields populated.
    pub fn new(query: impl Into<String>) -> Self {
        Self {
            query: query.into(),
            orientation: None,
            min_duration_secs: None,
            max_duration_secs: None,
            per_page: default_per_page(),
            page: default_page(),
        }
    }
}

/// Result of a stock search — list of clips + pagination metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StockSearchResult {
    /// Provider identifier (`pexels`, `pond5`).
    pub provider: String,
    /// Returned items in this page.
    pub items: Vec<StockItem>,
    /// Total hits matching the query, when the backend reports it.
    /// `None` means the backend didn't include the count.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total_hits: Option<u32>,
    /// Page index this result is for.
    pub page: u32,
}

/// One clip in a stock-search response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StockItem {
    /// Provider-specific id (used to look up the original on the
    /// provider's site).
    pub id: String,
    /// Direct download URL of the highest-quality variant the adapter
    /// surfaces (typically HD).
    pub url: String,
    /// Optional thumbnail URL for preview.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thumb_url: Option<String>,
    /// Pixel width of the highest-quality variant.
    pub width: u32,
    /// Pixel height of the highest-quality variant.
    pub height: u32,
    /// Clip duration in seconds, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_secs: Option<u32>,
    /// Attribution string (author/photographer name).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    /// License identifier (`pexels`, `pond5-rf`, etc.).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub license: Option<String>,
    /// Source page URL — where to credit / view the clip.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_page: Option<String>,
}

/// Cluster trait shared by every stock-search adapter.
pub trait StockSearchBackend {
    /// Provider name (`"pexels"`, `"pond5"`).
    fn name(&self) -> &'static str;

    /// Estimate the cost of a request. Stock providers are typically
    /// free / unlimited; expect `0.0` for this cluster.
    fn estimate_cost(&self, request: &StockSearchRequest) -> CostEstimate;

    /// Execute the search.
    fn search(
        &self,
        request: &StockSearchRequest,
        mode: RunMode,
    ) -> Result<BackendCallOutcome<StockSearchResult>, BackendError>;
}

// Re-export the centralized budget gate so existing adapter code still
// compiles against `crate::backends::stock::check_budget`.
pub(crate) use crate::backends::check_budget;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_round_trips_through_json() {
        let req = StockSearchRequest {
            query: "ocean".into(),
            orientation: Some(Orientation::Landscape),
            min_duration_secs: Some(3),
            max_duration_secs: Some(30),
            per_page: 5,
            page: 2,
        };
        let json = serde_json::to_string(&req).unwrap();
        let back: StockSearchRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(back.query, "ocean");
        assert_eq!(back.per_page, 5);
        assert_eq!(back.orientation, Some(Orientation::Landscape));
    }

    // Centralized budget gate is now covered by `backends::mod.rs` tests.
}
