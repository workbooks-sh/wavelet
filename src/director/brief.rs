//! 9-line ad creative brief parser. Structure: PRODUCT / AUDIENCE / INSIGHT /
//! PROMISE / PROOF / TONE / MUSIC / CALL / RUNTIME. Slot names are
//! case-insensitive and may appear in any order; leading Markdown decoration
//! (`#`, `-`, `*`, `1.`) and surrounding whitespace are stripped; blank lines
//! are ignored; empty content fails. See wavelet-director SKILL.md for the
//! authoring guide and a worked example.

use serde::{Deserialize, Serialize};
use std::fmt;

/// The nine required slot names, in canonical order.
pub const SLOTS: [&str; 9] = [
    "PRODUCT", "AUDIENCE", "INSIGHT", "PROMISE", "PROOF", "TONE", "MUSIC",
    "CALL", "RUNTIME",
];

/// Parsed 9-line ad creative brief. The shape downstream stages (LLM
/// director, L-Storyboard slot generator) consume.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdBrief {
    /// What we're selling — one noun phrase.
    pub product: String,
    /// Who the spot is for — specific demographic/psychographic, not "everyone".
    pub audience: String,
    /// What the audience currently believes/feels that the brand wants to shift.
    pub insight: String,
    /// What the brand says it will deliver.
    pub promise: String,
    /// One concrete reason to believe the promise.
    pub proof: String,
    /// Single-word aesthetic register (e.g. "cinematic", "irreverent").
    pub tone: String,
    /// Genre + energy curve (e.g. "ambient build → driving electronic peak").
    pub music: String,
    /// CTA in 1-5 words.
    pub call: String,
    /// Target duration in seconds.
    pub runtime_seconds: u32,
}

/// Non-fatal advisories from [`AdBrief::warnings`]. These don't fail
/// validation — they just hint that a slot looks suspicious.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BriefWarning {
    /// Canonical slot name (e.g. `"TONE"`).
    pub slot: &'static str,
    /// Human-readable description.
    pub message: String,
}

/// Errors from [`AdBrief::from_markdown`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParseError {
    /// Required slot was absent.
    MissingSlot(&'static str),
    /// Slot's content was empty after trimming.
    EmptySlot(&'static str),
    /// `RUNTIME:` content didn't parse as a positive integer.
    InvalidRuntime(String),
    /// Slot name appeared more than once.
    DuplicateSlot(&'static str),
    /// Unknown slot name in input.
    UnknownSlot(String),
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ParseError::MissingSlot(s) => write!(f, "missing required slot: {s}"),
            ParseError::EmptySlot(s) => write!(f, "slot {s} is empty"),
            ParseError::InvalidRuntime(v) => {
                write!(f, "RUNTIME must be a positive integer (seconds), got: {v:?}")
            }
            ParseError::DuplicateSlot(s) => write!(f, "slot {s} appears more than once"),
            ParseError::UnknownSlot(s) => write!(f, "unknown slot: {s:?}"),
        }
    }
}

impl std::error::Error for ParseError {}

impl AdBrief {
    /// Parse a markdown-formatted 9-line brief. See module docs for the
    /// accepted shape.
    pub fn from_markdown(s: &str) -> Result<Self, ParseError> {
        let mut slots: [Option<String>; 9] = Default::default();

        for raw in s.lines() {
            let line = strip_decoration(raw);
            if line.is_empty() {
                continue;
            }
            let Some((name, content)) = line.split_once(':') else {
                continue;
            };
            let name = name.trim();
            let content = content.trim().to_string();
            let idx = match canonical_slot_index(name) {
                Some(i) => i,
                None => return Err(ParseError::UnknownSlot(name.to_string())),
            };
            if slots[idx].is_some() {
                return Err(ParseError::DuplicateSlot(SLOTS[idx]));
            }
            if content.is_empty() {
                return Err(ParseError::EmptySlot(SLOTS[idx]));
            }
            slots[idx] = Some(content);
        }

        for (i, slot) in slots.iter().enumerate() {
            if slot.is_none() {
                return Err(ParseError::MissingSlot(SLOTS[i]));
            }
        }
        let [product, audience, insight, promise, proof, tone, music, call, runtime]: [String; 9] =
            slots.map(|v| v.unwrap());

        let runtime_seconds = runtime
            .parse::<u32>()
            .map_err(|_| ParseError::InvalidRuntime(runtime.clone()))?;
        if runtime_seconds == 0 {
            return Err(ParseError::InvalidRuntime(runtime));
        }

        Ok(AdBrief {
            product,
            audience,
            insight,
            promise,
            proof,
            tone,
            music,
            call,
            runtime_seconds,
        })
    }

    /// JSON shape suitable for piping into the LLM director.
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::to_value(self).expect("AdBrief always serializes")
    }

    /// Heuristic post-parse advisories. None of these are fatal — they hint
    /// that a slot was probably misused (prose dumped where a keyword was
    /// expected, an excessively wordy CTA, etc.).
    pub fn warnings(&self) -> Vec<BriefWarning> {
        let mut out = Vec::new();
        let short = |s: &str| s.split_whitespace().count() < 3;
        let very_long = |s: &str, max: usize| s.split_whitespace().count() > max;

        if short(&self.audience) {
            out.push(warn("AUDIENCE", "fewer than 3 words — likely too vague"));
        }
        if short(&self.insight) {
            out.push(warn("INSIGHT", "fewer than 3 words — likely too vague"));
        }
        if short(&self.promise) {
            out.push(warn("PROMISE", "fewer than 3 words — likely too vague"));
        }
        if short(&self.proof) {
            out.push(warn("PROOF", "fewer than 3 words — likely too vague"));
        }
        if very_long(&self.tone, 20) {
            out.push(warn("TONE", "looks like prose — expected a tone keyword"));
        }
        if very_long(&self.call, 20) {
            out.push(warn("CALL", "looks like prose — CTAs are 1-5 words"));
        } else if very_long(&self.call, 5) {
            out.push(warn("CALL", "CTAs read best at 1-5 words"));
        }
        if self.runtime_seconds > 120 {
            out.push(warn("RUNTIME", "longer than 2 minutes — verify"));
        }
        out
    }
}

fn warn(slot: &'static str, msg: &str) -> BriefWarning {
    BriefWarning { slot, message: msg.to_string() }
}

fn strip_decoration(raw: &str) -> &str {
    let t = raw.trim()
        .trim_start_matches(|c: char| matches!(c, '#' | '-' | '*' | '>'))
        .trim_start();
    t.trim_start_matches(|c: char| c.is_ascii_digit() || c == '.')
        .trim_start()
}

fn canonical_slot_index(name: &str) -> Option<usize> {
    let upper = name.to_ascii_uppercase();
    SLOTS.iter().position(|s| *s == upper.as_str())
}

#[cfg(test)]
mod tests {
    use super::*;

    const GOOD: &str = "\
PRODUCT: Allbirds Tree Runner sneakers
AUDIENCE: 28-40 urban professionals who walk more than they run
INSIGHT: Sustainable usually means uncomfortable or ugly
PROMISE: All-day comfort that happens to be made from trees
PROOF: Eucalyptus-fiber upper plus sugarcane sole, machine washable
TONE: understated
MUSIC: acoustic minimal -> warm indie-folk swell
CALL: Try them barefoot
RUNTIME: 15
";

    #[test]
    fn parses_canonical_form() {
        let b = AdBrief::from_markdown(GOOD).unwrap();
        assert_eq!(b.product, "Allbirds Tree Runner sneakers");
        assert_eq!(b.runtime_seconds, 15);
        assert_eq!(b.tone, "understated");
        assert_eq!(b.call, "Try them barefoot");
    }

    #[test]
    fn round_trip_through_json() {
        let b = AdBrief::from_markdown(GOOD).unwrap();
        let v = b.to_json();
        let s = serde_json::to_string(&v).unwrap();
        let back: AdBrief = serde_json::from_str(&s).unwrap();
        assert_eq!(b, back);
    }

    #[test]
    fn slot_order_does_not_matter() {
        let scrambled = "\
RUNTIME: 12
MUSIC: lo-fi minimal -> tropical house drop
PRODUCT: Liquid Death Mountain Water
AUDIENCE: 21-30 craft-beer-curious skaters and punks
INSIGHT: Hydration brands talk down to you
PROMISE: Death to plastic
PROOF: Tall aluminum can, infinitely recyclable
TONE: irreverent
CALL: Murder your thirst
";
        let b = AdBrief::from_markdown(scrambled).unwrap();
        assert_eq!(b.product, "Liquid Death Mountain Water");
        assert_eq!(b.runtime_seconds, 12);
    }

    #[test]
    fn tolerates_markdown_decoration_and_blank_lines() {
        let decorated = "\
# Brief

- PRODUCT: Allbirds Tree Runner sneakers
- AUDIENCE: 28-40 urban professionals who walk more than they run
- INSIGHT: Sustainable usually means uncomfortable or ugly

- PROMISE: All-day comfort that happens to be made from trees
- PROOF: Eucalyptus-fiber upper plus sugarcane sole, machine washable
- TONE: understated
- MUSIC: acoustic minimal swell
- CALL: Try them barefoot
- RUNTIME: 15
";
        let b = AdBrief::from_markdown(decorated).unwrap();
        assert_eq!(b.tone, "understated");
    }

    #[test]
    fn tolerates_numbered_lines() {
        let numbered = "\
1. PRODUCT: Allbirds Tree Runner sneakers
2. AUDIENCE: 28-40 urban professionals who walk more than they run
3. INSIGHT: Sustainable usually means uncomfortable or ugly
4. PROMISE: All-day comfort that happens to be made from trees
5. PROOF: Eucalyptus-fiber upper plus sugarcane sole, machine washable
6. TONE: understated
7. MUSIC: acoustic minimal swell
8. CALL: Try them barefoot
9. RUNTIME: 15
";
        let b = AdBrief::from_markdown(numbered).unwrap();
        assert_eq!(b.product, "Allbirds Tree Runner sneakers");
    }

    #[test]
    fn missing_required_slot_errors() {
        let missing_proof = "\
PRODUCT: x
AUDIENCE: x y z
INSIGHT: x y z
PROMISE: x y z
TONE: understated
MUSIC: x y z
CALL: x
RUNTIME: 15
";
        let err = AdBrief::from_markdown(missing_proof).unwrap_err();
        assert_eq!(err, ParseError::MissingSlot("PROOF"));
    }

    #[test]
    fn non_numeric_runtime_errors() {
        let bad = GOOD.replace("RUNTIME: 15", "RUNTIME: fifteen");
        let err = AdBrief::from_markdown(&bad).unwrap_err();
        assert!(matches!(err, ParseError::InvalidRuntime(_)));
    }

    #[test]
    fn zero_runtime_errors() {
        let bad = GOOD.replace("RUNTIME: 15", "RUNTIME: 0");
        let err = AdBrief::from_markdown(&bad).unwrap_err();
        assert!(matches!(err, ParseError::InvalidRuntime(_)));
    }

    #[test]
    fn empty_content_errors() {
        let bad = GOOD.replace("TONE: understated", "TONE:");
        let err = AdBrief::from_markdown(&bad).unwrap_err();
        assert_eq!(err, ParseError::EmptySlot("TONE"));
    }

    #[test]
    fn duplicate_slot_errors() {
        let dup = format!("{GOOD}TONE: cinematic\n");
        let err = AdBrief::from_markdown(&dup).unwrap_err();
        assert_eq!(err, ParseError::DuplicateSlot("TONE"));
    }

    #[test]
    fn unknown_slot_errors() {
        let bad = format!("{GOOD}MOOD: chill\n");
        let err = AdBrief::from_markdown(&bad).unwrap_err();
        assert_eq!(err, ParseError::UnknownSlot("MOOD".into()));
    }

    #[test]
    fn warnings_flag_prose_in_tone_and_call() {
        let prose = "\
PRODUCT: Allbirds Tree Runner sneakers
AUDIENCE: 28-40 urban professionals who walk more than they run
INSIGHT: Sustainable usually means uncomfortable or ugly
PROMISE: All-day comfort that happens to be made from trees
PROOF: Eucalyptus-fiber upper plus sugarcane sole, machine washable
TONE: a kind of quiet restrained californian morning light feeling that spreads across everything you see when you wake up alone in the apartment
MUSIC: acoustic minimal swell
CALL: go visit the store and pick up a pair after work today if you can spare a moment to walk around and try them on
RUNTIME: 15
";
        let b = AdBrief::from_markdown(prose).unwrap();
        let ws = b.warnings();
        assert!(ws.iter().any(|w| w.slot == "TONE"));
        assert!(ws.iter().any(|w| w.slot == "CALL"));
    }

    #[test]
    fn well_formed_brief_has_no_warnings() {
        let b = AdBrief::from_markdown(GOOD).unwrap();
        assert!(b.warnings().is_empty(), "got warnings: {:?}", b.warnings());
    }
}
