//! Synthetic equal-pacing fallback for the captions cluster.
//!
//! No network call. Splits the VO `text` into whitespace-delimited
//! words, then distributes the requested `duration_ms` evenly across
//! them: each word's dwell is `duration_ms / word_count`. Used when
//! the real ASR backend is unavailable (dry-run, no `FAL_KEY`, offline)
//! or when the caller explicitly opts in via `--backend synthetic`.
//!
//! Crude — natural speech has uneven dwell (stressed syllables and
//! line-final words stretch; conjunctions shrink). Still good enough as
//! a v0 fallback because the overlay generator's CSS keyframes only
//! need *some* timing to drive word-by-word reveals.

use super::{tokenize_words, CaptionsBackend, CaptionsRequest, CaptionsResult, WordTimestamp, CLUSTER};
use crate::backends::{
    check_budget, mode_label, BackendCallOutcome, BackendError, CostEstimate, RunMode,
};

/// Provider identifier.
pub const PROVIDER: &str = "synthetic-equal-pacing";

/// Synthetic equal-pacing adapter. Stateless; no client needed.
#[derive(Debug, Clone, Default)]
pub struct SyntheticEqualPacingAdapter;

impl SyntheticEqualPacingAdapter {
    /// Construct the adapter.
    pub fn new() -> Self {
        Self
    }
}

impl CaptionsBackend for SyntheticEqualPacingAdapter {
    fn name(&self) -> &'static str {
        PROVIDER
    }

    fn estimate_cost(&self, _request: &CaptionsRequest) -> CostEstimate {
        CostEstimate {
            provider: PROVIDER.into(),
            cost_usd: 0.0,
            explanation: "synthetic equal-pacing — no API call".into(),
        }
    }

    fn captions(
        &self,
        request: &CaptionsRequest,
        mode: RunMode,
    ) -> Result<BackendCallOutcome<CaptionsResult>, BackendError> {
        if request.text.trim().is_empty() {
            return Err(BackendError::InvalidRequest(
                "synthetic captions need --text to know what to word-split".into(),
            ));
        }
        if request.duration_ms == 0 {
            return Err(BackendError::InvalidRequest(
                "synthetic captions need --duration-ms to compute per-word dwell".into(),
            ));
        }

        let estimate = self.estimate_cost(request);
        check_budget(&estimate, mode)?;

        let words = compute_equal_pacing(&request.text, request.duration_ms);
        let total_ms = words.last().map(|w| w.end_ms).unwrap_or(0);

        let response = CaptionsResult {
            provider: PROVIDER.into(),
            words,
            total_ms,
        };
        let request_hash =
            crate::backends::cache::AssetCache::request_hash(PROVIDER, CLUSTER, request)?;

        Ok(BackendCallOutcome {
            response,
            provider: PROVIDER.into(),
            request_hash,
            cached: false,
            cost_estimate_usd: 0.0,
            mode: mode_label(mode),
        })
    }
}

/// Distribute `duration_ms` evenly across the whitespace-split words of
/// `text`. Returned timestamps cover `[0, duration_ms)` end-to-end with
/// no gaps. Public so the unit tests and the CLI fallback path can
/// share the math.
pub fn compute_equal_pacing(text: &str, duration_ms: u32) -> Vec<WordTimestamp> {
    let tokens = tokenize_words(text);
    if tokens.is_empty() {
        return Vec::new();
    }
    let per = duration_ms as f64 / tokens.len() as f64;
    tokens
        .into_iter()
        .enumerate()
        .map(|(i, w)| {
            let start = (i as f64 * per).round() as u32;
            let end = ((i + 1) as f64 * per).round() as u32;
            WordTimestamp {
                word: w,
                start_ms: start,
                end_ms: end,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn equal_pacing_distributes_evenly() {
        let words = compute_equal_pacing("one two three four", 4000);
        assert_eq!(words.len(), 4);
        assert_eq!(words[0].start_ms, 0);
        assert_eq!(words[0].end_ms, 1000);
        assert_eq!(words[1].start_ms, 1000);
        assert_eq!(words[3].end_ms, 4000);
    }

    #[test]
    fn equal_pacing_handles_uneven_division() {
        let words = compute_equal_pacing("a b c", 1000);
        assert_eq!(words.len(), 3);
        assert_eq!(words[0].start_ms, 0);
        assert!(words[2].end_ms == 1000 || words[2].end_ms == 999);
    }

    #[test]
    fn synthetic_rejects_empty_text() {
        let req = CaptionsRequest::new("https://x/a.wav", "   ").with_duration_ms(1000);
        let err = SyntheticEqualPacingAdapter::new()
            .captions(&req, RunMode::DryRun)
            .unwrap_err();
        assert!(matches!(err, BackendError::InvalidRequest(_)));
    }

    #[test]
    fn synthetic_rejects_zero_duration() {
        let req = CaptionsRequest::new("https://x/a.wav", "hello");
        let err = SyntheticEqualPacingAdapter::new()
            .captions(&req, RunMode::DryRun)
            .unwrap_err();
        assert!(matches!(err, BackendError::InvalidRequest(_)));
    }

    #[test]
    fn synthetic_produces_monotonic_timestamps() {
        let req = CaptionsRequest::new("https://x/a.wav", "the spoken line of five words")
            .with_duration_ms(2500);
        let out = SyntheticEqualPacingAdapter::new()
            .captions(&req, RunMode::DryRun)
            .unwrap();
        assert!(out.response.is_monotonic());
        assert_eq!(out.response.words.len(), 6);
        assert_eq!(out.response.words[0].word, "the");
    }
}
