//! Config cascade for tool defaults — per wb-e90g.
//!
//! Drops the requirement to pass `--backend …` on every wavelet call by
//! resolving defaults from a layered config: CLI flag → workdir
//! `wavelet.config.toml` → user-global `~/.wavelet/config.toml` → tool
//! default in [`defaults`].
//!
//! Scope is *which* backend the user gets, not credentials or
//! per-backend tuning. Adapters still read their own keys from env.

pub mod cascade;
pub mod defaults;

pub use cascade::{
    resolve_aspect, resolve_backend, resolve_backend_from, resolve_duration_secs, resolved,
    BackendKind, ResolvedConfig,
};
