//! Screenplay scene → clip-ref split + reassemble (wb-n33n.5).
//!
//! Splits a Fountain source into per-scene chunks at scene-heading
//! boundaries. Each chunk becomes a `kind: screenplay-scene` clip-ref.
//! `reassemble` concatenates them back into one fountain — byte-identical
//! for canonical-formatted input.
//!
//! The split is purely textual (no AST round-trip needed): we find the
//! line offsets of every scene heading and slice between them. Title
//! page and pre-first-scene body go into a synthetic scene 0 ("prelude"),
//! so reassembly is lossless even when the screenplay opens with action
//! before its first slugline.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use chrono::Utc;
use ulid::Ulid;

use super::{ClipKind, ClipRef, ClipRefError};

/// One scene extracted from a fountain source. Holds the exact byte
/// range so reassembly is a straight concat.
pub struct SceneChunk {
    /// 1-based scene index. Index `0` is the prelude (title page +
    /// anything before the first scene heading), present only when
    /// non-empty.
    pub index: u32,
    /// Slugline if this chunk has one. Empty for the prelude.
    pub slugline: String,
    /// The full fountain text of this scene including any trailing
    /// blank lines that belong to it.
    pub text: String,
}

/// Split a fountain source into per-scene chunks.
pub fn split_scenes(source: &str) -> Vec<SceneChunk> {
    let mut starts: Vec<(usize, String)> = Vec::new();
    let bytes = source.as_bytes();
    let mut line_start = 0usize;
    let mut prev_blank = true;
    let mut i = 0usize;
    while i <= bytes.len() {
        if i == bytes.len() || bytes[i] == b'\n' {
            let line = &source[line_start..i];
            let line_trimmed = line.trim();
            // Heading rule: matches uppercase-prefix list AND is preceded
            // by a blank line (Fountain spec). The first line of the
            // body is treated as preceded-by-blank too.
            let forced = line_trimmed.starts_with('.')
                && !line_trimmed.starts_with("..");
            let unforced = looks_like_scene_heading(line_trimmed);
            if prev_blank && (forced || unforced) && !line_trimmed.is_empty() {
                let slug = if forced {
                    line_trimmed[1..].trim().to_string()
                } else {
                    line_trimmed.to_string()
                };
                starts.push((line_start, slug));
            }
            prev_blank = line_trimmed.is_empty();
            line_start = i + 1;
        }
        i += 1;
    }

    let mut out = Vec::new();
    let prelude_end = starts.first().map(|s| s.0).unwrap_or(source.len());
    if prelude_end > 0 {
        out.push(SceneChunk {
            index: 0,
            slugline: String::new(),
            text: source[..prelude_end].to_string(),
        });
    }
    for (i, (start, slug)) in starts.iter().enumerate() {
        let end = starts
            .get(i + 1)
            .map(|s| s.0)
            .unwrap_or(source.len());
        out.push(SceneChunk {
            index: (i + 1) as u32,
            slugline: slug.clone(),
            text: source[*start..end].to_string(),
        });
    }
    out
}

/// Concatenate scene chunks back into one fountain string. Scenes are
/// sorted by `index` so callers can hand us files read in any order.
pub fn reassemble(mut chunks: Vec<SceneChunk>) -> String {
    chunks.sort_by_key(|c| c.index);
    let mut out = String::new();
    for c in chunks {
        out.push_str(&c.text);
    }
    out
}

/// Per-scene clip-ref builder. `fountain_path` is the source file that
/// scenes refer back to in their `asset:` field — there's no rendered
/// binary for a screenplay-scene clip-ref, just the source-text pointer.
pub struct EmitOptions<'a> {
    /// Project workdir. Clip-refs land at `<workdir>/refs/screenplay-scene/`.
    pub workdir: &'a Path,
    /// Source fountain file, expressed however you want it stored in
    /// `asset:` (usually relative to the clip-ref's directory).
    pub fountain_asset: &'a Path,
    /// Optional parent ULID for lineage when re-emitting a rewritten
    /// scene. `edit_kind` is set to `Regenerate` when present.
    pub parent: Option<Ulid>,
}

/// Result of one scene's emission.
pub struct Emission {
    /// Path the clip-ref was written to.
    pub path: PathBuf,
    /// ULID assigned to the clip-ref.
    pub clip: Ulid,
}

/// Emit one clip-ref per scene. Returns the emission record per chunk
/// in input order (skipping the prelude when present — it has no
/// slugline and doesn't render as a scene).
pub fn emit_scenes(
    chunks: &[SceneChunk],
    opts: &EmitOptions<'_>,
) -> Result<Vec<Emission>, ClipRefError> {
    let mut out = Vec::with_capacity(chunks.len());
    for chunk in chunks {
        if chunk.index == 0 {
            continue;
        }
        let asset_hash = hash_text(&chunk.text);
        let slug = scene_slug(chunk.index, &chunk.slugline, &asset_hash);
        let path = super::default_path(opts.workdir, ClipKind::ScreenplayScene, &slug);

        let body = render_scene_html(&chunk.slugline, &chunk.text);

        let clip = Ulid::new();
        let edit_kind = opts.parent.map(|_| super::EditKind::Regenerate);
        let clip_ref = ClipRef {
            clip,
            kind: ClipKind::ScreenplayScene,
            asset: opts.fountain_asset.to_path_buf(),
            asset_hash,
            provider: "wavelet".into(),
            prompt: chunk.text.clone(),
            created_at: Utc::now(),
            model: None,
            cost_usd: None,
            parent: opts.parent,
            edit_kind,
            edit_prompt: None,
            tags: Vec::new(),
            scene: Some(format!("{:02}", chunk.index)),
            extra: scene_extra(&chunk.slugline),
        };
        clip_ref.write(&path, &body)?;
        out.push(Emission { path, clip });
    }
    Ok(out)
}

/// Walk `<workdir>/refs/screenplay-scene/` and read every clip-ref into
/// a chunk. Sorted by `scene` index. Returns an empty vec when the
/// directory doesn't exist (no screenplay split yet).
pub fn load_scenes(workdir: &Path) -> Result<Vec<SceneChunk>, ClipRefError> {
    let dir = workdir.join("refs").join("screenplay-scene");
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    for entry in std::fs::read_dir(&dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("html") {
            continue;
        }
        let raw = std::fs::read_to_string(&path)?;
        let (clip, _body) = ClipRef::parse(&raw)?;
        let index = clip
            .scene
            .as_deref()
            .and_then(|s| s.parse::<u32>().ok())
            .unwrap_or(0);
        let slugline = clip
            .extra
            .get("slugline")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .unwrap_or_default();
        out.push(SceneChunk {
            index,
            slugline,
            text: clip.prompt,
        });
    }
    out.sort_by_key(|c| c.index);
    Ok(out)
}

fn scene_extra(slugline: &str) -> BTreeMap<String, serde_yaml::Value> {
    let mut m = BTreeMap::new();
    if !slugline.is_empty() {
        m.insert(
            "slugline".to_string(),
            serde_yaml::Value::String(slugline.to_string()),
        );
    }
    m
}

fn hash_text(s: &str) -> String {
    use std::hash::{Hash, Hasher};
    let mut h = twox_hash::XxHash64::with_seed(0);
    s.hash(&mut h);
    format!("{:016x}", h.finish())
}

fn scene_slug(index: u32, slugline: &str, hash: &str) -> String {
    let scene_part = if slugline.is_empty() {
        "untitled".to_string()
    } else {
        sluggify(slugline)
    };
    let hash_part: String = hash.chars().take(6).collect();
    format!("{:03}-{scene_part}-{hash_part}", index)
}

fn sluggify(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut last_dash = true;
    for c in input.chars() {
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
    let truncated: String = out.chars().take(48).collect();
    truncated.trim_end_matches('-').to_string()
}

fn looks_like_scene_heading(line: &str) -> bool {
    let u = line.to_ascii_uppercase();
    const PREFIXES: &[&str] = &[
        "INT.", "EXT.", "EST.", "INT/EXT", "INT./EXT.", "I/E", "I./E.",
    ];
    PREFIXES.iter().any(|p| u.starts_with(*p))
}

fn render_scene_html(slugline: &str, fountain_text: &str) -> String {
    let escaped_text = html_escape(fountain_text);
    let escaped_slug = html_escape(slugline);
    format!(
        "\n<article class=\"screenplay-scene\">\n\
         <h2 class=\"slugline\">{escaped_slug}</h2>\n\
         <pre class=\"fountain\">{escaped_text}</pre>\n\
         </article>\n"
    )
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "\
Title: Coffee
Author: Anon

INT. KITCHEN - DAY

Anna pours water in a slow steady stream.

EXT. STREET - DAY

She crosses, holding the cup.
";

    #[test]
    fn split_finds_two_scenes_plus_prelude() {
        let chunks = split_scenes(SAMPLE);
        assert_eq!(chunks.len(), 3, "prelude + 2 scenes");
        assert_eq!(chunks[0].index, 0);
        assert_eq!(chunks[1].index, 1);
        assert_eq!(chunks[1].slugline, "INT. KITCHEN - DAY");
        assert_eq!(chunks[2].slugline, "EXT. STREET - DAY");
    }

    #[test]
    fn reassemble_is_byte_identical() {
        let chunks = split_scenes(SAMPLE);
        let back = reassemble(chunks);
        assert_eq!(back, SAMPLE);
    }

    #[test]
    fn emit_writes_per_scene_files() {
        let dir = tempfile::tempdir().unwrap();
        let workdir = dir.path();
        let chunks = split_scenes(SAMPLE);
        let opts = EmitOptions {
            workdir,
            fountain_asset: Path::new("../../script.fountain"),
            parent: None,
        };
        let emissions = emit_scenes(&chunks, &opts).unwrap();
        assert_eq!(emissions.len(), 2, "prelude isn't emitted");
        for e in &emissions {
            assert!(e.path.exists(), "{}", e.path.display());
            assert!(e.path.to_string_lossy().contains("refs/screenplay-scene/"));
        }
    }

    #[test]
    fn load_after_emit_round_trips_text() {
        let dir = tempfile::tempdir().unwrap();
        let workdir = dir.path();
        let chunks = split_scenes(SAMPLE);
        let opts = EmitOptions {
            workdir,
            fountain_asset: Path::new("../../script.fountain"),
            parent: None,
        };
        let _ = emit_scenes(&chunks, &opts).unwrap();

        let loaded = load_scenes(workdir).unwrap();
        // Reassembling the loaded scenes drops the prelude; concatenating
        // them yields the post-prelude tail of SAMPLE.
        let tail = reassemble(loaded);
        let prelude_end = SAMPLE.find("INT.").unwrap();
        assert_eq!(tail, SAMPLE[prelude_end..]);
    }

    #[test]
    fn forced_scene_heading_with_dot() {
        let src = "\nSome action.\n\n.UNDERWATER\n\nMore action.\n";
        let chunks = split_scenes(src);
        let scenes: Vec<_> = chunks.iter().filter(|c| c.index > 0).collect();
        assert_eq!(scenes.len(), 1);
        assert_eq!(scenes[0].slugline, "UNDERWATER");
    }

    #[test]
    fn slug_is_stable() {
        let a = scene_slug(3, "INT. KITCHEN - DAY", "abcdef123");
        let b = scene_slug(3, "INT. KITCHEN - DAY", "abcdef123");
        assert_eq!(a, b);
        assert!(a.starts_with("003-"));
    }
}
