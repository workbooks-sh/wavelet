//! Script-duration coherence check — runs at the screenplay stage so
//! over-stuffed copy can't slip into paid generation.
//!
//! Premise: when the brief declares a 12-second spot and the script
//! contains 70 words of VO + 5 captions, the author wrote the wrong
//! copy. No amount of pacing or production polish recovers from that —
//! the message physically can't be delivered in the time available.
//! Catch it here, force a rewrite, save the Veo dollars.
//!
//! Budget math (intentionally simple; calibrated against the realistic
//! delivery rates a confident UGC-style read targets):
//!
//! - **Voiceover**  : words ÷ 2.5 wps  (≈ 150 WPM)
//! - **On-screen text dwell** : `max(words ÷ 4.0 wps, 1.2s per caption)`
//!   — silent reading is faster than speech, but every caption needs a
//!   minimum dwell to register.
//! - **Shot floor** : `max(shot_count × 1.0s, 1.5s hook minimum)` — a
//!   cut shorter than 1s reads as a glitch even at fast UGC pace.
//!
//! Verdict logic:
//!
//! - **fits**         — total ≤ declared × 1.10
//! - **under_budget** — total < declared × 0.60 (sparse; flag but pass)
//! - **over_budget**  — total > declared × 1.10 (hard fail; refuse)
//!
//! Author surface: `wavelet screenplay validate <fountain> --duration <secs>`
//! returns exit 0 on `fits` / `under_budget`, non-zero on `over_budget`.
//! Pipeline plumbing gates the storyboard stage behind this.

use fountain::{DialogueLine, Element, Screenplay};
use serde::{Deserialize, Serialize};

/// Spoken-word rate used to estimate VO time. 2.5 wps = 150 WPM —
/// a relaxed confident UGC-style delivery. Faster reads (hook lines,
/// 180-200 WPM) still fit because we allow 10% over budget.
pub const VO_WORDS_PER_SEC: f32 = 2.5;

/// Silent-reading rate for on-screen text. 4.0 wps = 240 WPM — the
/// floor of a competent native-English reader scanning short copy on
/// a moving frame. Captions get the larger of this estimate and a
/// per-caption minimum dwell.
pub const READ_WORDS_PER_SEC: f32 = 4.0;

/// Minimum on-screen time per caption to register. Below this the
/// viewer flicks past before reading.
pub const MIN_CAPTION_DWELL_SECS: f32 = 1.2;

/// Minimum dwell per cut. Anything faster is a glitch, not pacing.
pub const MIN_SHOT_SECS: f32 = 1.0;

/// Hook minimum — the first cut needs at least this much to land.
pub const MIN_HOOK_SECS: f32 = 1.5;

/// Tolerance band on the declared duration. Total budget within
/// `[0.6 × declared, 1.1 × declared]` is considered acceptable; outside
/// this band we either warn (under) or refuse (over).
/// Over-tolerance ratio — ratios above this trigger `OverBudget`.
pub const OVER_TOLERANCE: f32 = 1.10;
/// Under-tolerance ratio — ratios below this trigger `UnderBudget`.
pub const UNDER_TOLERANCE: f32 = 0.60;

/// Verdict of `evaluate` — one of these three outcomes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    /// Total budget within tolerance. Exit 0; pipeline advances.
    Fits,
    /// Total budget noticeably under declared duration — copy is
    /// thin. Exit 0 but the JSON output flags it for review.
    UnderBudget,
    /// Total budget exceeds the over-tolerance ceiling. Exit non-zero;
    /// pipeline blocks the next stage.
    OverBudget,
}

/// One element's contribution to the budget. Carried in the report so
/// the agent (or a human reading the JSON) sees which lines push it
/// over.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Contribution {
    /// Element category: `"voiceover"`, `"dialogue"`, `"caption"`, or
    /// `"shot_floor"`.
    pub kind: String,
    /// Source-side identifier — character name for dialogue, scene
    /// slugline for shot_floor, first ~40 chars of text for captions.
    pub source: String,
    /// Word count contributing to the estimate (0 for shot_floor).
    pub words: u32,
    /// Estimated seconds.
    pub seconds: f32,
}

/// Full result of evaluating one screenplay against one target
/// duration. Designed to round-trip JSON for the trace-based gate.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Report {
    /// Verdict for the full evaluation.
    pub verdict: Verdict,
    /// Target spot length from the brief.
    pub declared_secs: f32,
    /// Sum of all contributions.
    pub estimated_secs: f32,
    /// Total VO + on-camera dialogue seconds.
    pub voiceover_secs: f32,
    /// Total caption dwell seconds.
    pub captions_secs: f32,
    /// Shot floor seconds.
    pub shot_floor_secs: f32,
    /// Number of scene headings in the screenplay.
    pub shot_count: u32,
    /// Per-element breakdown.
    pub contributions: Vec<Contribution>,
    /// One-line fix direction tailored to the verdict.
    pub fix_hint: String,
}

impl Report {
    /// True if the verdict blocks the pipeline.
    pub fn blocks(&self) -> bool {
        matches!(self.verdict, Verdict::OverBudget)
    }
}

/// Evaluate `screenplay` against a target `declared_secs`. Returns a
/// `Report` summarizing the budget; caller decides what to do with the
/// verdict.
pub fn evaluate(screenplay: &Screenplay, declared_secs: f32) -> Report {
    let mut contributions: Vec<Contribution> = Vec::new();
    let mut voiceover_secs = 0.0f32;
    let mut captions_secs = 0.0f32;
    let mut shot_count = 0u32;

    for el in &screenplay.elements {
        match el {
            Element::SceneHeading { slugline, .. } => {
                shot_count += 1;
                contributions.push(Contribution {
                    kind: "shot_floor".into(),
                    source: slugline.clone(),
                    words: 0,
                    seconds: MIN_SHOT_SECS,
                });
            }
            Element::Dialogue {
                character,
                lines,
                is_voiceover,
                ..
            } => {
                let words = count_dialogue_words(lines);
                if words == 0 {
                    continue;
                }
                let seconds = words as f32 / VO_WORDS_PER_SEC;
                voiceover_secs += seconds;
                contributions.push(Contribution {
                    kind: if *is_voiceover {
                        "voiceover".into()
                    } else {
                        "dialogue".into()
                    },
                    source: character.clone(),
                    words,
                    seconds,
                });
            }
            Element::Action { text, .. } => {
                // Action paragraphs that look like overlay/caption
                // copy (short, declarative, often quoted) get counted
                // against the caption budget. Longer action prose
                // describes the SHOT and doesn't appear on-screen —
                // we skip it. Heuristic: <= 10 words OR contains a
                // quoted phrase.
                let stripped = text.trim();
                if stripped.is_empty() {
                    continue;
                }
                if !looks_like_caption(stripped) {
                    continue;
                }
                let words = count_words(stripped);
                if words == 0 {
                    continue;
                }
                let read_secs = words as f32 / READ_WORDS_PER_SEC;
                let dwell = read_secs.max(MIN_CAPTION_DWELL_SECS);
                captions_secs += dwell;
                contributions.push(Contribution {
                    kind: "caption".into(),
                    source: truncate_for_source(stripped, 40),
                    words,
                    seconds: dwell,
                });
            }
            _ => {}
        }
    }

    let shot_floor_secs = (shot_count as f32 * MIN_SHOT_SECS).max(MIN_HOOK_SECS);
    let estimated_secs = voiceover_secs + captions_secs + shot_floor_secs;

    let verdict = classify(estimated_secs, declared_secs);
    let fix_hint = build_fix_hint(verdict, estimated_secs, declared_secs, voiceover_secs);

    Report {
        verdict,
        declared_secs,
        estimated_secs,
        voiceover_secs,
        captions_secs,
        shot_floor_secs,
        shot_count,
        contributions,
        fix_hint,
    }
}

fn classify(estimated: f32, declared: f32) -> Verdict {
    if declared <= 0.0 {
        return Verdict::Fits;
    }
    let ratio = estimated / declared;
    if ratio > OVER_TOLERANCE {
        Verdict::OverBudget
    } else if ratio < UNDER_TOLERANCE {
        Verdict::UnderBudget
    } else {
        Verdict::Fits
    }
}

fn build_fix_hint(
    verdict: Verdict,
    estimated: f32,
    declared: f32,
    voiceover_secs: f32,
) -> String {
    match verdict {
        Verdict::Fits => format!(
            "estimated {estimated:.1}s fits within ±10% of {declared:.0}s — proceed."
        ),
        Verdict::UnderBudget => format!(
            "estimated {estimated:.1}s is well under {declared:.0}s — script may be \
             too thin to justify the spot length. Consider tightening to a shorter \
             format or adding one supporting beat."
        ),
        Verdict::OverBudget => {
            // Suggest a concrete word count to cut, derived from the
            // overshoot. If VO dominates, target VO; otherwise hint at
            // dropping captions or extending duration.
            let overshoot_secs = estimated - declared;
            let words_to_cut = (overshoot_secs * VO_WORDS_PER_SEC).ceil() as i32;
            if voiceover_secs > (estimated * 0.5) {
                format!(
                    "estimated {estimated:.1}s in a {declared:.0}s spot. Cut ~{words_to_cut} \
                     words of voiceover, OR convert half the VO to on-screen icons + \
                     captions stacked over a shorter VO bed, OR extend the spot to \
                     {target_secs:.0}s.",
                    target_secs = (estimated / OVER_TOLERANCE).ceil()
                )
            } else {
                format!(
                    "estimated {estimated:.1}s in a {declared:.0}s spot. Drop captions \
                     or trim shot count, OR extend the spot to {target_secs:.0}s.",
                    target_secs = (estimated / OVER_TOLERANCE).ceil()
                )
            }
        }
    }
}

fn count_dialogue_words(lines: &[DialogueLine]) -> u32 {
    let mut total = 0u32;
    for line in lines {
        match line {
            DialogueLine::Text(s) => total += count_words(s),
            // Parentheticals are direction notes, not spoken; lyrics
            // get the same treatment since they're musical accent.
            DialogueLine::Parenthetical(_) | DialogueLine::Lyric(_) => {}
        }
    }
    total
}

/// Word count by whitespace split, with HTML/markdown-y noise filtered.
fn count_words(s: &str) -> u32 {
    s.split_whitespace()
        .filter(|w| w.chars().any(|c| c.is_alphanumeric()))
        .count() as u32
}

/// Treat short or quoted action paragraphs as on-screen text.
fn looks_like_caption(s: &str) -> bool {
    let words = count_words(s);
    if words == 0 {
        return false;
    }
    if words <= 10 {
        return true;
    }
    // Quoted phrases (smart or straight quotes) are typically overlay
    // copy. Longer prose without quotes is scene description.
    s.contains('"') || s.contains('“') || s.contains('”')
}

fn truncate_for_source(s: &str, max: usize) -> String {
    let trimmed = s.trim();
    if trimmed.chars().count() <= max {
        return trimmed.to_string();
    }
    let cut: String = trimmed.chars().take(max).collect();
    format!("{cut}…")
}

#[cfg(test)]
mod tests {
    use super::*;
    use fountain::parse;

    #[test]
    fn fits_when_copy_matches_declared() {
        // ~25 words of VO at 2.5 wps = ~10s + 1.5s shot floor (2 shots)
        // = ~11.5s. Declared 12s. Should fit.
        let src = r#"
INT. KITCHEN - DAY

NARRATOR (V.O.)
Six speeds. One bowl. Built to last a lifetime — your grandmother had one, and so should you.

EXT. WINDOW - DAY

The mixer hums.
"#;
        let sp = parse(src).unwrap();
        let report = evaluate(&sp, 12.0);
        assert_eq!(report.verdict, Verdict::Fits, "report: {:?}", report);
        assert!(!report.blocks());
    }

    #[test]
    fn over_budget_when_copy_dwarfs_duration() {
        // ~80 words of VO at 2.5 wps = ~32s. Declared 12s. Hard fail.
        let src = r#"
INT. KITCHEN - DAY

NARRATOR (V.O.)
The KitchenAid Artisan stand mixer has been a kitchen staple since nineteen-thirty-seven, and today we're going to walk you through every single one of its ten speeds, the planetary mixing action that ensures complete ingredient incorporation, and the iconic tilt-head design that has defined home baking for nearly a century — there's truly no substitute for the real thing.
"#;
        let sp = parse(src).unwrap();
        let report = evaluate(&sp, 12.0);
        assert_eq!(report.verdict, Verdict::OverBudget, "report: {:?}", report);
        assert!(report.blocks());
        assert!(report.fix_hint.contains("words of voiceover"));
        assert!(report.estimated_secs > 12.0 * OVER_TOLERANCE);
    }

    #[test]
    fn under_budget_when_copy_too_thin() {
        // 4 words of VO + 1 shot = ~3s in a 20s spot. Under budget.
        let src = r#"
INT. KITCHEN - DAY

NARRATOR (V.O.)
Whisk it good.
"#;
        let sp = parse(src).unwrap();
        let report = evaluate(&sp, 20.0);
        assert_eq!(report.verdict, Verdict::UnderBudget, "report: {:?}", report);
        assert!(!report.blocks());
        assert!(report.fix_hint.contains("under"));
    }

    #[test]
    fn captions_count_against_budget() {
        // 4 captions × 1.2s floor each = 4.8s + 2 shot floor (2s) = ~6.8s.
        // Declared 8s. Should fit comfortably.
        let src = r#"
INT. STUDIO - DAY

"Six speeds."

"Tilt head."

"Made to last."

"Empire Red."

EXT. WINDOW - DAY

A close-up of the mixer.
"#;
        let sp = parse(src).unwrap();
        let report = evaluate(&sp, 8.0);
        // Caption count: 4 short quoted lines plus the scene description
        // (5 words, ≤10 → also counts as caption). 5 captions × 1.2 = 6s
        // + 2.0 shot floor = 8.0s. Right at the limit, fits.
        assert!(report.captions_secs >= 4.8, "report: {:?}", report);
        assert_eq!(report.verdict, Verdict::Fits, "report: {:?}", report);
    }

    #[test]
    fn shot_floor_dominates_when_no_copy() {
        // 10 scene headings, no dialogue, no captions. Floor = 10s.
        // Declared 5s → over by 100%.
        let src = (1..=10)
            .map(|i| format!("INT. SHOT {i} - DAY\n\n"))
            .collect::<String>();
        let sp = parse(&src).unwrap();
        let report = evaluate(&sp, 5.0);
        assert_eq!(report.verdict, Verdict::OverBudget);
        assert_eq!(report.shot_count, 10);
        assert!(report.shot_floor_secs >= 10.0);
    }

    #[test]
    fn parentheticals_dont_count_as_words() {
        let src = r#"
INT. KITCHEN - DAY

NARRATOR (V.O.)
(softly)
One bowl.
"#;
        let sp = parse(src).unwrap();
        let report = evaluate(&sp, 5.0);
        // "One bowl." = 2 words. Parenthetical excluded.
        let vo: Vec<&Contribution> = report
            .contributions
            .iter()
            .filter(|c| c.kind == "voiceover")
            .collect();
        assert_eq!(vo.len(), 1);
        assert_eq!(vo[0].words, 2);
    }

    #[test]
    fn long_action_prose_skipped_as_description() {
        // 20-word action paragraph with no quotes — scene description,
        // not overlay copy. Should not contribute to caption budget.
        let src = "\
INT. KITCHEN - DAY

The morning sun slants across a marble counter cluttered with eggs flour butter \
and a single brass thimble standing watch beside an open recipe book.

NARRATOR (V.O.)
Make something.
";
        let sp = parse(src).unwrap();
        let report = evaluate(&sp, 8.0);
        assert!(
            report.captions_secs <= 0.001,
            "long descriptive action shouldn't count as caption: {report:?}"
        );
    }
}
