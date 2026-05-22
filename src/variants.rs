//! N-variant generation per shot — roll K candidates, judge, keep the winner.
//!
//! The community pattern around mid-2026 SOTA t2i/i2v workflows is to roll
//! K candidates per shot and pick the best one; one-shot picks are too
//! noisy. This module provides the seed-enumeration + parallel-orchestration
//! + selection-policy infrastructure. The single-dim VLM scorer wired below
//! is the May-2026 baseline; the follow-up `wb-kwac` (VISTA pairwise
//! tournament) swaps in multi-dim pairwise judging behind the same
//! `--select` surface.
//!
//! ## Layering
//!
//! Sits ABOVE the existing `RefConditionedImgGen` / `Txt2VidGen` /
//! `Img2VidGen` traits — those signatures are unchanged. Variant
//! orchestration calls the per-seed gen N times via the same trait and
//! aggregates the outcomes.
//!
//! ## Parallelism
//!
//! Uses `std::thread::scope` over `Send + Sync` adapter clones — no tokio,
//! no rayon, no new deps. Each adapter call is a blocking `ureq` POST, so
//! N threads × ~one-RPC each is the right granularity. Cache hits collapse
//! to ~ms each.
//!
//! ## Cost gating
//!
//! `--max-cost` continues to apply per-call. `--max-variants-cost` (when
//! set) is the aggregate ceiling across all N variants; the check runs
//! before any network call.

use serde::{Deserialize, Serialize};

/// Hard ceiling — the spec says "default 1, max 8". Beyond this is silly
/// (N × cost grows linearly, marginal quality gain saturates by 4-6).
pub const MAX_VARIANTS: u32 = 8;

/// Selection policy — how to pick the winner from N variant outcomes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SelectPolicy {
    /// Production default — VLM-grade each variant against the brief's
    /// criteria, pick the highest pass-rate. Tie-break by first-by-seed.
    MaxVlm,
    /// VISTA-style (arXiv 2510.15831) — pairwise structured comparison
    /// across 4 dimensions (subject fidelity, composition, lighting +
    /// color, production polish), single-elimination bracket. Costs
    /// N-1 VLM calls vs `max-vlm`'s N, but the per-pair JSON is much
    /// richer than a single pass-count score.
    PairwiseTournament,
    /// Debug / mock — take variant 0.
    First,
    /// User-driven — emit all N + a manifest; the agent chooses
    /// interactively. The "winner" field is left as `None`.
    User,
    /// Debug only — pick whichever cached fastest. Tie-break by
    /// first-by-seed.
    Cheapest,
}

impl SelectPolicy {
    /// Parse from the CLI flag value.
    pub fn parse(s: &str) -> Result<Self, String> {
        match s {
            "max-vlm" => Ok(Self::MaxVlm),
            "pairwise-tournament" => Ok(Self::PairwiseTournament),
            "first" => Ok(Self::First),
            "user" => Ok(Self::User),
            "cheapest" => Ok(Self::Cheapest),
            other => Err(format!(
                "unknown --select '{other}', want one of: max-vlm | pairwise-tournament | first | user | cheapest"
            )),
        }
    }
}

/// Enumerate the seeds used for the N variants. Returns `[base, base+1,
/// ..., base+n-1]`. When `base` is `None`, defaults to `0` so the cache
/// key is reproducible across re-runs.
pub fn enumerate_seeds(base: Option<u64>, n: u32) -> Vec<u64> {
    let start = base.unwrap_or(0);
    (0..n as u64).map(|i| start.wrapping_add(i)).collect()
}

/// Per-variant outcome record. Successful gens carry a serialized response
/// payload + cost; errored gens carry the error string so the manifest
/// still records the seed that failed.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VariantRecord<R> {
    /// Variant index, `0..N`.
    pub index: u32,
    /// Seed used for this variant.
    pub seed: u64,
    /// Successful response, when the gen completed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response: Option<R>,
    /// Provider id (when known).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    /// Request hash (cache key).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request_hash: Option<String>,
    /// True iff the response came from cache.
    pub cached: bool,
    /// Cost estimate at request time. Cache hits report `0.0`.
    pub cost_estimate_usd: f32,
    /// Wall-clock elapsed ms for this variant — used by `cheapest`.
    pub elapsed_ms: u64,
    /// VLM pass count for `max-vlm`. `None` when the policy didn't run
    /// VLM (debug policies / clip variants when VLM isn't wired).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vlm_pass_count: Option<u32>,
    /// VLM total criteria count — denominator for the pass rate.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vlm_total: Option<u32>,
    /// Error string when the gen failed. Mutually exclusive with
    /// `response`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl<R> VariantRecord<R> {
    /// True when the gen call completed and we have a payload.
    pub fn is_success(&self) -> bool {
        self.response.is_some()
    }
}

/// Aggregate manifest emitted by every variant run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VariantManifest<R> {
    /// Selection policy used.
    pub select: SelectPolicy,
    /// Number of variants requested.
    pub requested: u32,
    /// Number of variants that produced a response.
    pub succeeded: u32,
    /// Aggregate cost estimate across all variants, USD.
    pub total_cost_usd: f32,
    /// Per-variant records in seed order.
    pub variants: Vec<VariantRecord<R>>,
    /// Winner's `index` in `variants`. `None` when no variant succeeded
    /// or policy is `user`.
    pub winner: Option<u32>,
    /// Why the winner was picked — human-readable note for the agent.
    pub winner_reason: String,
    /// Per-round bracket history when `select` is `pairwise-tournament`.
    /// Empty for all other policies.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub bracket: Vec<BracketRound>,
}

/// Cost-gate result.
#[derive(Debug, Clone)]
pub enum CostGate {
    /// Cleared the gate — proceed.
    Pass {
        /// Aggregate estimate in USD.
        estimated_usd: f32,
    },
    /// Estimate exceeds the aggregate ceiling.
    Block {
        /// Aggregate estimate in USD.
        estimated_usd: f32,
        /// Ceiling that was breached, USD.
        ceiling_usd: f32,
    },
}

/// Check the aggregate cost ceiling. Returns `Block` when `per_variant ×
/// n` exceeds `max_variants_cost` (when set), `Pass` otherwise. The
/// per-call `--max-cost` budget continues to apply inside each gen
/// adapter — this is the additional aggregate ceiling on top.
pub fn check_aggregate_cost(
    per_variant_usd: f32,
    n: u32,
    max_variants_cost: Option<f32>,
) -> CostGate {
    let estimated = per_variant_usd * n as f32;
    match max_variants_cost {
        Some(ceiling) if estimated > ceiling => CostGate::Block {
            estimated_usd: estimated,
            ceiling_usd: ceiling,
        },
        _ => CostGate::Pass {
            estimated_usd: estimated,
        },
    }
}

/// Pick the winner index given the policy + records. Returns `None` when
/// no variant succeeded, or the policy is `User`.
pub fn select_winner<R>(
    policy: SelectPolicy,
    variants: &[VariantRecord<R>],
) -> Option<u32> {
    let successes: Vec<&VariantRecord<R>> =
        variants.iter().filter(|v| v.is_success()).collect();
    if successes.is_empty() {
        return None;
    }
    match policy {
        SelectPolicy::User => None,
        // The bracket runner picks the winner externally; this function
        // returns `None` so the CLI driver knows to overwrite it.
        SelectPolicy::PairwiseTournament => None,
        SelectPolicy::First => Some(successes[0].index),
        SelectPolicy::Cheapest => {
            let mut best = successes[0];
            for v in successes.iter().skip(1) {
                if v.elapsed_ms < best.elapsed_ms
                    || (v.elapsed_ms == best.elapsed_ms && v.seed < best.seed)
                {
                    best = v;
                }
            }
            Some(best.index)
        }
        SelectPolicy::MaxVlm => {
            // Highest VLM pass count wins; tie-break by first-by-seed.
            // Variants without VLM data score as 0 (still beats true
            // failures, since failures aren't in `successes`).
            let scored: Vec<(&VariantRecord<R>, u32)> = successes
                .iter()
                .map(|v| (*v, v.vlm_pass_count.unwrap_or(0)))
                .collect();
            let max_score = scored.iter().map(|(_, s)| *s).max().unwrap_or(0);
            let mut tied: Vec<&VariantRecord<R>> = scored
                .iter()
                .filter(|(_, s)| *s == max_score)
                .map(|(v, _)| *v)
                .collect();
            tied.sort_by_key(|v| v.seed);
            Some(tied[0].index)
        }
    }
}

/// Default verification criteria when the shot didn't supply its own.
/// Drives `max-vlm` for `scene-still` and `shot still` without forcing
/// callers to author a brief upfront.
pub fn default_criteria(subject: Option<&str>) -> Vec<String> {
    let subj = subject.unwrap_or("the intended subject");
    vec![
        format!("the subject is clearly {subj}"),
        "no baked-in text or watermarks".into(),
        "no extra limbs or duplicated body parts".into(),
        "the image is in focus".into(),
    ]
}

/// Format the pre-call line every variant verb prints.
pub fn estimate_line(n: u32, per_call_usd: f32) -> String {
    format!(
        "variants={n} estimated cost = ${:.4} ({n} × ${:.4})",
        per_call_usd * n as f32,
        per_call_usd
    )
}

/// Number of pairwise VLM calls needed to crown a champion across `n`
/// candidates with single-elimination. With byes, a tournament of N
/// candidates is always exactly `N - 1` matches (each match eliminates
/// exactly one). `0` and `1` need no calls.
pub fn pairwise_call_count(n: u32) -> u32 {
    n.saturating_sub(1)
}

/// Format the pre-call estimate line for `pairwise-tournament`. The
/// gen cost grows linearly with N; the judging cost grows as N - 1.
pub fn pairwise_estimate_line(
    n: u32,
    per_call_usd: f32,
    per_judge_usd: f32,
) -> String {
    let gen = per_call_usd * n as f32;
    let pairs = pairwise_call_count(n);
    let judge = per_judge_usd * pairs as f32;
    format!(
        "variants={n} pairwise gen=${gen:.4} + judging=${judge:.4} = ${:.4}",
        gen + judge
    )
}

/// One per-dimension verdict in a pairwise comparison.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PairVerdict {
    /// First candidate (A) wins this dimension.
    A,
    /// Second candidate (B) wins this dimension.
    B,
    /// Indistinguishable on this dimension.
    Tie,
}

/// The four VISTA dimensions the VLM grades per pair.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PairJudgments {
    /// Does the candidate match the brief's specified subject — right
    /// product, right identity, right brand.
    pub subject_fidelity: PairVerdict,
    /// Rule-of-thirds, negative space, framing — not centered-and-flat.
    pub composition: PairVerdict,
    /// Coherent with the prompted atmosphere, no plastic / no overcooked.
    pub lighting_color: PairVerdict,
    /// Sharpness, no artifacts, no anatomy errors.
    pub production: PairVerdict,
    /// One sentence per dimension, max four sentences total. Free-form.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub rationale: String,
}

/// Aggregate a `PairJudgments` block into a single `PairVerdict` for the
/// pair. Rules (VISTA-style):
///
/// - A wins if A wins ≥ 3 of 4 dims, OR `(A_wins == 2 && ties == 2)`.
/// - B wins by the symmetric rule.
/// - Otherwise (genuine deadlock — e.g. 2 wins each, or 1+1+2t) the
///   pair returns `Tie`; the bracket runner falls back to lower-seed.
pub fn aggregate_pair(j: &PairJudgments) -> PairVerdict {
    let dims = [
        j.subject_fidelity,
        j.composition,
        j.lighting_color,
        j.production,
    ];
    let a_wins = dims.iter().filter(|v| matches!(v, PairVerdict::A)).count();
    let b_wins = dims.iter().filter(|v| matches!(v, PairVerdict::B)).count();
    let ties = dims.iter().filter(|v| matches!(v, PairVerdict::Tie)).count();
    let a_decisive = a_wins >= 3 || (a_wins == 2 && ties == 2);
    let b_decisive = b_wins >= 3 || (b_wins == 2 && ties == 2);
    match (a_decisive, b_decisive) {
        (true, false) => PairVerdict::A,
        (false, true) => PairVerdict::B,
        _ => PairVerdict::Tie,
    }
}

/// One round of the single-elimination bracket — two competing seeds,
/// the dimension-level judgments, and the chosen winner's seed.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BracketRound {
    /// Round label — `"semi-1"`, `"semi-2"`, `"final"`, `"qf-1"`, …
    pub round: String,
    /// Seed of candidate A in this pair.
    pub a_seed: u64,
    /// Variant index of A in the `variants` array.
    pub a_index: u32,
    /// Seed of candidate B in this pair.
    pub b_seed: u64,
    /// Variant index of B.
    pub b_index: u32,
    /// Winning seed.
    pub winner: u64,
    /// Winning variant index.
    pub winner_index: u32,
    /// `true` when the pair-aggregator returned `Tie` and the runner
    /// fell back to lower-seed.
    #[serde(default)]
    pub seed_tiebreak: bool,
    /// Per-dimension verdicts from the VLM. `None` only when the pair
    /// was a bye round (one slot unfilled).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub judgments: Option<PairJudgments>,
}

/// Build the round-label progression for a bracket over `n` candidates.
/// Returns labels for each *match*, in the order the runner will play
/// them — `["qf-1", "qf-2", "qf-3", "qf-4", "semi-1", "semi-2", "final"]`
/// at N=8, `["semi-1", "semi-2", "final"]` at N=4, etc. With byes,
/// odd-N rounds advance the trailing seed unmatched.
pub fn round_labels(n: u32) -> Vec<String> {
    if n <= 1 {
        return Vec::new();
    }
    let mut labels = Vec::new();
    let mut surviving = n;
    while surviving > 1 {
        let matches_this_round = surviving / 2;
        let next = surviving - matches_this_round;
        let label = match surviving {
            2 => "final".to_string(),
            3 | 4 => "semi".to_string(),
            5..=8 => "quarter".to_string(),
            _ => format!("r{surviving}"),
        };
        for i in 0..matches_this_round {
            if surviving == 2 {
                labels.push("final".to_string());
            } else {
                labels.push(format!("{label}-{}", i + 1));
            }
        }
        surviving = next;
    }
    labels
}

/// Run a single-elimination bracket over the per-variant indices using
/// a caller-supplied pair-judge closure. The closure receives `(a, b)`
/// where each is an `(index, seed)` pair, and returns the dimension-
/// level judgments. The runner aggregates each pair, records the
/// round, advances the winner, and crowns the survivor.
///
/// Inputs are expected to be the *successful* variant indices in seed
/// order — failed variants are filtered out by the caller. With an odd
/// count, the trailing competitor gets a bye and advances unmatched.
///
/// Returns `(champion_index, bracket_history)`. The history is empty
/// when there are 0 or 1 competitors.
pub fn run_pairwise_bracket<F>(
    competitors: &[(u32, u64)],
    mut pair_judge: F,
) -> Result<(Option<u32>, Vec<BracketRound>), String>
where
    F: FnMut(u32, u64, u32, u64) -> Result<PairJudgments, String>,
{
    if competitors.is_empty() {
        return Ok((None, Vec::new()));
    }
    if competitors.len() == 1 {
        return Ok((Some(competitors[0].0), Vec::new()));
    }
    let labels = round_labels(competitors.len() as u32);
    let mut label_iter = labels.into_iter();
    let mut history: Vec<BracketRound> = Vec::new();
    let mut current: Vec<(u32, u64)> = competitors.to_vec();

    while current.len() > 1 {
        let mut next_round: Vec<(u32, u64)> = Vec::new();
        let mut pairs = current.chunks_exact(2);
        for pair in pairs.by_ref() {
            let (ai, aseed) = pair[0];
            let (bi, bseed) = pair[1];
            let judgments = pair_judge(ai, aseed, bi, bseed)?;
            let agg = aggregate_pair(&judgments);
            let (winner_index, winner_seed, seed_tiebreak) = match agg {
                PairVerdict::A => (ai, aseed, false),
                PairVerdict::B => (bi, bseed, false),
                PairVerdict::Tie => {
                    // Lower seed wins.
                    if aseed <= bseed {
                        (ai, aseed, true)
                    } else {
                        (bi, bseed, true)
                    }
                }
            };
            let label = label_iter.next().unwrap_or_else(|| "match".into());
            history.push(BracketRound {
                round: label,
                a_seed: aseed,
                a_index: ai,
                b_seed: bseed,
                b_index: bi,
                winner: winner_seed,
                winner_index,
                seed_tiebreak,
                judgments: Some(judgments),
            });
            next_round.push((winner_index, winner_seed));
        }
        // Odd-count bye: trailing competitor advances unmatched.
        if let [bye] = pairs.remainder() {
            next_round.push(*bye);
        }
        current = next_round;
    }
    Ok((Some(current[0].0), history))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(index: u32, seed: u64, success: bool) -> VariantRecord<&'static str> {
        VariantRecord {
            index,
            seed,
            response: if success { Some("ok") } else { None },
            provider: None,
            request_hash: None,
            cached: false,
            cost_estimate_usd: 0.04,
            elapsed_ms: 0,
            vlm_pass_count: None,
            vlm_total: None,
            error: if success { None } else { Some("oops".into()) },
        }
    }

    #[test]
    fn enumerate_seeds_returns_n_distinct() {
        let s = enumerate_seeds(Some(100), 3);
        assert_eq!(s, vec![100, 101, 102]);
        let s2 = enumerate_seeds(None, 4);
        assert_eq!(s2, vec![0, 1, 2, 3]);
    }

    #[test]
    fn enumerate_seeds_zero_n_is_empty() {
        assert!(enumerate_seeds(Some(7), 0).is_empty());
    }

    #[test]
    fn parse_policy_round_trip() {
        assert_eq!(SelectPolicy::parse("max-vlm").unwrap(), SelectPolicy::MaxVlm);
        assert_eq!(SelectPolicy::parse("first").unwrap(), SelectPolicy::First);
        assert_eq!(SelectPolicy::parse("user").unwrap(), SelectPolicy::User);
        assert_eq!(
            SelectPolicy::parse("cheapest").unwrap(),
            SelectPolicy::Cheapest
        );
        assert!(SelectPolicy::parse("bogus").is_err());
    }

    #[test]
    fn first_policy_picks_index_zero() {
        let v = vec![rec(0, 100, true), rec(1, 101, true), rec(2, 102, true)];
        assert_eq!(select_winner(SelectPolicy::First, &v), Some(0));
    }

    #[test]
    fn first_policy_skips_failures() {
        let v = vec![rec(0, 100, false), rec(1, 101, true), rec(2, 102, true)];
        assert_eq!(select_winner(SelectPolicy::First, &v), Some(1));
    }

    #[test]
    fn user_policy_leaves_winner_blank() {
        let v = vec![rec(0, 100, true), rec(1, 101, true)];
        assert!(select_winner(SelectPolicy::User, &v).is_none());
    }

    #[test]
    fn max_vlm_picks_highest_pass_count() {
        let mut a = rec(0, 100, true);
        a.vlm_pass_count = Some(2);
        a.vlm_total = Some(4);
        let mut b = rec(1, 101, true);
        b.vlm_pass_count = Some(4);
        b.vlm_total = Some(4);
        let mut c = rec(2, 102, true);
        c.vlm_pass_count = Some(3);
        c.vlm_total = Some(4);
        let v = vec![a, b, c];
        assert_eq!(select_winner(SelectPolicy::MaxVlm, &v), Some(1));
    }

    #[test]
    fn max_vlm_tie_break_by_seed() {
        let mut a = rec(0, 200, true);
        a.vlm_pass_count = Some(3);
        a.vlm_total = Some(4);
        let mut b = rec(1, 100, true);
        b.vlm_pass_count = Some(3);
        b.vlm_total = Some(4);
        let mut c = rec(2, 150, true);
        c.vlm_pass_count = Some(3);
        c.vlm_total = Some(4);
        let v = vec![a, b, c];
        // Lowest seed wins on tie.
        assert_eq!(select_winner(SelectPolicy::MaxVlm, &v), Some(1));
    }

    #[test]
    fn max_vlm_with_one_error_picks_from_successes() {
        let a = rec(0, 100, false);
        let mut b = rec(1, 101, true);
        b.vlm_pass_count = Some(2);
        let mut c = rec(2, 102, true);
        c.vlm_pass_count = Some(4);
        let v = vec![a, b, c];
        assert_eq!(select_winner(SelectPolicy::MaxVlm, &v), Some(2));
    }

    #[test]
    fn cheapest_picks_fastest_elapsed() {
        let mut a = rec(0, 100, true);
        a.elapsed_ms = 1200;
        let mut b = rec(1, 101, true);
        b.elapsed_ms = 800;
        let mut c = rec(2, 102, true);
        c.elapsed_ms = 1500;
        let v = vec![a, b, c];
        assert_eq!(select_winner(SelectPolicy::Cheapest, &v), Some(1));
    }

    #[test]
    fn select_winner_none_when_all_failed() {
        let v = vec![rec(0, 100, false), rec(1, 101, false)];
        assert!(select_winner(SelectPolicy::MaxVlm, &v).is_none());
        assert!(select_winner(SelectPolicy::First, &v).is_none());
        assert!(select_winner(SelectPolicy::Cheapest, &v).is_none());
    }

    #[test]
    fn cost_gate_passes_under_ceiling() {
        match check_aggregate_cost(0.04, 3, Some(0.20)) {
            CostGate::Pass { estimated_usd } => {
                assert!((estimated_usd - 0.12).abs() < 1e-6);
            }
            CostGate::Block { .. } => panic!("should pass"),
        }
    }

    #[test]
    fn cost_gate_blocks_over_ceiling() {
        // variants=3 × $0.05 = $0.15 > $0.10
        match check_aggregate_cost(0.05, 3, Some(0.10)) {
            CostGate::Block {
                estimated_usd,
                ceiling_usd,
            } => {
                assert!((estimated_usd - 0.15).abs() < 1e-6);
                assert!((ceiling_usd - 0.10).abs() < 1e-6);
            }
            CostGate::Pass { .. } => panic!("should block"),
        }
    }

    #[test]
    fn cost_gate_no_ceiling_always_passes() {
        match check_aggregate_cost(1000.0, 8, None) {
            CostGate::Pass { .. } => {}
            CostGate::Block { .. } => panic!("no ceiling = no block"),
        }
    }

    #[test]
    fn default_criteria_subs_in_subject() {
        let c = default_criteria(Some("the green Porsche 911"));
        assert!(c[0].contains("green Porsche"));
        assert_eq!(c.len(), 4);
    }

    #[test]
    fn default_criteria_handles_missing_subject() {
        let c = default_criteria(None);
        assert!(c[0].contains("intended subject"));
    }

    #[test]
    fn estimate_line_format() {
        let line = estimate_line(3, 0.04);
        assert!(line.contains("variants=3"));
        assert!(line.contains("$0.1200"));
        assert!(line.contains("$0.0400"));
    }

    fn judg(
        sf: PairVerdict,
        comp: PairVerdict,
        lc: PairVerdict,
        prod: PairVerdict,
    ) -> PairJudgments {
        PairJudgments {
            subject_fidelity: sf,
            composition: comp,
            lighting_color: lc,
            production: prod,
            rationale: String::new(),
        }
    }

    #[test]
    fn parse_pairwise_tournament_policy() {
        assert_eq!(
            SelectPolicy::parse("pairwise-tournament").unwrap(),
            SelectPolicy::PairwiseTournament
        );
    }

    #[test]
    fn aggregate_pair_a_sweep_three_of_four() {
        let j = judg(PairVerdict::A, PairVerdict::A, PairVerdict::A, PairVerdict::B);
        assert_eq!(aggregate_pair(&j), PairVerdict::A);
    }

    #[test]
    fn aggregate_pair_two_wins_and_two_ties_is_decisive() {
        // {A, B, tie, tie} → 1 + 0 + 2 = nobody hits the 2W+2T rule (A has 1, not 2).
        let j = judg(PairVerdict::A, PairVerdict::B, PairVerdict::Tie, PairVerdict::Tie);
        assert_eq!(aggregate_pair(&j), PairVerdict::Tie);

        // {A, A, tie, tie} → A wins 2 with 2 ties → A wins.
        let j2 = judg(PairVerdict::A, PairVerdict::A, PairVerdict::Tie, PairVerdict::Tie);
        assert_eq!(aggregate_pair(&j2), PairVerdict::A);
    }

    #[test]
    fn aggregate_pair_all_ties_is_tie() {
        let j = judg(PairVerdict::Tie, PairVerdict::Tie, PairVerdict::Tie, PairVerdict::Tie);
        assert_eq!(aggregate_pair(&j), PairVerdict::Tie);
    }

    #[test]
    fn aggregate_pair_two_vs_two_is_tie() {
        let j = judg(PairVerdict::A, PairVerdict::A, PairVerdict::B, PairVerdict::B);
        assert_eq!(aggregate_pair(&j), PairVerdict::Tie);
    }

    #[test]
    fn aggregate_pair_b_sweep() {
        let j = judg(PairVerdict::B, PairVerdict::B, PairVerdict::B, PairVerdict::B);
        assert_eq!(aggregate_pair(&j), PairVerdict::B);
    }

    #[test]
    fn pairwise_call_count_is_n_minus_one() {
        assert_eq!(pairwise_call_count(0), 0);
        assert_eq!(pairwise_call_count(1), 0);
        assert_eq!(pairwise_call_count(2), 1);
        assert_eq!(pairwise_call_count(4), 3);
        assert_eq!(pairwise_call_count(7), 6);
        assert_eq!(pairwise_call_count(8), 7);
    }

    #[test]
    fn pairwise_estimate_line_includes_gen_and_judging() {
        // 4 variants × $0.04 gen + 3 pairs × $0.01 judge = $0.16 + $0.03 = $0.19
        let line = pairwise_estimate_line(4, 0.04, 0.01);
        assert!(line.contains("variants=4"));
        assert!(line.contains("gen=$0.1600"));
        assert!(line.contains("judging=$0.0300"));
        assert!(line.contains("= $0.1900"));
    }

    #[test]
    fn round_labels_for_four_is_semi_semi_final() {
        let l = round_labels(4);
        assert_eq!(l, vec!["semi-1", "semi-2", "final"]);
    }

    #[test]
    fn round_labels_for_two_is_just_final() {
        assert_eq!(round_labels(2), vec!["final"]);
    }

    #[test]
    fn round_labels_for_eight_quarters_semis_final() {
        let l = round_labels(8);
        assert_eq!(
            l,
            vec![
                "quarter-1", "quarter-2", "quarter-3", "quarter-4", "semi-1", "semi-2", "final",
            ]
        );
    }

    #[test]
    fn round_labels_one_or_zero_empty() {
        assert!(round_labels(0).is_empty());
        assert!(round_labels(1).is_empty());
    }

    #[test]
    fn bracket_runner_four_variant_picks_decisive_winner() {
        // seeds 100-103. Force: 100 sweeps 101 (semi-1), 103 sweeps 102
        // (semi-2), then 103 sweeps 100 (final).
        let competitors = vec![(0u32, 100u64), (1, 101), (2, 102), (3, 103)];
        let (champ, history) = run_pairwise_bracket(&competitors, |_ai, aseed, _bi, bseed| {
            // 100 beats 101; 103 beats 102; 103 beats 100.
            let aw = match (aseed, bseed) {
                (100, 101) => PairVerdict::A,
                (102, 103) => PairVerdict::B,
                (100, 103) => PairVerdict::B,
                _ => panic!("unexpected pair: {aseed} vs {bseed}"),
            };
            Ok(judg(aw, aw, aw, aw))
        })
        .unwrap();
        assert_eq!(champ, Some(3));
        assert_eq!(history.len(), 3);
        assert_eq!(history[0].round, "semi-1");
        assert_eq!(history[0].winner, 100);
        assert_eq!(history[1].round, "semi-2");
        assert_eq!(history[1].winner, 103);
        assert_eq!(history[2].round, "final");
        assert_eq!(history[2].winner, 103);
        assert!(!history.iter().any(|r| r.seed_tiebreak));
    }

    #[test]
    fn bracket_runner_seed_tiebreak_when_pair_ties() {
        let competitors = vec![(0u32, 50u64), (1, 25)];
        let (champ, history) = run_pairwise_bracket(&competitors, |_, _, _, _| {
            Ok(judg(PairVerdict::Tie, PairVerdict::Tie, PairVerdict::Tie, PairVerdict::Tie))
        })
        .unwrap();
        // Tie → lower seed wins. 25 < 50, so index 1 wins.
        assert_eq!(champ, Some(1));
        assert_eq!(history.len(), 1);
        assert!(history[0].seed_tiebreak);
        assert_eq!(history[0].winner, 25);
    }

    #[test]
    fn bracket_runner_three_variant_has_bye() {
        // Three competitors: pair (0, 1) plays; (2) gets a bye into the
        // final. Then final between winner of pair and (2).
        let competitors = vec![(0u32, 100u64), (1, 101), (2, 102)];
        let (champ, history) = run_pairwise_bracket(&competitors, |_, aseed, _, bseed| {
            // 100 beats 101 in the only pair; then 102 beats 100 in final.
            let aw = match (aseed, bseed) {
                (100, 101) => PairVerdict::A,
                (100, 102) => PairVerdict::B,
                _ => panic!("unexpected: {aseed} vs {bseed}"),
            };
            Ok(judg(aw, aw, aw, aw))
        })
        .unwrap();
        assert_eq!(champ, Some(2));
        assert_eq!(history.len(), 2);
    }

    #[test]
    fn bracket_runner_single_competitor_is_champion() {
        let competitors = vec![(0u32, 100u64)];
        let (champ, history) =
            run_pairwise_bracket(&competitors, |_, _, _, _| {
                Ok(judg(PairVerdict::A, PairVerdict::A, PairVerdict::A, PairVerdict::A))
            })
            .unwrap();
        assert_eq!(champ, Some(0));
        assert!(history.is_empty());
    }

    #[test]
    fn bracket_runner_empty_yields_no_champion() {
        let competitors: Vec<(u32, u64)> = Vec::new();
        let (champ, history) =
            run_pairwise_bracket(&competitors, |_, _, _, _| panic!("never called"))
                .unwrap();
        assert!(champ.is_none());
        assert!(history.is_empty());
    }

    #[test]
    fn pairwise_tournament_select_winner_returns_none_for_external_decision() {
        // PairwiseTournament defers winner picking to the bracket runner;
        // select_winner returns None so the CLI overwrites it.
        let mut a = rec(0, 100, true);
        a.vlm_pass_count = Some(2);
        let b = rec(1, 101, true);
        let v = vec![a, b];
        assert!(select_winner(SelectPolicy::PairwiseTournament, &v).is_none());
    }

    #[test]
    fn cost_gate_pairwise_aggregate_check() {
        // 4 variants × $0.04 gen + 3 pairs × $0.01 judge = $0.19
        // Pass under $0.25, block at $0.15.
        let total_per_variant_gen = 0.04;
        let judging = 0.01 * pairwise_call_count(4) as f32;
        let aggregate_per_call = total_per_variant_gen + judging / 4.0;
        // Wired in CLI as: check_aggregate_cost(per_variant_gen + per_judge_share, n, ceil)
        // here we just confirm the math the CLI will compose.
        let total = total_per_variant_gen * 4.0 + judging;
        assert!((total - 0.19).abs() < 1e-6);
        let _ = aggregate_per_call;
    }
}
