//! Slug + default path helpers. Pure functions, no I/O.

use std::path::{Path, PathBuf};

use super::types::ClipKind;

/// Build a stable slug:
/// `<kind>-<scene-or-none>-<sluggified-prompt-prefix>-<6char-asset-hash>`.
///
/// Same inputs → same slug. `scene` becomes `none` when absent. The prompt
/// prefix is the first 32 characters of the sluggified prompt; if the
/// prompt is empty the segment becomes `untitled`.
pub fn slug_for(
    kind: ClipKind,
    scene: Option<&str>,
    prompt: &str,
    asset_hash: &str,
) -> String {
    let scene_seg = scene
        .map(sluggify)
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "none".to_string());
    let prompt_seg = {
        let s = sluggify(prompt);
        if s.is_empty() {
            "untitled".to_string()
        } else {
            truncate_chars(&s, 32).to_string()
        }
    };
    let hash_seg = truncate_chars(asset_hash, 6);
    format!("{kind}-{scene_seg}-{prompt_seg}-{hash_seg}", kind = kind.as_kebab())
}

/// `<workdir>/refs/<kind>/<slug>.clip.html`.
pub fn default_path(workdir: &Path, kind: ClipKind, slug: &str) -> PathBuf {
    workdir
        .join("refs")
        .join(kind.as_kebab())
        .join(format!("{slug}.clip.html"))
}

/// ASCII slug: lowercase, alphanumerics and `-` only, collapse runs of
/// non-alphanumerics into a single `-`, strip leading/trailing `-`.
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
    out
}

/// Truncate to at most `n` characters (not bytes). UTF-8-safe.
fn truncate_chars(s: &str, n: usize) -> &str {
    match s.char_indices().nth(n) {
        Some((idx, _)) => &s[..idx],
        None => s,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sluggify_basic() {
        assert_eq!(sluggify("Hello, World!"), "hello-world");
        assert_eq!(sluggify("INT. KITCHEN - DAY"), "int-kitchen-day");
        assert_eq!(sluggify("  multi   space  "), "multi-space");
        assert_eq!(sluggify(""), "");
    }

    #[test]
    fn truncate_utf8_safe() {
        assert_eq!(truncate_chars("abcdef", 3), "abc");
        assert_eq!(truncate_chars("abc", 10), "abc");
    }
}
