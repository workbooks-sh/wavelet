//! Clip-ref emission wired onto `AssetCache` (wb-n33n.2).
//!
//! Adds `AssetCache::store_with_clip` — stores the manifest as today AND
//! writes a sibling `.clip.html` clip-ref under `<workdir>/refs/<kind>/`.
//! Legacy `AssetCache::store` is unchanged.
//!
//! Idempotency: if the target clip-ref already exists and its `asset_hash`
//! matches `manifest.request_hash`, we return the existing path without
//! re-minting the ULID. Slug-hash collisions surface as `BackendError::Cache`.
//!
//! ## Field-name notes
//!
//! - `Manifest.asset_path` is `Option<String>` and is relative to the cache
//!   root (not the workdir). We resolve it against `cache.root()` to get
//!   the absolute blob path, then compute the workdir-relative `asset:`
//!   field by walking the path with `relative_from` below (no `pathdiff`
//!   dep — see Cargo.toml; we keep the dep surface minimal).
//! - `Manifest.cost_estimate_usd` is `f32`. Zero → `cost_usd = None` per
//!   spec; non-zero → `Some(value)`.
//! - `request_hash` (xxhash64, 16 hex) is what we put into `asset_hash`
//!   on the clip-ref — the schema doc says "sha256" but the actual
//!   producer hash is xxhash64. Naming is informational; uniqueness is
//!   what matters.

use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};

use chrono::Utc;
use ulid::Ulid;

use super::cache::{AssetCache, Manifest};
use super::BackendError;
use crate::clipref::{default_path, slug_for, ClipKind, ClipRef, EditKind};

/// Context for one clip-ref emission. Kept small — anything that varies
/// per producer call lives here; everything else is on the manifest.
pub struct ClipEmitContext<'a> {
    /// Project workdir. Clip-refs land at `<workdir>/refs/<kind>/`.
    pub workdir: &'a Path,
    /// Producer-declared clip kind. Drives slug + path + preview body.
    pub kind: ClipKind,
    /// Natural-language prompt that produced this asset.
    pub prompt: String,
    /// Provider-specific model slug (e.g. `veo-3.1-generate-preview`).
    pub model: Option<String>,
    /// ULID of parent clip-ref, if this is an edit. Validated at write time.
    pub parent: Option<String>,
    /// Edit kind — set iff `parent` is set.
    pub edit_kind: Option<EditKind>,
    /// Edit instruction text (e.g. "make the sky bluer").
    pub edit_prompt: Option<String>,
    /// Scene slug — used for `Shot` / `ScreenplayScene`.
    pub scene: Option<String>,
    /// Free-form tags forwarded onto the clip-ref.
    pub tags: Vec<String>,
}

/// Paths returned from `store_with_clip` so the caller can log or follow
/// up. Both are absolute.
pub struct StoreWithClipResult {
    /// Absolute path to the manifest JSON.
    pub manifest_path: PathBuf,
    /// Absolute path to the emitted `.clip.html`.
    pub clipref_path: PathBuf,
}

impl AssetCache {
    /// Store the manifest, then emit a sibling `.clip.html` clip-ref under
    /// `<workdir>/refs/<kind>/`. Idempotent on `asset_hash`.
    pub fn store_with_clip(
        &self,
        manifest: &Manifest,
        ctx: ClipEmitContext<'_>,
    ) -> Result<StoreWithClipResult, BackendError> {
        let manifest_path = self.store(manifest)?;

        let slug = slug_for(
            ctx.kind,
            ctx.scene.as_deref(),
            &ctx.prompt,
            &manifest.request_hash,
        );
        let clipref_path = default_path(ctx.workdir, ctx.kind, &slug);

        if let Some(parent) = clipref_path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| {
                BackendError::Cache(format!("mkdir {}: {e}", parent.display()))
            })?;
        }

        if clipref_path.exists() {
            let raw = std::fs::read_to_string(&clipref_path).map_err(|e| {
                BackendError::Cache(format!("read {}: {e}", clipref_path.display()))
            })?;
            let (existing, _body) = ClipRef::parse(&raw).map_err(|e| {
                BackendError::Cache(format!("parse {}: {e}", clipref_path.display()))
            })?;
            if existing.asset_hash == manifest.request_hash {
                return Ok(StoreWithClipResult {
                    manifest_path,
                    clipref_path,
                });
            }
            return Err(BackendError::Cache(format!(
                "clipref collision: {} already exists with asset_hash {} (incoming {})",
                clipref_path.display(),
                existing.asset_hash,
                manifest.request_hash,
            )));
        }

        let asset_rel = compute_asset_rel(self, manifest, &clipref_path)?;
        let asset_filename = asset_rel
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "asset".to_string());

        let parent_ulid = match ctx.parent.as_deref() {
            Some(s) => Some(
                Ulid::from_string(s)
                    .map_err(|e| BackendError::Cache(format!("invalid parent ulid {s}: {e}")))?,
            ),
            None => None,
        };

        let cost_usd = if manifest.cost_estimate_usd > 0.0 {
            Some(manifest.cost_estimate_usd)
        } else {
            None
        };

        let clip_ref = ClipRef {
            clip: Ulid::new(),
            kind: ctx.kind,
            asset: asset_rel.clone(),
            asset_hash: manifest.request_hash.clone(),
            provider: manifest.provider.clone(),
            prompt: ctx.prompt,
            created_at: Utc::now(),
            model: ctx.model,
            cost_usd,
            parent: parent_ulid,
            edit_kind: ctx.edit_kind,
            edit_prompt: ctx.edit_prompt,
            tags: ctx.tags,
            scene: ctx.scene,
            extra: BTreeMap::new(),
        };

        let body = render_body(ctx.kind, &asset_rel, &asset_filename);

        clip_ref.write(&clipref_path, &body).map_err(|e| {
            BackendError::Cache(format!("write {}: {e}", clipref_path.display()))
        })?;

        Ok(StoreWithClipResult {
            manifest_path,
            clipref_path,
        })
    }
}

/// Compute the path the clip-ref's `asset:` field should carry. Resolves
/// the manifest's cache-root-relative blob path to an absolute path, then
/// makes it relative to the clip-ref's parent directory. Falls back to a
/// "missing blob" placeholder when the manifest has no `asset_path`
/// (e.g. JSON-only providers like stock search).
fn compute_asset_rel(
    cache: &AssetCache,
    manifest: &Manifest,
    clipref_path: &Path,
) -> Result<PathBuf, BackendError> {
    let Some(rel_to_root) = manifest.asset_path.as_deref() else {
        return Ok(PathBuf::from(format!(
            "../../{}/{}.manifest.json",
            manifest.provider, manifest.request_hash
        )));
    };
    let absolute = cache.root().join(rel_to_root);
    let clipref_parent = clipref_path
        .parent()
        .ok_or_else(|| BackendError::Cache("clipref path has no parent".into()))?;
    Ok(relative_from(&absolute, clipref_parent))
}

/// Compute `target` expressed relative to `base`. Both inputs are
/// normalized (no `.` or `..` components inside) but they may not share a
/// common ancestor — in that case we emit `..` segments to walk up out of
/// `base` and then descend into `target`. Mirrors `pathdiff::diff_paths`
/// for the cases we hit (cache + workdir on the same filesystem).
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

/// Resolve `.` and `..` components without touching the filesystem. We
/// only call this on absolute paths inside the same root, so there are
/// no surprising symlink semantics to worry about.
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

/// Minimal HTML preview body. The compose pre-pass (wb-n33n.4) replaces
/// `<wavelet-clip>` elements with these — they're not the final render.
fn render_body(kind: ClipKind, asset: &Path, filename: &str) -> String {
    let src = asset.to_string_lossy();
    let snippet = match kind {
        ClipKind::Shot | ClipKind::SceneStill => {
            format!("<video controls src=\"{src}\"></video>")
        }
        ClipKind::Still => format!("<img src=\"{src}\" />"),
        ClipKind::Music | ClipKind::Tts => {
            format!("<audio controls src=\"{src}\"></audio>")
        }
        _ => format!("<a href=\"{src}\">{filename}</a>"),
    };
    format!("\n{snippet}\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backends::cache::utc_now_iso8601;
    use serde_json::json;

    fn mk_manifest(hash: &str, with_blob: bool) -> Manifest {
        Manifest {
            version: 1,
            provider: "google".into(),
            cluster: "img2vid_gen".into(),
            request_hash: hash.into(),
            request: json!({"prompt": "hand pouring water"}),
            response: json!({"job_id": "abc"}),
            cost_estimate_usd: 0.20,
            asset_path: if with_blob {
                Some(format!("google/{hash}.mp4"))
            } else {
                None
            },
            created_at: utc_now_iso8601(),
        }
    }

    fn mk_ctx(workdir: &Path) -> ClipEmitContext<'_> {
        ClipEmitContext {
            workdir,
            kind: ClipKind::Shot,
            prompt: "hand pouring water in a slow steady stream".into(),
            model: Some("veo-3.1-generate-preview".into()),
            parent: None,
            edit_kind: None,
            edit_prompt: None,
            scene: Some("INT. KITCHEN - DAY".into()),
            tags: vec!["hero".into()],
        }
    }

    #[test]
    fn first_write_emits_manifest_and_clipref_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let workdir = dir.path().join("work");
        let cache_root = dir.path().join("cache");
        std::fs::create_dir_all(&workdir).unwrap();
        std::fs::create_dir_all(&cache_root).unwrap();
        // pre-create the blob so paths resolve sensibly
        std::fs::create_dir_all(cache_root.join("google")).unwrap();
        std::fs::write(cache_root.join("google/hash01.mp4"), b"bytes").unwrap();

        let cache = AssetCache::new(&cache_root);
        let manifest = mk_manifest("hash01", true);
        let res = cache
            .store_with_clip(&manifest, mk_ctx(&workdir))
            .unwrap();

        assert!(res.manifest_path.exists(), "manifest must exist");
        assert!(res.clipref_path.exists(), "clipref must exist");

        let raw = std::fs::read_to_string(&res.clipref_path).unwrap();
        let (parsed, body) = ClipRef::parse(&raw).unwrap();
        assert_eq!(parsed.kind, ClipKind::Shot);
        assert_eq!(parsed.asset_hash, "hash01");
        assert_eq!(parsed.provider, "google");
        assert_eq!(parsed.model.as_deref(), Some("veo-3.1-generate-preview"));
        assert_eq!(parsed.cost_usd, Some(0.20));
        assert_eq!(parsed.scene.as_deref(), Some("INT. KITCHEN - DAY"));
        assert!(body.contains("<video"), "shot body uses <video>: {body}");
    }

    #[test]
    fn idempotent_rewrite_preserves_ulid() {
        let dir = tempfile::tempdir().unwrap();
        let workdir = dir.path().join("work");
        let cache_root = dir.path().join("cache");
        std::fs::create_dir_all(&workdir).unwrap();
        std::fs::create_dir_all(cache_root.join("google")).unwrap();
        std::fs::write(cache_root.join("google/hash02.mp4"), b"bytes").unwrap();

        let cache = AssetCache::new(&cache_root);
        let manifest = mk_manifest("hash02", true);

        let first = cache.store_with_clip(&manifest, mk_ctx(&workdir)).unwrap();
        let raw_before = std::fs::read_to_string(&first.clipref_path).unwrap();
        let (parsed_before, _) = ClipRef::parse(&raw_before).unwrap();
        let ulid_before = parsed_before.clip;

        let second = cache.store_with_clip(&manifest, mk_ctx(&workdir)).unwrap();
        assert_eq!(
            first.clipref_path, second.clipref_path,
            "second call returns the same path"
        );
        let raw_after = std::fs::read_to_string(&second.clipref_path).unwrap();
        let (parsed_after, _) = ClipRef::parse(&raw_after).unwrap();
        assert_eq!(
            ulid_before, parsed_after.clip,
            "ULID must be preserved across idempotent re-write"
        );
    }

    #[test]
    fn parent_pointer_and_edit_kind_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let workdir = dir.path().join("work");
        let cache_root = dir.path().join("cache");
        std::fs::create_dir_all(&workdir).unwrap();
        std::fs::create_dir_all(cache_root.join("google")).unwrap();
        std::fs::write(cache_root.join("google/hash03.mp4"), b"bytes").unwrap();

        let cache = AssetCache::new(&cache_root);
        let manifest = mk_manifest("hash03", true);

        let parent_ulid = Ulid::new();
        let mut ctx = mk_ctx(&workdir);
        ctx.parent = Some(parent_ulid.to_string());
        ctx.edit_kind = Some(EditKind::RefineFace);
        ctx.edit_prompt = Some("sharpen face details".into());

        let res = cache.store_with_clip(&manifest, ctx).unwrap();
        let raw = std::fs::read_to_string(&res.clipref_path).unwrap();
        let (parsed, _) = ClipRef::parse(&raw).unwrap();
        assert_eq!(parsed.parent, Some(parent_ulid));
        assert_eq!(parsed.edit_kind, Some(EditKind::RefineFace));
        assert_eq!(parsed.edit_prompt.as_deref(), Some("sharpen face details"));
    }

    #[test]
    fn collision_with_different_hash_errors() {
        let dir = tempfile::tempdir().unwrap();
        let workdir = dir.path().join("work");
        let cache_root = dir.path().join("cache");
        std::fs::create_dir_all(&workdir).unwrap();
        std::fs::create_dir_all(cache_root.join("google")).unwrap();
        std::fs::write(cache_root.join("google/hash04.mp4"), b"bytes").unwrap();

        let cache = AssetCache::new(&cache_root);

        // First emission with hash04.
        let m1 = mk_manifest("hash04", true);
        cache.store_with_clip(&m1, mk_ctx(&workdir)).unwrap();

        // Overwrite the on-disk clipref's asset_hash to simulate collision.
        let slug = slug_for(
            ClipKind::Shot,
            Some("INT. KITCHEN - DAY"),
            "hand pouring water in a slow steady stream",
            "hash04",
        );
        let path = default_path(&workdir, ClipKind::Shot, &slug);
        let raw = std::fs::read_to_string(&path).unwrap();
        let tampered = raw.replace("asset-hash: hash04", "asset-hash: someother");
        std::fs::write(&path, tampered).unwrap();

        // Second emission with the original manifest now collides.
        let err = cache
            .store_with_clip(&m1, mk_ctx(&workdir))
            .err()
            .expect("collision must error");
        match err {
            BackendError::Cache(msg) => assert!(msg.contains("collision"), "msg: {msg}"),
            other => panic!("expected Cache error, got {other:?}"),
        }
    }

    #[test]
    fn relative_from_walks_up_and_down() {
        let r = relative_from(
            Path::new("/a/b/cache/google/x.mp4"),
            Path::new("/a/b/work/refs/shot"),
        );
        assert_eq!(r, PathBuf::from("../../../cache/google/x.mp4"));
    }
}
