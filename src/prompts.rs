//! Shared prompt constants applied across every gen-call adapter.
//!
//! ## Standard negative prompt (wb-ynn0)
//!
//! The May-2026 ComfyUI workflow audit identified four
//! highest-frequency failure modes for image + video gen models:
//! hallucinated text/logos, watermark hallucination, anatomy errors
//! (extra limbs/fingers), and generic-AI blur. The Artlist 2026
//! negative-prompt guide reports a documented ~30% reduction in
//! unusable outputs from appending a canonical negative-prompt string
//! to every shot — same string across Kling/Veo/Wan/Hailuo.
//!
//! [`DEFAULT_NEGATIVE_PROMPT`] holds the canonical string. Adapters
//! whose underlying API supports a `negative_prompt` parameter
//! pre-fill it with this default when the caller didn't supply one,
//! and merge it with any caller-provided addition via
//! [`negative_prompt_with`]. The agent therefore never has to think
//! about the standard negatives; they flow automatically.

/// Canonical default negative prompt — appended to every image + video
/// generation call that supports a `negative_prompt` parameter. Steers
/// gen models away from the four highest-frequency embarrassing
/// failure modes per the May-2026 ComfyUI workflow audit
/// (hallucinated text/logos, watermark hallucination, anatomy errors,
/// generic-AI blur). Verbatim copy from the Artlist 2026 negative-
/// prompt guide.
pub const DEFAULT_NEGATIVE_PROMPT: &str = "no text overlay, no watermark, no distortion, no extra limbs, no extra fingers, low quality, blurry";

/// Merge a caller-provided negative-prompt addition with the canonical
/// default. The default appears first, followed by a comma + space,
/// followed by `addition` (with any leading commas / whitespace
/// stripped so we don't emit `"…, , foo"`).
///
/// Empty / whitespace-only `addition` returns the default unchanged.
pub fn negative_prompt_with(addition: &str) -> String {
    let trimmed = addition.trim_start_matches(|c: char| c == ',' || c.is_whitespace());
    let trimmed = trimmed.trim();
    if trimmed.is_empty() {
        return DEFAULT_NEGATIVE_PROMPT.to_string();
    }
    format!("{DEFAULT_NEGATIVE_PROMPT}, {trimmed}")
}

/// Resolve the negative prompt to send to a backend, applying the
/// default if `caller_provided` is `None`, otherwise merging the
/// default with the caller's value via [`negative_prompt_with`].
///
/// When `use_default` is `false` the caller's value is passed through
/// verbatim (or `None` propagates) — the escape hatch wired to the
/// CLI's `--no-default-negatives` flag.
pub fn resolve_negative_prompt(
    caller_provided: Option<&str>,
    use_default: bool,
) -> Option<String> {
    if !use_default {
        return caller_provided.map(|s| s.to_string());
    }
    match caller_provided {
        None => Some(DEFAULT_NEGATIVE_PROMPT.to_string()),
        Some(addition) => Some(negative_prompt_with(addition)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_addition_returns_default() {
        assert_eq!(negative_prompt_with(""), DEFAULT_NEGATIVE_PROMPT);
        assert_eq!(negative_prompt_with("   "), DEFAULT_NEGATIVE_PROMPT);
    }

    #[test]
    fn non_empty_joins_with_comma_space() {
        let out = negative_prompt_with("ugly, deformed");
        assert!(out.starts_with(DEFAULT_NEGATIVE_PROMPT));
        assert!(out.ends_with(", ugly, deformed"));
        assert_eq!(
            out,
            format!("{DEFAULT_NEGATIVE_PROMPT}, ugly, deformed")
        );
    }

    #[test]
    fn no_double_commas_when_addition_starts_with_comma() {
        let out = negative_prompt_with(", ugly, deformed");
        assert!(!out.contains(",,"));
        assert!(!out.contains(", ,"));
        assert_eq!(
            out,
            format!("{DEFAULT_NEGATIVE_PROMPT}, ugly, deformed")
        );
    }

    #[test]
    fn whitespace_in_addition_trimmed() {
        let out = negative_prompt_with("  ,   ugly  ");
        assert_eq!(out, format!("{DEFAULT_NEGATIVE_PROMPT}, ugly"));
    }

    #[test]
    fn resolve_uses_default_when_none_provided() {
        let out = resolve_negative_prompt(None, true);
        assert_eq!(out.as_deref(), Some(DEFAULT_NEGATIVE_PROMPT));
    }

    #[test]
    fn resolve_merges_when_caller_provided() {
        let out = resolve_negative_prompt(Some("ugly"), true);
        assert_eq!(
            out.as_deref(),
            Some(format!("{DEFAULT_NEGATIVE_PROMPT}, ugly").as_str())
        );
    }

    #[test]
    fn resolve_passes_through_when_default_disabled() {
        let out = resolve_negative_prompt(Some("ugly"), false);
        assert_eq!(out.as_deref(), Some("ugly"));

        let out = resolve_negative_prompt(None, false);
        assert!(out.is_none());
    }

    #[test]
    fn default_lists_canonical_four_failure_modes() {
        // Locks the constant to its documented form. Anyone changing
        // the canonical default must also update the research-doc
        // lineage in `docs/research/text-in-ai-video.md` §3.
        for needle in [
            "no text overlay",
            "no watermark",
            "no distortion",
            "no extra limbs",
            "no extra fingers",
            "low quality",
            "blurry",
        ] {
            assert!(
                DEFAULT_NEGATIVE_PROMPT.contains(needle),
                "default negative prompt missing required token '{needle}': {DEFAULT_NEGATIVE_PROMPT}"
            );
        }
    }
}
