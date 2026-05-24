//! Character clip-ref — keyed reference-image bundle (wb-cx08 + wb-jwnk).
//!
//! A `character-ref` clip-HTML file holds the canonical CHARACTER cue
//! name (matching `fountain::canonicalize_name`) plus 1..N reference
//! images. It's the load-bearing wiring that lets the storyboard
//! planner route a Dialogue scene through `fal-veo3-ref` instead of
//! stock-search / plain txt2vid, so the generated commercial stays
//! character-consistent across cuts.
//!
//! Files land at `<workdir>/refs/character/<slug>.clip.html`. The slug
//! is lowercased canonical-name plus a `-hands` / `-product-hands`
//! suffix for the non-default types — so the same CHARACTER cue can
//! have multiple parallel refs (one face/body bundle + one hands
//! bundle). The planner reads `character-type` from the clip-HTML
//! front-matter, not the filename; the suffix is just author
//! ergonomics.
//!
//! Files are auto-discovered by `wavelet storyboard plan`.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use chrono::Utc;
use serde::{Deserialize, Serialize};
use ulid::Ulid;

use super::{ClipKind, ClipRef, ClipRefError};

/// Character framing focus — drives which Fal Veo 3.1 reference cluster
/// the planner targets. The storyboard planner uses these to route a
/// face/dialogue shot through `FullBody` refs while routing an
/// ECU-hands cutaway through a `Hands` (or `ProductHands`) ref for the
/// same character — so the hands shot doesn't leak face features into
/// the conditioning signal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CharacterType {
    /// Full-body / face references. Default.
    FullBody,
    /// Hands-only references (ECU / closeup of hand action).
    Hands,
    /// Hands holding a product (ECU + product-hands shot vocab).
    ProductHands,
}

impl Default for CharacterType {
    fn default() -> Self {
        CharacterType::FullBody
    }
}

impl CharacterType {
    /// Wire-format string for the `character-type` YAML field.
    pub fn as_kebab(self) -> &'static str {
        match self {
            CharacterType::FullBody => "full-body",
            CharacterType::Hands => "hands",
            CharacterType::ProductHands => "product-hands",
        }
    }
}

/// One loaded character reference. Returned by [`load_characters`] and
/// passed into the storyboard planner.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CharacterRef {
    /// Canonical name (uppercase, extension-stripped — matches
    /// `fountain::CharacterEntry::canonical`).
    pub name: String,
    /// Reference image paths or HTTPS URLs, in the order they were
    /// declared. Same shape the Fal Veo 3.1 reference adapter's
    /// `--reference` flag takes.
    pub reference_images: Vec<String>,
    /// Character framing focus.
    pub character_type: CharacterType,
}

/// Options for `wavelet character define`.
pub struct EmitOptions<'a> {
    /// Project workdir. Character refs land at
    /// `<workdir>/refs/character/`.
    pub workdir: &'a Path,
}

/// Result of emitting one character ref.
pub struct Emission {
    /// Path the clip-ref was written to.
    pub path: PathBuf,
    /// ULID assigned to the clip-ref.
    pub clip: Ulid,
    /// Canonical name keyed into the clip-ref.
    pub name: String,
}

/// Emit a character-ref clip-HTML at
/// `<workdir>/refs/character/<slug>.clip.html`.
///
/// `name` is canonicalized via `fountain::canonicalize_name` so the
/// emitted file's `name` field always matches the keying the screenplay
/// character extractor uses. Reference images may be local paths or
/// HTTPS URLs — both shapes are passed through verbatim to the Fal
/// adapter downstream.
///
/// The on-disk slug picks up a `-hands` / `-product-hands` suffix for
/// the non-default types (wb-jwnk) so the same canonical character can
/// own multiple parallel ref bundles without collisions. The planner
/// keys off the `character-type` front-matter field, not the filename
/// — the suffix is for author ergonomics only.
pub fn emit_character(
    raw_name: &str,
    reference_images: &[String],
    character_type: CharacterType,
    opts: &EmitOptions<'_>,
) -> Result<Emission, ClipRefError> {
    let name = fountain::canonicalize_name(raw_name)
        .unwrap_or_else(|| raw_name.trim().to_ascii_uppercase());

    let slug = slug_with_type(&name, character_type);
    let path = opts
        .workdir
        .join("refs")
        .join(ClipKind::CharacterRef.as_kebab())
        .join(format!("{slug}.clip.html"));

    let mut extra: BTreeMap<String, serde_yaml::Value> = BTreeMap::new();
    extra.insert(
        "name".to_string(),
        serde_yaml::Value::String(name.clone()),
    );
    extra.insert(
        "reference-images".to_string(),
        serde_yaml::Value::Sequence(
            reference_images
                .iter()
                .map(|s| serde_yaml::Value::String(s.clone()))
                .collect(),
        ),
    );
    extra.insert(
        "character-type".to_string(),
        serde_yaml::Value::String(character_type.as_kebab().to_string()),
    );

    let body = render_body_html(&name, reference_images, character_type);
    let clip = Ulid::new();
    let asset_filename = format!("{slug}.clip.html");
    let clip_ref = ClipRef {
        clip,
        kind: ClipKind::CharacterRef,
        asset: PathBuf::from(asset_filename),
        asset_hash: hash_inputs(&name, reference_images),
        provider: "manual".into(),
        prompt: String::new(),
        created_at: Utc::now(),
        model: None,
        cost_usd: None,
        parent: None,
        edit_kind: None,
        edit_prompt: None,
        tags: Vec::new(),
        scene: None,
        extra,
    };
    clip_ref.write(&path, &body)?;
    Ok(Emission { path, clip, name })
}

/// Collection of loaded character refs keyed by `(canonical_name,
/// character_type)`. Carrying both axes in the key is what makes
/// hand-cutaway routing legible (wb-jwnk) — a single canonical name
/// like `ALEX` can own a `FullBody` ref *and* a `Hands` ref in
/// parallel, and the planner picks based on shot context.
#[derive(Debug, Clone, Default)]
pub struct CharacterRefs {
    inner: HashMap<(String, CharacterType), CharacterRef>,
}

impl CharacterRefs {
    /// Construct an empty collection.
    pub fn new() -> Self {
        Self { inner: HashMap::new() }
    }

    /// Insert a ref. Replaces any prior ref with the same
    /// `(name, character_type)` key.
    pub fn insert(&mut self, r: CharacterRef) {
        self.inner.insert((r.name.clone(), r.character_type), r);
    }

    /// Number of loaded refs across all types.
    pub fn len(&self) -> usize {
        self.inner.len()
    }

    /// True when no refs are loaded.
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    /// Iterate over every loaded ref.
    pub fn iter(&self) -> impl Iterator<Item = &CharacterRef> {
        self.inner.values()
    }

    /// Look up a specific (name, type) pair.
    pub fn get_typed(&self, name: &str, t: CharacterType) -> Option<&CharacterRef> {
        self.inner.get(&(name.to_string(), t))
    }

    /// Look up the full-body ref for a character. Returns `None` when
    /// no full-body ref was defined for that name.
    pub fn lookup_full(&self, name: &str) -> Option<&CharacterRef> {
        self.get_typed(name, CharacterType::FullBody)
    }

    /// Look up the hands-only ref for a character.
    pub fn lookup_hands(&self, name: &str) -> Option<&CharacterRef> {
        self.get_typed(name, CharacterType::Hands)
    }

    /// Look up the product-hands ref for a character.
    pub fn lookup_product_hands(&self, name: &str) -> Option<&CharacterRef> {
        self.get_typed(name, CharacterType::ProductHands)
    }

    /// Best-available lookup for a character — returns full-body when
    /// present, else hands, else product-hands. Used by callers that
    /// just want "any ref I can use for this character" without
    /// caring about the type axis.
    pub fn get(&self, name: &str) -> Option<&CharacterRef> {
        self.lookup_full(name)
            .or_else(|| self.lookup_hands(name))
            .or_else(|| self.lookup_product_hands(name))
    }

    /// True when the character has at least one ref of any type.
    pub fn contains_key(&self, name: &str) -> bool {
        self.get(name).is_some()
    }
}

/// Walk `<workdir>/refs/character/*.clip.html`, parse each one, and
/// return a `CharacterRefs` keyed by `(canonical_name, character_type)`.
/// Returns an empty collection when the directory doesn't exist.
pub fn load_characters(workdir: &Path) -> Result<CharacterRefs, ClipRefError> {
    let dir = workdir
        .join("refs")
        .join(ClipKind::CharacterRef.as_kebab());
    let mut out = CharacterRefs::new();
    if !dir.exists() {
        return Ok(out);
    }
    for entry in std::fs::read_dir(&dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("html") {
            continue;
        }
        let raw = std::fs::read_to_string(&path)?;
        let (clip, _body) = match ClipRef::parse(&raw) {
            Ok(parsed) => parsed,
            Err(_) => continue,
        };
        if clip.kind != ClipKind::CharacterRef {
            continue;
        }
        let Some(parsed) = parse_character_fields(&clip) else {
            continue;
        };
        out.insert(parsed);
    }
    Ok(out)
}

/// Pull the `name`, `reference-images`, and `character-type` fields
/// out of a parsed clip-ref's `extra` map. Returns `None` when the
/// required fields are absent or malformed — caller skips that file.
fn parse_character_fields(clip: &ClipRef) -> Option<CharacterRef> {
    let name = clip.extra.get("name")?.as_str()?.to_string();
    let refs_val = clip.extra.get("reference-images")?;
    let refs_seq = refs_val.as_sequence()?;
    let reference_images: Vec<String> = refs_seq
        .iter()
        .filter_map(|v| v.as_str().map(|s| s.to_string()))
        .collect();
    let character_type = clip
        .extra
        .get("character-type")
        .and_then(|v| v.as_str())
        .and_then(|s| match s {
            "full-body" => Some(CharacterType::FullBody),
            "hands" => Some(CharacterType::Hands),
            "product-hands" => Some(CharacterType::ProductHands),
            _ => None,
        })
        .unwrap_or_default();
    Some(CharacterRef {
        name,
        reference_images,
        character_type,
    })
}

/// Compose the on-disk slug from canonical name + character type.
/// Full-body keeps the bare lowercased name (wb-cx08 wire-compat);
/// hands / product-hands append a suffix so multi-type bundles for
/// the same canonical character don't clobber each other.
fn slug_with_type(name: &str, t: CharacterType) -> String {
    let base = name_to_slug(name);
    match t {
        CharacterType::FullBody => base,
        CharacterType::Hands => format!("{base}-hands"),
        CharacterType::ProductHands => format!("{base}-product-hands"),
    }
}

fn name_to_slug(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    let mut last_dash = true;
    for c in name.chars() {
        let lc = c.to_ascii_lowercase();
        if lc.is_ascii_alphanumeric() {
            out.push(lc);
            last_dash = false;
        } else if !last_dash {
            out.push('-');
            last_dash = true;
        }
    }
    if out.ends_with('-') {
        out.pop();
    }
    if out.is_empty() {
        out.push_str("character");
    }
    out
}

fn hash_inputs(name: &str, refs: &[String]) -> String {
    use std::hash::{Hash, Hasher};
    let mut h = twox_hash::XxHash64::with_seed(0);
    name.hash(&mut h);
    for r in refs {
        r.hash(&mut h);
    }
    format!("{:016x}", h.finish())
}

fn render_body_html(
    name: &str,
    refs: &[String],
    character_type: CharacterType,
) -> String {
    let mut out = String::new();
    out.push_str("\n<article class=\"character-ref\">\n");
    out.push_str(&format!(
        "  <h2 class=\"name\">{}</h2>\n",
        html_escape(name)
    ));
    out.push_str(&format!(
        "  <p class=\"character-type\">{}</p>\n",
        html_escape(character_type.as_kebab())
    ));
    out.push_str("  <ul class=\"references\">\n");
    for r in refs {
        out.push_str(&format!(
            "    <li><img src=\"{}\" alt=\"reference\" /></li>\n",
            html_escape(r)
        ));
    }
    out.push_str("  </ul>\n");
    out.push_str("</article>\n");
    out
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn emit_and_load_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let opts = EmitOptions { workdir: dir.path() };
        let refs = vec![
            "./dana-face-1.jpg".to_string(),
            "https://example.com/dana-2.jpg".to_string(),
        ];
        let emission =
            emit_character("Dana", &refs, CharacterType::FullBody, &opts).unwrap();
        assert!(emission.path.exists());
        assert_eq!(emission.name, "DANA");
        assert!(emission
            .path
            .to_string_lossy()
            .contains("refs/character/dana.clip.html"));

        let loaded = load_characters(dir.path()).unwrap();
        assert_eq!(loaded.len(), 1);
        let dana = loaded.lookup_full("DANA").expect("DANA loaded");
        assert_eq!(dana.name, "DANA");
        assert_eq!(dana.reference_images, refs);
        assert_eq!(dana.character_type, CharacterType::FullBody);
    }

    #[test]
    fn canonicalizes_extension_form_on_emit() {
        let dir = tempfile::tempdir().unwrap();
        let opts = EmitOptions { workdir: dir.path() };
        let refs = vec!["a.jpg".to_string()];
        let emission =
            emit_character("Alex (V.O.)", &refs, CharacterType::FullBody, &opts).unwrap();
        // Canonical key drops the extension and uppercases.
        assert_eq!(emission.name, "ALEX");
        let loaded = load_characters(dir.path()).unwrap();
        assert!(loaded.contains_key("ALEX"));
    }

    #[test]
    fn character_type_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let opts = EmitOptions { workdir: dir.path() };
        let refs = vec!["a.jpg".to_string()];
        emit_character("HANDS", &refs, CharacterType::ProductHands, &opts).unwrap();
        let loaded = load_characters(dir.path()).unwrap();
        assert_eq!(
            loaded.lookup_product_hands("HANDS").unwrap().character_type,
            CharacterType::ProductHands
        );
    }

    #[test]
    fn load_returns_empty_when_dir_missing() {
        let dir = tempfile::tempdir().unwrap();
        let loaded = load_characters(dir.path()).unwrap();
        assert!(loaded.is_empty());
    }

    /// wb-jwnk: a character with both full-body and hands refs loads
    /// both, keyed by `(name, type)`. `lookup_full` / `lookup_hands`
    /// return the right bundle; the on-disk filenames differ via the
    /// `-hands` suffix so neither clobbers the other.
    #[test]
    fn dual_type_refs_for_same_character_coexist() {
        let dir = tempfile::tempdir().unwrap();
        let opts = EmitOptions { workdir: dir.path() };

        let full = emit_character(
            "DANA",
            &["./dana-face-1.jpg".to_string()],
            CharacterType::FullBody,
            &opts,
        )
        .unwrap();
        let hands = emit_character(
            "DANA",
            &["./dana-hands.jpg".to_string()],
            CharacterType::Hands,
            &opts,
        )
        .unwrap();
        // Slugs differ — no overwrite.
        assert_ne!(full.path, hands.path);
        assert!(full.path.to_string_lossy().ends_with("dana.clip.html"));
        assert!(hands.path.to_string_lossy().ends_with("dana-hands.clip.html"));

        let loaded = load_characters(dir.path()).unwrap();
        assert_eq!(loaded.len(), 2, "both refs should load");
        let f = loaded.lookup_full("DANA").expect("full-body");
        assert_eq!(f.character_type, CharacterType::FullBody);
        assert_eq!(f.reference_images, vec!["./dana-face-1.jpg".to_string()]);
        let h = loaded.lookup_hands("DANA").expect("hands");
        assert_eq!(h.character_type, CharacterType::Hands);
        assert_eq!(h.reference_images, vec!["./dana-hands.jpg".to_string()]);
        // Product-hands wasn't defined.
        assert!(loaded.lookup_product_hands("DANA").is_none());
    }

    /// `get()` is a best-available helper — prefers full-body, then
    /// hands, then product-hands. Tested explicitly because the
    /// planner's Dialogue branch leans on it.
    #[test]
    fn best_available_lookup_prefers_full_body() {
        let mut refs = CharacterRefs::new();
        refs.insert(CharacterRef {
            name: "X".into(),
            reference_images: vec!["hand.jpg".into()],
            character_type: CharacterType::Hands,
        });
        // Only hands loaded — `get` returns it.
        assert_eq!(
            refs.get("X").unwrap().character_type,
            CharacterType::Hands,
        );
        // Add full-body — `get` now returns the full-body one.
        refs.insert(CharacterRef {
            name: "X".into(),
            reference_images: vec!["face.jpg".into()],
            character_type: CharacterType::FullBody,
        });
        assert_eq!(
            refs.get("X").unwrap().character_type,
            CharacterType::FullBody,
        );
    }

    #[test]
    fn kind_serializes_as_character_ref() {
        // Sanity check: serde kebab-case rename gives `character-ref` on the
        // wire even though the on-disk directory is `refs/character/`.
        let yaml = serde_yaml::to_string(&ClipKind::CharacterRef).unwrap();
        assert!(yaml.contains("character-ref"), "got {yaml}");
    }

    #[test]
    fn name_to_slug_lowercases_and_handles_punctuation() {
        assert_eq!(name_to_slug("DANA"), "dana");
        assert_eq!(name_to_slug("DR. SMITH"), "dr-smith");
        assert_eq!(name_to_slug("MARIE-CLAIRE"), "marie-claire");
    }
}
