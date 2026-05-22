//! Walk `<workdir>/refs/**/*.clip.html` and parse each ref into memory.
//! Used by the `wavelet clip` inspection commands (wb-n33n.6) and the
//! `import` backfill (wb-n33n.7).

use std::path::{Path, PathBuf};

use super::{ClipRef, ClipRefError};

/// One walked clip-ref, paired with the on-disk path it came from.
#[derive(Debug, Clone)]
pub struct WalkedRef {
    /// Absolute path to the `.clip.html`.
    pub path: PathBuf,
    /// Parsed clip-ref. Body discarded — `wavelet clip` doesn't need it.
    pub clip: ClipRef,
}

/// Walk every `.clip.html` under `<workdir>/refs/` (recursively).
/// Returns refs sorted by `created_at` for stable ordering.
pub fn walk_refs(workdir: &Path) -> Result<Vec<WalkedRef>, ClipRefError> {
    let root = workdir.join("refs");
    let mut out = Vec::new();
    if !root.exists() {
        return Ok(out);
    }
    walk_dir(&root, &mut out)?;
    out.sort_by(|a, b| a.clip.created_at.cmp(&b.clip.created_at));
    Ok(out)
}

fn walk_dir(dir: &Path, out: &mut Vec<WalkedRef>) -> Result<(), ClipRefError> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        let ftype = entry.file_type()?;
        if ftype.is_dir() {
            walk_dir(&path, out)?;
            continue;
        }
        if !path.to_string_lossy().ends_with(".clip.html") {
            continue;
        }
        let raw = std::fs::read_to_string(&path)?;
        let (clip, _body) = match ClipRef::parse(&raw) {
            Ok(parsed) => parsed,
            Err(_) => continue,
        };
        out.push(WalkedRef { path, clip });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clipref::{default_path, slug_for, ClipKind, ClipRef};
    use chrono::Utc;
    use std::collections::BTreeMap;
    use ulid::Ulid;

    fn write_one(workdir: &Path, kind: ClipKind, prompt: &str) {
        let slug = slug_for(kind, None, prompt, "deadbe");
        let path = default_path(workdir, kind, &slug);
        let clip = ClipRef {
            clip: Ulid::new(),
            kind,
            asset: PathBuf::from("x"),
            asset_hash: "deadbe".into(),
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
        clip.write(&path, "\nbody\n").unwrap();
    }

    #[test]
    fn walks_nested_dirs() {
        let dir = tempfile::tempdir().unwrap();
        write_one(dir.path(), ClipKind::Shot, "a");
        write_one(dir.path(), ClipKind::Still, "b");
        write_one(dir.path(), ClipKind::Music, "c");

        let walked = walk_refs(dir.path()).unwrap();
        assert_eq!(walked.len(), 3);
    }

    #[test]
    fn empty_workdir_is_fine() {
        let dir = tempfile::tempdir().unwrap();
        let walked = walk_refs(dir.path()).unwrap();
        assert!(walked.is_empty());
    }
}
