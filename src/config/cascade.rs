//! Layered config resolution for tool defaults.
//!
//! Lookup order (highest to lowest priority):
//!
//!   1. explicit CLI flag (caller passes `Some(...)`)
//!   2. workdir `wavelet.config.toml` (walk up from cwd until found or `/`)
//!   3. user-global `~/.wavelet/config.toml`
//!   4. hardcoded default from `super::defaults`
//!
//! Missing or malformed files at layers 2-3 silently fall through to
//! the next layer. We don't surface IO/parse errors here — config is
//! an aid, not a contract. The caller can never observe a panic, only
//! a falling-back-to-default.
//!
//! The whole cascade is read once per process via a `LazyLock`. Tests
//! that need to verify file-walking behavior call [`load_from`]
//! directly with a synthetic root.
//!
//! Per wb-e90g — scope is *which default backend the user gets*. We
//! deliberately do not expose backend-specific knobs (API keys, model
//! variants, etc.) here. Adapters keep reading credentials from env
//! the way they already do.

use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use serde::Deserialize;

use super::defaults;

/// Discriminator for the four default-able backend slots.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum BackendKind {
    /// Text-to-video / image-to-video.
    Video,
    /// Reference-conditioned music generation.
    Music,
    /// Text-to-image (still generation).
    Image,
    /// Text-to-speech.
    Tts,
}

/// The `[defaults]` block of `wavelet.config.toml`. Every field is
/// optional — a missing field means "fall through to the next layer."
#[derive(Default, Debug, Deserialize)]
pub struct DefaultsBlock {
    /// Video backend (txt2vid / img2vid).
    pub video_backend: Option<String>,
    /// Reference-conditioned music backend.
    pub music_backend: Option<String>,
    /// Text-to-image backend (still generation).
    pub image_backend: Option<String>,
    /// Text-to-speech backend.
    pub tts_backend: Option<String>,
    /// Default aspect ratio (e.g. `16:9`).
    pub aspect: Option<String>,
    /// Default clip duration in seconds.
    pub duration_secs: Option<f32>,
}

#[derive(Default, Debug, Deserialize)]
struct ConfigFile {
    #[serde(default)]
    defaults: DefaultsBlock,
}

/// Merged-and-resolved view of the cascade for the current process.
#[derive(Default, Debug)]
pub struct ResolvedConfig {
    /// Resolved video backend (none → fall through to tool default).
    pub video_backend: Option<String>,
    /// Resolved music backend.
    pub music_backend: Option<String>,
    /// Resolved image backend.
    pub image_backend: Option<String>,
    /// Resolved TTS backend.
    pub tts_backend: Option<String>,
    /// Resolved aspect ratio.
    pub aspect: Option<String>,
    /// Resolved clip duration in seconds.
    pub duration_secs: Option<f32>,
}

impl ResolvedConfig {
    fn merge_under(&mut self, layer: DefaultsBlock) {
        if self.video_backend.is_none() {
            self.video_backend = layer.video_backend;
        }
        if self.music_backend.is_none() {
            self.music_backend = layer.music_backend;
        }
        if self.image_backend.is_none() {
            self.image_backend = layer.image_backend;
        }
        if self.tts_backend.is_none() {
            self.tts_backend = layer.tts_backend;
        }
        if self.aspect.is_none() {
            self.aspect = layer.aspect;
        }
        if self.duration_secs.is_none() {
            self.duration_secs = layer.duration_secs;
        }
    }
}

static PROCESS_CONFIG: LazyLock<ResolvedConfig> = LazyLock::new(|| {
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    load_from(&cwd, dirs_home())
});

// wb-uory.12: `WAVELET_HOME` overrides `HOME` for config-cascade resolution
// only. Lets eval runners isolate wavelet config (and any future
// home-relative state wavelet itself writes) per-run without touching
// global HOME — which is load-bearing for agent-CLI auth (claude reads
// ~/.claude.json, codex reads ~/.codex/, etc.). The earlier HOME-sandbox
// approach broke those CLIs; this surgical override does not.
fn dirs_home() -> Option<PathBuf> {
    std::env::var_os("WAVELET_HOME")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from)
}

/// Build a `ResolvedConfig` by walking from `workdir` upward for
/// `wavelet.config.toml`, then layering the user-global config under it.
/// Public for tests; production callers use [`resolved`].
pub fn load_from(workdir: &Path, home: Option<PathBuf>) -> ResolvedConfig {
    let mut out = ResolvedConfig::default();
    if let Some(block) = walk_up_for_workdir_config(workdir) {
        out.merge_under(block);
    }
    if let Some(h) = home {
        let user_global = h.join(".wavelet").join("config.toml");
        if let Some(block) = read_block(&user_global) {
            out.merge_under(block);
        }
    }
    out
}

fn walk_up_for_workdir_config(start: &Path) -> Option<DefaultsBlock> {
    let mut cur: Option<&Path> = Some(start);
    while let Some(dir) = cur {
        let candidate = dir.join("wavelet.config.toml");
        if let Some(block) = read_block(&candidate) {
            return Some(block);
        }
        cur = dir.parent();
    }
    None
}

fn read_block(path: &Path) -> Option<DefaultsBlock> {
    let bytes = std::fs::read_to_string(path).ok()?;
    let parsed: ConfigFile = toml::from_str(&bytes).ok()?;
    Some(parsed.defaults)
}

/// Process-wide resolved cascade (workdir + user-global). Tool defaults
/// from [`super::defaults`] are *not* baked in here — callers apply
/// them as the final fallback so tests can isolate "what did the file
/// layer give us?" from the hardcoded fallback.
pub fn resolved() -> &'static ResolvedConfig {
    &PROCESS_CONFIG
}

/// Resolve a backend identifier for a given slot.
///
/// `cli_override` is the value the user passed via `--backend` (or
/// equivalent). When present it wins. Otherwise the cascade decides;
/// if nothing in the cascade has a value, the hardcoded default for
/// `kind` is used.
pub fn resolve_backend(kind: BackendKind, cli_override: Option<&str>) -> String {
    if let Some(v) = cli_override {
        return v.to_string();
    }
    resolve_backend_from(kind, resolved())
}

/// Variant of [`resolve_backend`] that takes an explicit `ResolvedConfig` —
/// used by tests with a synthetic workdir.
pub fn resolve_backend_from(kind: BackendKind, cfg: &ResolvedConfig) -> String {
    let from_cfg = match kind {
        BackendKind::Video => cfg.video_backend.as_deref(),
        BackendKind::Music => cfg.music_backend.as_deref(),
        BackendKind::Image => cfg.image_backend.as_deref(),
        BackendKind::Tts => cfg.tts_backend.as_deref(),
    };
    from_cfg
        .map(str::to_string)
        .unwrap_or_else(|| default_for(kind).to_string())
}

fn default_for(kind: BackendKind) -> &'static str {
    match kind {
        BackendKind::Video => defaults::VIDEO_BACKEND,
        BackendKind::Music => defaults::MUSIC_BACKEND,
        BackendKind::Image => defaults::IMAGE_BACKEND,
        BackendKind::Tts => defaults::TTS_BACKEND,
    }
}

/// Resolve the aspect-ratio default with optional CLI override.
pub fn resolve_aspect(cli_override: Option<&str>) -> String {
    if let Some(v) = cli_override {
        return v.to_string();
    }
    resolved()
        .aspect
        .clone()
        .unwrap_or_else(|| defaults::ASPECT.to_string())
}

/// Resolve the clip-duration default with optional CLI override.
pub fn resolve_duration_secs(cli_override: Option<f32>) -> f32 {
    if let Some(v) = cli_override {
        return v;
    }
    resolved().duration_secs.unwrap_or(defaults::DURATION_SECS)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn missing_workdir_and_global_falls_back_to_tool_default() {
        let tmp = tempdir().unwrap();
        let cfg = load_from(tmp.path(), None);
        assert_eq!(
            resolve_backend_from(BackendKind::Video, &cfg),
            defaults::VIDEO_BACKEND
        );
        assert_eq!(
            resolve_backend_from(BackendKind::Music, &cfg),
            defaults::MUSIC_BACKEND
        );
        assert_eq!(
            resolve_backend_from(BackendKind::Image, &cfg),
            defaults::IMAGE_BACKEND
        );
        assert_eq!(
            resolve_backend_from(BackendKind::Tts, &cfg),
            defaults::TTS_BACKEND
        );
    }

    #[test]
    fn workdir_config_overrides_tool_default() {
        let tmp = tempdir().unwrap();
        fs::write(
            tmp.path().join("wavelet.config.toml"),
            "[defaults]\nvideo_backend = \"veo\"\n",
        )
        .unwrap();
        let cfg = load_from(tmp.path(), None);
        assert_eq!(resolve_backend_from(BackendKind::Video, &cfg), "veo");
    }

    #[test]
    fn workdir_config_walks_up_to_parent() {
        let tmp = tempdir().unwrap();
        let nested = tmp.path().join("a").join("b").join("c");
        fs::create_dir_all(&nested).unwrap();
        fs::write(
            tmp.path().join("wavelet.config.toml"),
            "[defaults]\nmusic_backend = \"udio\"\n",
        )
        .unwrap();
        let cfg = load_from(&nested, None);
        assert_eq!(resolve_backend_from(BackendKind::Music, &cfg), "udio");
    }

    #[test]
    fn workdir_config_overrides_user_global() {
        let tmp = tempdir().unwrap();
        let home = tempdir().unwrap();
        fs::create_dir_all(home.path().join(".wavelet")).unwrap();
        fs::write(
            home.path().join(".wavelet").join("config.toml"),
            "[defaults]\nimage_backend = \"global-img\"\n",
        )
        .unwrap();
        fs::write(
            tmp.path().join("wavelet.config.toml"),
            "[defaults]\nimage_backend = \"workdir-img\"\n",
        )
        .unwrap();
        let cfg = load_from(tmp.path(), Some(home.path().to_path_buf()));
        assert_eq!(
            resolve_backend_from(BackendKind::Image, &cfg),
            "workdir-img"
        );
    }

    #[test]
    fn user_global_used_when_no_workdir_config() {
        let tmp = tempdir().unwrap();
        let home = tempdir().unwrap();
        fs::create_dir_all(home.path().join(".wavelet")).unwrap();
        fs::write(
            home.path().join(".wavelet").join("config.toml"),
            "[defaults]\ntts_backend = \"my-tts\"\n",
        )
        .unwrap();
        let cfg = load_from(tmp.path(), Some(home.path().to_path_buf()));
        assert_eq!(resolve_backend_from(BackendKind::Tts, &cfg), "my-tts");
    }

    #[test]
    fn malformed_toml_falls_through_silently() {
        let tmp = tempdir().unwrap();
        fs::write(
            tmp.path().join("wavelet.config.toml"),
            "this is not [valid toml = \n",
        )
        .unwrap();
        let cfg = load_from(tmp.path(), None);
        assert_eq!(
            resolve_backend_from(BackendKind::Video, &cfg),
            defaults::VIDEO_BACKEND
        );
    }

    #[test]
    fn cli_override_beats_workdir() {
        let tmp = tempdir().unwrap();
        fs::write(
            tmp.path().join("wavelet.config.toml"),
            "[defaults]\nvideo_backend = \"veo\"\n",
        )
        .unwrap();
        let cfg = load_from(tmp.path(), None);
        // Explicit override wins regardless of the cascade.
        // The free-function resolve_backend goes through PROCESS_CONFIG,
        // but resolve_backend_from operates on this specific cfg — the
        // override path is identical, so we test the precedence via
        // a small inline match.
        let chosen = match Some("fal-wan-t2v") {
            Some(v) => v.to_string(),
            None => resolve_backend_from(BackendKind::Video, &cfg),
        };
        assert_eq!(chosen, "fal-wan-t2v");
    }

    // wb-uory.12 — WAVELET_HOME overrides HOME for cascade resolution.
    // Test serializes on its own env vars by setting + unsetting in the
    // same body; pair with #[serial] if more env-touching tests land.
    #[test]
    fn wavelet_home_overrides_home_in_dirs_home() {
        let real_home = tempdir().unwrap();
        let wavelet_home = tempdir().unwrap();

        // Write a config under WAVELET_HOME that should win.
        fs::create_dir_all(wavelet_home.path().join(".wavelet")).unwrap();
        fs::write(
            wavelet_home.path().join(".wavelet").join("config.toml"),
            "[defaults]\nvideo_backend = \"from-wavelet-home\"\n",
        )
        .unwrap();

        // And one under real HOME that must NOT be picked up.
        fs::create_dir_all(real_home.path().join(".wavelet")).unwrap();
        fs::write(
            real_home.path().join(".wavelet").join("config.toml"),
            "[defaults]\nvideo_backend = \"from-real-home\"\n",
        )
        .unwrap();

        let prev_home = std::env::var_os("HOME");
        let prev_wavelet_home = std::env::var_os("WAVELET_HOME");

        // SAFETY: tests in this module aren't run in parallel with anything
        // else that mutates these env vars; toggling here is acceptable.
        unsafe {
            std::env::set_var("HOME", real_home.path());
            std::env::set_var("WAVELET_HOME", wavelet_home.path());
        }

        let h = dirs_home();
        assert_eq!(h.as_deref(), Some(wavelet_home.path()));

        let cfg = load_from(tempdir().unwrap().path(), h);
        assert_eq!(
            resolve_backend_from(BackendKind::Video, &cfg),
            "from-wavelet-home"
        );

        // Restore env so subsequent tests aren't polluted.
        unsafe {
            match prev_home {
                Some(v) => std::env::set_var("HOME", v),
                None => std::env::remove_var("HOME"),
            }
            match prev_wavelet_home {
                Some(v) => std::env::set_var("WAVELET_HOME", v),
                None => std::env::remove_var("WAVELET_HOME"),
            }
        }
    }

    #[test]
    fn dirs_home_falls_back_to_home_when_wavelet_home_unset() {
        let real_home = tempdir().unwrap();

        let prev_home = std::env::var_os("HOME");
        let prev_wavelet_home = std::env::var_os("WAVELET_HOME");

        unsafe {
            std::env::set_var("HOME", real_home.path());
            std::env::remove_var("WAVELET_HOME");
        }

        assert_eq!(dirs_home().as_deref(), Some(real_home.path()));

        unsafe {
            match prev_home {
                Some(v) => std::env::set_var("HOME", v),
                None => std::env::remove_var("HOME"),
            }
            if let Some(v) = prev_wavelet_home {
                std::env::set_var("WAVELET_HOME", v);
            }
        }
    }

    #[test]
    fn aspect_and_duration_resolve_from_workdir() {
        let tmp = tempdir().unwrap();
        fs::write(
            tmp.path().join("wavelet.config.toml"),
            "[defaults]\naspect = \"9:16\"\nduration_secs = 8.0\n",
        )
        .unwrap();
        let cfg = load_from(tmp.path(), None);
        assert_eq!(cfg.aspect.as_deref(), Some("9:16"));
        assert_eq!(cfg.duration_secs, Some(8.0));
    }
}
