//! Clip-ref resolution pre-pass (wb-n33n.4).
//!
//! Walks scene HTML for `<wavelet-clip src="…">` elements, loads the
//! referenced `.clip.html` via the schema crate, and rewrites the source
//! HTML to substitute each `<wavelet-clip>` with the asset element implied
//! by the clip-ref's `kind`:
//!
//! - `still` / `scene-still` → `<img src="…">`
//! - `shot`                  → `<video src="…" autoplay muted playsinline>`
//! - `overlay`               → the clip-ref's HTML body inlined verbatim
//! - `music` / `tts`         → empty (the cue is hoisted into `audio_cues`)
//! - `screenplay-scene`      → empty (storyboard/velocity only)
//!
//! Pass-through attributes (`id`, `class`, `style`, plus any `data-*`)
//! on the original `<wavelet-clip>` transfer onto the substituted element.
//! The `src` attribute is *not* copied — it points at the clip-ref, not
//! the asset.
//!
//! Asset paths in the clip-ref are resolved relative to the clip-ref
//! file, then re-expressed relative to the scene HTML so the final DOM
//! base-URL still resolves them correctly.

use std::path::{Component, Path, PathBuf};

use crate::clipref::{ClipKind, ClipRef, ClipRefError};
use crate::render_offline::AudioCueSpec;

/// Errors raised by the pre-pass. Missing files / parse failures surface
/// as `Resolve` so the compose loader can attach the manifest path.
#[derive(Debug, thiserror::Error)]
pub enum ResolveError {
    /// Failed to read a referenced `.clip.html`.
    #[error("read {path}: {source}")]
    Io {
        /// Path the resolver tried to read.
        path: String,
        /// Underlying I/O error.
        #[source]
        source: std::io::Error,
    },
    /// `.clip.html` parse failure.
    #[error("parse {path}: {source}")]
    Parse {
        /// Path of the clip-ref being parsed.
        path: String,
        /// Underlying schema error.
        #[source]
        source: ClipRefError,
    },
    /// `<wavelet-clip>` is missing its `src` attribute.
    #[error("<wavelet-clip> missing src attribute")]
    MissingSrc,
}

/// Replace every `<wavelet-clip src="…" …>…</wavelet-clip>` (and self-closing
/// variants) in `html` with the substitute element implied by the clip-ref.
///
/// `scene_dir` is the directory containing the scene HTML — `src` is
/// resolved against it.
pub fn resolve_clip_refs(html: &str, scene_dir: &Path) -> Result<String, ResolveError> {
    let mut out = String::with_capacity(html.len());
    let mut cursor = 0usize;
    for tag in find_clip_tags(html) {
        out.push_str(&html[cursor..tag.start]);
        let replacement = substitute(html, &tag, scene_dir)?;
        out.push_str(&replacement);
        cursor = tag.end;
    }
    out.push_str(&html[cursor..]);
    Ok(out)
}

/// Walk every scene HTML in the composition, collect music/tts clip-refs,
/// and return them as `AudioCueSpec`s with default timing (cue spans full
/// scene duration, no fades).
///
/// Cues returned here are appended to the composition's `audio_cues` and
/// then resolved against `total_frames` by the same `data-spans="all"`
/// logic the manifest already uses for `<audio>` elements.
pub fn extract_audio_clip_cues(
    html: &str,
    scene_dir: &Path,
    fps: u32,
    scene_start_frame: u32,
    scene_duration_frames: u32,
) -> Result<Vec<AudioCueSpec>, ResolveError> {
    let mut out = Vec::new();
    for tag in find_clip_tags(html) {
        let src = attr_value(&html[tag.attrs_start..tag.attrs_end], "src")
            .ok_or(ResolveError::MissingSrc)?;
        let clip_path = scene_dir.join(&src);
        let (clip, _body) = load_clip_ref(&clip_path)?;
        if !matches!(clip.kind, ClipKind::Music | ClipKind::Tts) {
            continue;
        }
        let asset_path = resolve_asset_path(&clip_path, &clip.asset);
        let id = attr_value(&html[tag.attrs_start..tag.attrs_end], "id")
            .unwrap_or_else(|| derive_cue_id(&asset_path));
        // Stretch the cue across the scene. Music typically spans the full
        // scene; tts cues land at scene start. Both can be tuned later via
        // explicit `<audio>` overrides — the pre-pass only auto-injects a
        // sensible default.
        let _ = fps;
        out.push(AudioCueSpec {
            asset_path,
            id,
            start_frame: scene_start_frame,
            duration_frames: scene_duration_frames,
            volume: 1.0,
            pan: 0.0,
            fade_in_frames: 0,
            fade_out_frames: 0,
            duck_targets: Vec::new(),
            duck_db: 0.0,
            align_to_beat: false,
        });
    }
    Ok(out)
}

struct ClipTag {
    /// Byte index of the leading `<`.
    start: usize,
    /// Byte index one past the trailing `>` of the closing (or self-closing) tag.
    end: usize,
    /// Byte index of the first attribute char on the opening tag.
    attrs_start: usize,
    /// Byte index just past the last attribute char on the opening tag.
    attrs_end: usize,
}

fn find_clip_tags(html: &str) -> Vec<ClipTag> {
    let bytes = html.as_bytes();
    let mut out = Vec::new();
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] != b'<' {
            i += 1;
            continue;
        }
        if !html[i..].to_ascii_lowercase().starts_with("<wavelet-clip") {
            i += 1;
            continue;
        }
        let name_end = i + "<wavelet-clip".len();
        if name_end < bytes.len() {
            let c = bytes[name_end];
            if c.is_ascii_alphanumeric() || c == b'-' {
                // e.g. <wavelet-clip-foo> — not us.
                i = name_end;
                continue;
            }
        }
        let open_end = match html[name_end..].find('>') {
            Some(p) => name_end + p,
            None => break,
        };
        let attrs_slice = html[name_end..open_end].trim();
        let self_closing = attrs_slice.ends_with('/');
        let attrs_inner_start = name_end;
        let attrs_inner_end =
            if self_closing { open_end - 1 } else { open_end };
        let opening_end_excl = open_end + 1;
        let end = if self_closing {
            opening_end_excl
        } else {
            let needle_lower = "</wavelet-clip>";
            match find_ci(&html[opening_end_excl..], needle_lower) {
                Some(rel) => opening_end_excl + rel + needle_lower.len(),
                None => opening_end_excl,
            }
        };
        out.push(ClipTag {
            start: i,
            end,
            attrs_start: attrs_inner_start,
            attrs_end: attrs_inner_end,
        });
        i = end;
    }
    out
}

fn substitute(
    html: &str,
    tag: &ClipTag,
    scene_dir: &Path,
) -> Result<String, ResolveError> {
    let attrs_text = &html[tag.attrs_start..tag.attrs_end];
    let src = attr_value(attrs_text, "src").ok_or(ResolveError::MissingSrc)?;
    let clip_path = scene_dir.join(&src);
    let (clip, body) = load_clip_ref(&clip_path)?;
    let asset_path = resolve_asset_path(&clip_path, &clip.asset);
    let asset_rel = relative_from(&asset_path, scene_dir);
    let asset_str = path_to_url_string(&asset_rel);
    let pass_through = passthrough_attrs(attrs_text);

    Ok(match clip.kind {
        ClipKind::Still | ClipKind::SceneStill => {
            format!(r#"<img src="{asset_str}"{pass_through}>"#)
        }
        ClipKind::Shot => format!(
            r#"<video src="{asset_str}" autoplay muted playsinline{pass_through}></video>"#
        ),
        ClipKind::Overlay => body,
        ClipKind::Music | ClipKind::Tts | ClipKind::ScreenplayScene | ClipKind::Caption
        | ClipKind::CharacterRef => {
            String::new()
        }
    })
}

fn load_clip_ref(path: &Path) -> Result<(ClipRef, String), ResolveError> {
    let raw = std::fs::read_to_string(path).map_err(|e| ResolveError::Io {
        path: path.display().to_string(),
        source: e,
    })?;
    ClipRef::parse(&raw).map_err(|e| ResolveError::Parse {
        path: path.display().to_string(),
        source: e,
    })
}

/// Build the pass-through attribute fragment ` foo="bar" baz="qux"`.
/// Leading space included when non-empty.
fn passthrough_attrs(attrs_text: &str) -> String {
    let mut out = String::new();
    for (k, v) in parse_attrs(attrs_text) {
        if k.eq_ignore_ascii_case("src") {
            continue;
        }
        out.push(' ');
        out.push_str(&k);
        if !v.is_empty() {
            out.push_str("=\"");
            out.push_str(&escape_attr(&v));
            out.push('"');
        }
    }
    out
}

fn escape_attr(v: &str) -> String {
    v.replace('&', "&amp;").replace('"', "&quot;")
}

fn resolve_asset_path(clip_path: &Path, asset: &Path) -> PathBuf {
    if asset.is_absolute() {
        return asset.to_path_buf();
    }
    let parent = clip_path.parent().unwrap_or_else(|| Path::new("."));
    normalize(&parent.join(asset))
}

fn relative_from(target: &Path, base: &Path) -> PathBuf {
    let t = normalize(target);
    let b = normalize(base);
    let tc: Vec<_> = t.components().collect();
    let bc: Vec<_> = b.components().collect();
    let mut i = 0;
    while i < tc.len() && i < bc.len() && tc[i] == bc[i] {
        i += 1;
    }
    let mut out = PathBuf::new();
    for _ in i..bc.len() {
        out.push("..");
    }
    for c in &tc[i..] {
        out.push(c.as_os_str());
    }
    if out.as_os_str().is_empty() {
        out.push(".");
    }
    out
}

fn normalize(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// Convert a filesystem path to a URL-style string with forward slashes.
fn path_to_url_string(p: &Path) -> String {
    let s = p.to_string_lossy().into_owned();
    if std::path::MAIN_SEPARATOR == '/' {
        s
    } else {
        s.replace('\\', "/")
    }
}

fn derive_cue_id(p: &Path) -> String {
    p.file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("clipref")
        .to_string()
}

fn find_ci(hay: &str, needle: &str) -> Option<usize> {
    let hay_lower = hay.to_ascii_lowercase();
    hay_lower.find(needle)
}

fn attr_value(body: &str, key: &str) -> Option<String> {
    parse_attrs(body)
        .into_iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(key))
        .map(|(_, v)| v)
}

fn parse_attrs(body: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let bytes = body.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= bytes.len() {
            break;
        }
        let key_start = i;
        while i < bytes.len()
            && !bytes[i].is_ascii_whitespace()
            && bytes[i] != b'='
            && bytes[i] != b'/'
        {
            i += 1;
        }
        if i == key_start {
            break;
        }
        let key = body[key_start..i].to_string();
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= bytes.len() || bytes[i] != b'=' {
            out.push((key, String::new()));
            continue;
        }
        i += 1;
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= bytes.len() {
            out.push((key, String::new()));
            break;
        }
        let quote = bytes[i];
        let value = if quote == b'"' || quote == b'\'' {
            i += 1;
            let start = i;
            while i < bytes.len() && bytes[i] != quote {
                i += 1;
            }
            let v = body[start..i].to_string();
            if i < bytes.len() {
                i += 1;
            }
            v
        } else {
            let start = i;
            while i < bytes.len() && !bytes[i].is_ascii_whitespace() {
                i += 1;
            }
            body[start..i].to_string()
        };
        out.push((key, decode_entities(&value)));
    }
    out
}

fn decode_entities(s: &str) -> String {
    s.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clipref::{default_path, slug_for};
    use chrono::Utc;
    use std::collections::BTreeMap;
    use ulid::Ulid;

    fn write_clip(
        workdir: &Path,
        kind: ClipKind,
        prompt: &str,
        asset_rel: &str,
        body: &str,
    ) -> PathBuf {
        let slug = slug_for(kind, None, prompt, "abc123");
        let path = default_path(workdir, kind, &slug);
        let clip = ClipRef {
            clip: Ulid::new(),
            kind,
            asset: PathBuf::from(asset_rel),
            asset_hash: "abc123".into(),
            provider: "google".into(),
            prompt: prompt.into(),
            created_at: Utc::now(),
            model: None,
            cost_usd: None,
            parent: None,
            edit_kind: None,
            edit_prompt: None,
            tags: vec![],
            scene: None,
            extra: BTreeMap::new(),
        };
        clip.write(&path, body).unwrap();
        path
    }

    #[test]
    fn resolves_shot_into_video_element() {
        let dir = tempfile::tempdir().unwrap();
        let workdir = dir.path();
        std::fs::create_dir_all(workdir.join("scenes")).unwrap();
        std::fs::create_dir_all(workdir.join("assets")).unwrap();
        std::fs::write(workdir.join("assets/hero.mp4"), b"x").unwrap();

        write_clip(
            workdir,
            ClipKind::Shot,
            "hero shot",
            "../../assets/hero.mp4",
            "\n<video src=\"../../assets/hero.mp4\"></video>\n",
        );

        let scene_dir = workdir.join("scenes");
        let scene_html = r#"<html><body>
<wavelet-clip src="../refs/shot/shot-none-hero-shot-abc123.clip.html" class="bg"></wavelet-clip>
</body></html>"#;
        let out = resolve_clip_refs(scene_html, &scene_dir).unwrap();
        assert!(out.contains("<video"), "video tag emitted: {out}");
        assert!(out.contains("autoplay"), "shot defaults: {out}");
        assert!(out.contains(r#"class="bg""#), "passthrough class: {out}");
        assert!(!out.contains("<wavelet-clip"), "wavelet-clip removed: {out}");
        assert!(out.contains("hero.mp4"), "asset path present: {out}");
    }

    #[test]
    fn resolves_still_into_img_element() {
        let dir = tempfile::tempdir().unwrap();
        let workdir = dir.path();
        std::fs::create_dir_all(workdir.join("scenes")).unwrap();
        std::fs::create_dir_all(workdir.join("assets")).unwrap();
        std::fs::write(workdir.join("assets/photo.png"), b"x").unwrap();

        write_clip(
            workdir,
            ClipKind::Still,
            "still photo",
            "../../assets/photo.png",
            "\n<img src=\"../../assets/photo.png\" />\n",
        );

        let scene_dir = workdir.join("scenes");
        let scene_html = r#"<wavelet-clip src="../refs/still/still-none-still-photo-abc123.clip.html" id="hero" />"#;
        let out = resolve_clip_refs(scene_html, &scene_dir).unwrap();
        assert!(out.contains("<img "), "img tag emitted: {out}");
        assert!(out.contains(r#"id="hero""#), "passthrough id: {out}");
        assert!(out.contains("photo.png"), "asset path present: {out}");
    }

    #[test]
    fn resolves_overlay_inlines_body() {
        let dir = tempfile::tempdir().unwrap();
        let workdir = dir.path();
        std::fs::create_dir_all(workdir.join("scenes")).unwrap();
        std::fs::create_dir_all(workdir.join("assets")).unwrap();

        write_clip(
            workdir,
            ClipKind::Overlay,
            "title card",
            "title.html",
            "\n<div class=\"title\">Welcome</div>\n",
        );

        let scene_dir = workdir.join("scenes");
        let scene_html = r#"<wavelet-clip src="../refs/overlay/overlay-none-title-card-abc123.clip.html"></wavelet-clip>"#;
        let out = resolve_clip_refs(scene_html, &scene_dir).unwrap();
        assert!(out.contains("Welcome"), "overlay body inlined: {out}");
        assert!(!out.contains("<wavelet-clip"), "wavelet-clip removed");
    }

    #[test]
    fn missing_src_errors() {
        let dir = tempfile::tempdir().unwrap();
        let scene_html = r#"<wavelet-clip></wavelet-clip>"#;
        let err = resolve_clip_refs(scene_html, dir.path()).err().unwrap();
        assert!(matches!(err, ResolveError::MissingSrc));
    }

    #[test]
    fn missing_clipref_file_errors_with_path() {
        let dir = tempfile::tempdir().unwrap();
        let scene_html = r#"<wavelet-clip src="missing.clip.html"></wavelet-clip>"#;
        let err = resolve_clip_refs(scene_html, dir.path()).err().unwrap();
        match err {
            ResolveError::Io { path, .. } => assert!(path.contains("missing.clip.html")),
            other => panic!("expected Io, got {other:?}"),
        }
    }

    #[test]
    fn music_clipref_emits_audio_cue() {
        let dir = tempfile::tempdir().unwrap();
        let workdir = dir.path();
        std::fs::create_dir_all(workdir.join("scenes")).unwrap();
        std::fs::create_dir_all(workdir.join("assets")).unwrap();
        std::fs::write(workdir.join("assets/track.wav"), b"x").unwrap();

        write_clip(
            workdir,
            ClipKind::Music,
            "ambient bed",
            "../../assets/track.wav",
            "\n<audio src=\"../../assets/track.wav\"></audio>\n",
        );

        let scene_dir = workdir.join("scenes");
        let scene_html = r#"<wavelet-clip src="../refs/music/music-none-ambient-bed-abc123.clip.html"></wavelet-clip>"#;

        let cues =
            extract_audio_clip_cues(scene_html, &scene_dir, 30, 60, 180).unwrap();
        assert_eq!(cues.len(), 1);
        assert_eq!(cues[0].start_frame, 60);
        assert_eq!(cues[0].duration_frames, 180);
        assert!(cues[0].asset_path.ends_with("track.wav"));

        // And the substitution removes it from the inline DOM.
        let out = resolve_clip_refs(scene_html, &scene_dir).unwrap();
        assert!(!out.contains("<audio"), "music is NOT inlined as audio");
        assert!(!out.contains("<wavelet-clip"), "wavelet-clip removed");
    }

    #[test]
    fn non_clip_html_is_unchanged() {
        let dir = tempfile::tempdir().unwrap();
        let scene_html =
            r#"<html><body><h1>hi</h1><p>plain</p></body></html>"#;
        let out = resolve_clip_refs(scene_html, dir.path()).unwrap();
        assert_eq!(out, scene_html);
    }
}
