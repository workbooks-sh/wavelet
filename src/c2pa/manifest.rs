//! Build a C2PA manifest definition (JSON) from a wavelet [`Composition`] plus
//! the backend cache state.
//!
//! The manifest carries four assertions:
//!
//! - `c2pa.actions` — one entry per scene: `c2pa.created` for the generated
//!   visual asset, plus a final `c2pa.placed` for the compositor's mux.
//! - `stds.schema-org.CreativeWork` — title, author, description.
//! - `c2pa.training-mining` — declares `notAllowed` (no AI training reuse).
//! - One ingredient per cached backend call — Seedream / Kling / EL Music /
//!   etc., each with the provider, cluster, request hash, and (when available)
//!   the cost estimate carried by the cache manifest.
//!
//! Sourcing ingredients from the cache (instead of from the composition
//! directly) means we don't have to thread provider identifiers through the
//! Composition schema — the cache already records every backend touch.

use crate::backends::cache::Manifest as CacheManifest;
use crate::render_offline::Composition;
use serde::Serialize;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

/// Inputs to the manifest builder. Constructed via
/// [`ManifestInputs::from_composition`].
#[derive(Debug, Clone)]
pub struct ManifestInputs {
    /// Composition title (from `--title`, the brief, or a fallback).
    pub title: String,
    /// Optional author name (`--author` or brief field). Carried in the
    /// CreativeWork assertion.
    pub author: Option<String>,
    /// One action entry per scene — `(scene_index, html_path)`.
    pub scene_actions: Vec<(usize, PathBuf)>,
    /// One ingredient per cached backend manifest discovered under
    /// `cache_root`. Empty when no cache root is supplied.
    pub ingredients: Vec<IngredientEntry>,
}

/// One C2PA ingredient — a generated input the final MP4 depends on.
#[derive(Debug, Clone)]
pub struct IngredientEntry {
    /// Human-readable name (`"Seedream 4.5 / image_generate / abc12345"`).
    pub title: String,
    /// Backend provider (`seedream`, `kling`, `elevenlabs`, `fal`, …).
    pub provider: String,
    /// Capability cluster (`image_generate`, `img2vid`, `tts`, `music`).
    pub cluster: String,
    /// Stable request hash, lifted from the cache manifest.
    pub request_hash: String,
    /// Cost estimate at fetch time in USD, if recorded by the cache.
    pub cost_estimate_usd: f32,
}

impl ManifestInputs {
    /// Assemble inputs from a composition + optional cache root. The cache is
    /// walked one level deep — provider subdirs each containing
    /// `*.manifest.json` entries. Missing / unreadable manifests are skipped
    /// silently (no provenance for unknown assets is honest; the alternative
    /// is to fabricate ingredients).
    pub fn from_composition(
        comp: &Composition,
        cache_root: Option<&Path>,
        title: Option<&str>,
        author: Option<&str>,
    ) -> Self {
        let scene_actions = comp
            .scenes
            .iter()
            .enumerate()
            .map(|(i, s)| (i, s.html_path.clone()))
            .collect();

        let ingredients = match cache_root {
            Some(root) => collect_ingredients(root),
            None => Vec::new(),
        };

        Self {
            title: title.map(str::to_string).unwrap_or_else(|| "wavelet export".to_string()),
            author: author.map(str::to_string),
            scene_actions,
            ingredients,
        }
    }
}

#[derive(Serialize)]
struct Action {
    action: &'static str,
    #[serde(rename = "softwareAgent", skip_serializing_if = "Option::is_none")]
    software_agent: Option<Value>,
    #[serde(rename = "digitalSourceType", skip_serializing_if = "Option::is_none")]
    digital_source_type: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    parameters: Option<Value>,
}

#[derive(Serialize)]
struct CreativeWork {
    #[serde(rename = "@context")]
    context: &'static str,
    #[serde(rename = "@type")]
    typ: &'static str,
    name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    author: Option<Value>,
    description: String,
}

#[derive(Serialize)]
struct TrainingMining {
    entries: serde_json::Map<String, Value>,
}

/// Build the JSON manifest definition string that `c2pa::Builder::with_definition`
/// expects. The result is a top-level object with `title`, `format`,
/// `claim_generator_info`, `ingredients`, and `assertions`.
pub fn build_manifest(inputs: &ManifestInputs) -> Result<String, super::C2paError> {
    // One synthesizing action declaring the whole composition as AI-composited.
    // Per-scene context lives in the `parameters.scenes` array. Keeping the
    // action list short avoids the v2 cross-validator's ingredientMismatch
    // check, which is strict about per-action `ingredients` refs we don't yet
    // emit. Adding granular per-shot actions with cross-refs is a follow-on
    // once the c2pa-rs API for that lands more polish.
    let scenes_param: Vec<Value> = inputs
        .scene_actions
        .iter()
        .map(|(idx, html_path)| {
            json!({ "index": idx, "html": html_path.display().to_string() })
        })
        .collect();
    let actions = vec![Action {
        action: "c2pa.created",
        software_agent: Some(json!({
            "name": "wavelet",
            "version": env!("CARGO_PKG_VERSION"),
        })),
        digital_source_type: Some(
            "http://cv.iptc.org/newscodes/digitalsourcetype/compositeWithTrainedAlgorithmicMedia",
        ),
        parameters: Some(json!({
            "scenes": scenes_param,
            "ingredient_count": inputs.ingredients.len(),
        })),
    }];

    let mut training_entries = serde_json::Map::new();
    training_entries.insert(
        "c2pa.ai_inference".to_string(),
        json!({ "use": "notAllowed" }),
    );
    training_entries.insert(
        "c2pa.ai_generative_training".to_string(),
        json!({ "use": "notAllowed" }),
    );
    training_entries.insert("c2pa.data_mining".to_string(), json!({ "use": "notAllowed" }));

    let creative_work = CreativeWork {
        context: "https://schema.org",
        typ: "CreativeWork",
        name: inputs.title.clone(),
        author: inputs
            .author
            .as_ref()
            .map(|a| json!([{ "@type": "Person", "name": a }])),
        description: format!(
            "Generated by wavelet. {} scenes; {} AI-generated ingredients.",
            inputs.scene_actions.len(),
            inputs.ingredients.len()
        ),
    };

    let ingredients_json: Vec<Value> = inputs
        .ingredients
        .iter()
        .map(|i| {
            json!({
                "title": i.title,
                "relationship": "componentOf",
                "metadata": {
                    "provider": i.provider,
                    "cluster": i.cluster,
                    "request_hash": i.request_hash,
                    "cost_estimate_usd": i.cost_estimate_usd,
                }
            })
        })
        .collect();

    let definition = json!({
        "title": inputs.title,
        "format": "video/mp4",
        "claim_generator": super::WAVELET_CLAIM_GENERATOR,
        "claim_generator_info": [{
            "name": "wavelet",
            "version": env!("CARGO_PKG_VERSION"),
        }],
        "ingredients": ingredients_json,
        "assertions": [
            { "label": "c2pa.actions", "data": { "actions": actions } },
            { "label": "stds.schema-org.CreativeWork", "data": creative_work },
            { "label": "c2pa.training-mining", "data": TrainingMining { entries: training_entries } },
        ],
    });

    Ok(serde_json::to_string(&definition)?)
}

fn collect_ingredients(root: &Path) -> Vec<IngredientEntry> {
    let Ok(providers) = std::fs::read_dir(root) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for prov in providers.flatten() {
        let path = prov.path();
        if !path.is_dir() {
            continue;
        }
        let Ok(entries) = std::fs::read_dir(&path) else {
            continue;
        };
        for entry in entries.flatten() {
            let p = entry.path();
            let Some(name) = p.file_name().and_then(|s| s.to_str()) else {
                continue;
            };
            if !name.ends_with(".manifest.json") {
                continue;
            }
            let Ok(src) = std::fs::read_to_string(&p) else {
                continue;
            };
            let Ok(m): Result<CacheManifest, _> = serde_json::from_str(&src) else {
                continue;
            };
            let short = &m.request_hash[..m.request_hash.len().min(8)];
            out.push(IngredientEntry {
                title: format!("{} / {} / {short}", m.provider, m.cluster),
                provider: m.provider,
                cluster: m.cluster,
                request_hash: m.request_hash,
                cost_estimate_usd: m.cost_estimate_usd,
            });
        }
    }
    out.sort_by(|a, b| a.title.cmp(&b.title));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backends::cache::AssetCache;
    use crate::render_offline::SceneSpec;
    use tempfile::tempdir;

    fn fake_comp(n_scenes: usize) -> Composition {
        Composition {
            width: 1280,
            height: 720,
            fps: 30,
            duration_frames: (n_scenes as u32) * 30,
            aspect: None,
            scenes: (0..n_scenes)
                .map(|i| SceneSpec {
                    html_path: PathBuf::from(format!("scenes/s{i}.html")),
                    start_frame: (i as u32) * 30,
                    duration_frames: 30,
                    transition_in: None,
                    video_bg: None,
                })
                .collect(),
            audio_cues: vec![],
        }
    }

    #[test]
    fn manifest_action_lists_each_scene_under_a_single_created() {
        let comp = fake_comp(4);
        let inputs = ManifestInputs::from_composition(&comp, None, Some("My Ad"), Some("Ad Agency"));
        let json = build_manifest(&inputs).unwrap();
        let v: Value = serde_json::from_str(&json).unwrap();
        let actions = v["assertions"]
            .as_array()
            .unwrap()
            .iter()
            .find(|a| a["label"] == "c2pa.actions")
            .unwrap()["data"]["actions"]
            .as_array()
            .unwrap();
        assert_eq!(actions.len(), 1);
        assert_eq!(actions[0]["action"], "c2pa.created");
        let scenes = actions[0]["parameters"]["scenes"].as_array().unwrap();
        assert_eq!(scenes.len(), 4);
        assert_eq!(scenes[0]["index"], 0);
    }

    #[test]
    fn manifest_includes_creative_work_with_author() {
        let comp = fake_comp(1);
        let inputs = ManifestInputs::from_composition(&comp, None, Some("Ad"), Some("Shane"));
        let json = build_manifest(&inputs).unwrap();
        assert!(json.contains("stds.schema-org.CreativeWork"));
        assert!(json.contains("Shane"));
    }

    #[test]
    fn manifest_includes_training_mining_not_allowed() {
        let comp = fake_comp(1);
        let inputs = ManifestInputs::from_composition(&comp, None, None, None);
        let json = build_manifest(&inputs).unwrap();
        assert!(json.contains("c2pa.training-mining"));
        assert!(json.contains("notAllowed"));
    }

    #[test]
    fn ingredients_sourced_from_cache_manifests() {
        let dir = tempdir().unwrap();
        let cache = AssetCache::new(dir.path());
        let manifests = [
            CacheManifest {
                version: 1,
                provider: "seedream".into(),
                cluster: "image_generate".into(),
                request_hash: "aaaaaaaa11111111".into(),
                request: json!({"prompt":"x"}),
                response: json!({}),
                cost_estimate_usd: 0.04,
                asset_path: None,
                created_at: "2026-05-04T00:00:00Z".into(),
            },
            CacheManifest {
                version: 1,
                provider: "kling".into(),
                cluster: "img2vid".into(),
                request_hash: "bbbbbbbb22222222".into(),
                request: json!({}),
                response: json!({}),
                cost_estimate_usd: 0.25,
                asset_path: None,
                created_at: "2026-05-04T00:00:00Z".into(),
            },
            CacheManifest {
                version: 1,
                provider: "elevenlabs".into(),
                cluster: "music".into(),
                request_hash: "cccccccc33333333".into(),
                request: json!({}),
                response: json!({}),
                cost_estimate_usd: 0.10,
                asset_path: None,
                created_at: "2026-05-04T00:00:00Z".into(),
            },
        ];
        for m in &manifests {
            cache.store(m).unwrap();
        }

        let comp = fake_comp(2);
        let inputs =
            ManifestInputs::from_composition(&comp, Some(dir.path()), Some("T"), None);
        assert_eq!(inputs.ingredients.len(), 3);
        assert!(inputs.ingredients.iter().any(|i| i.provider == "seedream"));
        assert!(inputs.ingredients.iter().any(|i| i.provider == "kling"));
        assert!(inputs.ingredients.iter().any(|i| i.provider == "elevenlabs"));

        let json = build_manifest(&inputs).unwrap();
        let v: Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["ingredients"].as_array().unwrap().len(), 3);
    }
}
