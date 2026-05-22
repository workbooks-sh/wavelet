//! Veo prompt-construction skill.
//!
//! Emits Google's official Veo 3.1 five-part formula
//! `[Cinematography] + [Subject] + [Action] + [Context] + [Style & Ambiance]`
//! along with a stable anti-stock `negativePrompt` default that excludes
//! Veo's dominant failure modes (smooth slow-mo, perfect HDR, oversaturated,
//! glossy stock-photography look).
//!
//! See `docs/wavelet-agent-research-2026-05-20.md` §1.1 / §1.2.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ShotPrompt {
    pub subject: String,
    pub action: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub style: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cinematography: Option<CineSpec>,
    /// Optional caller-supplied negative-prompt addendum. Appended to the
    /// anti-stock default, comma-separated. To opt out of the default
    /// entirely, set `anti_stock = false`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub negative_prompt: Option<String>,
    /// Include the anti-stock negative defaults. Defaults to true via
    /// `default_anti_stock`.
    #[serde(default = "default_anti_stock")]
    pub anti_stock: bool,
}

fn default_anti_stock() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CineSpec {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shot_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lens: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub movement: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lighting: Option<String>,
}

/// Anti-stock negative-prompt default. Sourced from research doc §1.2 —
/// the failure modes Veo skews toward when given a generic prompt.
pub const ANTI_STOCK_NEGATIVE: &str = "smooth slow-motion, perfect HDR, oversaturated colors, glossy, stock photography look, generic b-roll, cliché lighting, lens flare overuse, overly clean, plastic skin, AI-perfect symmetry";

/// Construct a Veo-shaped prompt + negativePrompt pair from a structured spec.
///
/// Emit order is `[Cinematography] + [Subject] + [Action] + [Context] + [Style & Ambiance]`.
/// The cinematography part assembles `shot_type, lens, movement, lighting`
/// in that order, comma-separated.
///
/// The negative prompt defaults to `ANTI_STOCK_NEGATIVE`. A caller-supplied
/// `negative_prompt` is **appended** to the default (comma-separated). To
/// fully replace the default, set `anti_stock = false`.
pub fn build_veo_prompt(spec: &ShotPrompt) -> (String, String) {
    let mut parts: Vec<String> = Vec::with_capacity(5);

    if let Some(cine) = spec.cinematography.as_ref() {
        let cine_part = assemble_cinematography(cine);
        if !cine_part.is_empty() {
            parts.push(cine_part);
        }
    }

    let subject = spec.subject.trim();
    if !subject.is_empty() {
        parts.push(subject.to_string());
    }

    let action = spec.action.trim();
    if !action.is_empty() {
        parts.push(action.to_string());
    }

    if let Some(ctx) = spec.context.as_ref().map(|s| s.trim()).filter(|s| !s.is_empty()) {
        parts.push(ctx.to_string());
    }

    if let Some(style) = spec.style.as_ref().map(|s| s.trim()).filter(|s| !s.is_empty()) {
        parts.push(style.to_string());
    }

    let prompt = parts.join(". ");

    let negative = match (spec.anti_stock, spec.negative_prompt.as_deref()) {
        (true, Some(extra)) if !extra.trim().is_empty() => {
            format!("{}, {}", ANTI_STOCK_NEGATIVE, extra.trim())
        }
        (true, _) => ANTI_STOCK_NEGATIVE.to_string(),
        (false, Some(extra)) => extra.trim().to_string(),
        (false, None) => String::new(),
    };

    (prompt, negative)
}

fn assemble_cinematography(cine: &CineSpec) -> String {
    let mut tags: Vec<&str> = Vec::new();
    if let Some(v) = cine.shot_type.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        tags.push(v);
    }
    if let Some(v) = cine.lens.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        tags.push(v);
    }
    if let Some(v) = cine.movement.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        tags.push(v);
    }
    if let Some(v) = cine.lighting.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        tags.push(v);
    }
    tags.join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fully_specified() -> ShotPrompt {
        ShotPrompt {
            subject: "A barista in a small espresso bar".into(),
            action: "pulls a single shot, steam rising from the cup".into(),
            context: Some("morning light through a window, wood counter".into()),
            style: Some("documentary-grade photoreal, 35mm film texture, muted warm palette".into()),
            cinematography: Some(CineSpec {
                shot_type: Some("close-up".into()),
                lens: Some("85mm portrait, shallow depth of field f/1.8".into()),
                movement: Some("slow dolly in".into()),
                lighting: Some("golden hour, hard side light from screen-left".into()),
            }),
            negative_prompt: None,
            anti_stock: true,
        }
    }

    #[test]
    fn full_spec_emits_all_five_parts_in_order() {
        let (prompt, _neg) = build_veo_prompt(&fully_specified());
        let cine_idx = prompt.find("close-up").expect("cine");
        let subj_idx = prompt.find("barista").expect("subj");
        let act_idx = prompt.find("pulls a single shot").expect("act");
        let ctx_idx = prompt.find("morning light").expect("ctx");
        let style_idx = prompt.find("documentary-grade").expect("style");
        assert!(cine_idx < subj_idx);
        assert!(subj_idx < act_idx);
        assert!(act_idx < ctx_idx);
        assert!(ctx_idx < style_idx);
        assert!(prompt.contains("85mm portrait"));
        assert!(prompt.contains("slow dolly in"));
        assert!(prompt.contains("golden hour"));
    }

    #[test]
    fn cinematography_assembles_in_canonical_order() {
        let spec = ShotPrompt {
            subject: "x".into(),
            action: "y".into(),
            cinematography: Some(CineSpec {
                shot_type: Some("wide shot".into()),
                lens: Some("35mm".into()),
                movement: Some("static".into()),
                lighting: Some("overcast".into()),
            }),
            ..Default::default()
        };
        let (prompt, _) = build_veo_prompt(&spec);
        let head = prompt.split(". ").next().unwrap();
        assert_eq!(head, "wide shot, 35mm, static, overcast");
    }

    #[test]
    fn all_defaults_subject_action_only_still_emits_prompt() {
        let spec = ShotPrompt {
            subject: "A coffee cup".into(),
            action: "sits on a table".into(),
            anti_stock: true,
            ..Default::default()
        };
        let (prompt, neg) = build_veo_prompt(&spec);
        assert!(!prompt.is_empty());
        assert_eq!(prompt, "A coffee cup. sits on a table");
        assert_eq!(neg, ANTI_STOCK_NEGATIVE);
    }

    #[test]
    fn anti_stock_default_present_when_no_override() {
        let (_p, neg) = build_veo_prompt(&fully_specified());
        assert_eq!(neg, ANTI_STOCK_NEGATIVE);
        assert!(neg.contains("stock photography look"));
        assert!(neg.contains("smooth slow-motion"));
    }

    #[test]
    fn custom_negative_appends_to_anti_stock() {
        let mut spec = fully_specified();
        spec.negative_prompt = Some("watermarks, text overlays".into());
        let (_p, neg) = build_veo_prompt(&spec);
        assert!(neg.starts_with(ANTI_STOCK_NEGATIVE));
        assert!(neg.ends_with("watermarks, text overlays"));
    }

    #[test]
    fn anti_stock_off_replaces_default_with_custom() {
        let mut spec = fully_specified();
        spec.anti_stock = false;
        spec.negative_prompt = Some("only this".into());
        let (_p, neg) = build_veo_prompt(&spec);
        assert_eq!(neg, "only this");
    }

    #[test]
    fn anti_stock_off_and_no_override_yields_empty_negative() {
        let mut spec = fully_specified();
        spec.anti_stock = false;
        spec.negative_prompt = None;
        let (_p, neg) = build_veo_prompt(&spec);
        assert!(neg.is_empty());
    }

    #[test]
    fn cinematography_none_skips_prefix() {
        let spec = ShotPrompt {
            subject: "a dog".into(),
            action: "runs".into(),
            cinematography: None,
            anti_stock: true,
            ..Default::default()
        };
        let (prompt, _) = build_veo_prompt(&spec);
        assert!(prompt.starts_with("a dog"));
    }

    #[test]
    fn empty_cinespec_no_prefix() {
        let spec = ShotPrompt {
            subject: "a dog".into(),
            action: "runs".into(),
            cinematography: Some(CineSpec::default()),
            anti_stock: true,
            ..Default::default()
        };
        let (prompt, _) = build_veo_prompt(&spec);
        assert!(prompt.starts_with("a dog"));
    }

    #[test]
    fn partial_cinespec_includes_only_set_fields() {
        let spec = ShotPrompt {
            subject: "x".into(),
            action: "y".into(),
            cinematography: Some(CineSpec {
                shot_type: Some("close-up".into()),
                lighting: Some("golden hour".into()),
                ..Default::default()
            }),
            ..Default::default()
        };
        let (prompt, _) = build_veo_prompt(&spec);
        let head = prompt.split(". ").next().unwrap();
        assert_eq!(head, "close-up, golden hour");
    }

    #[test]
    fn deserialize_from_json_with_defaults() {
        let j = serde_json::json!({
            "subject": "a barista",
            "action": "pulls a shot"
        });
        let spec: ShotPrompt = serde_json::from_value(j).unwrap();
        assert!(spec.anti_stock);
        let (prompt, neg) = build_veo_prompt(&spec);
        assert_eq!(prompt, "a barista. pulls a shot");
        assert_eq!(neg, ANTI_STOCK_NEGATIVE);
    }
}
