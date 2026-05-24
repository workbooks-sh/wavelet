//! CharacterOp — `wavelet character …` subcommand surface (wb-cx08).
//!
//! For now only `define` is wired; `list` and friends are out of scope
//! and tracked separately.

use std::path::PathBuf;

use clap::{Subcommand, ValueEnum};

/// CLI mirror of `clipref::character::CharacterType`. Kept separate so
/// the CLI surface can evolve independently of the on-disk schema.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum CharacterType {
    /// Full-body / face references. Default.
    FullBody,
    /// Hands-only references (ECU / closeup of hand action).
    Hands,
    /// Hands holding a product (ECU + product-hands shot vocab).
    ProductHands,
}

impl From<CharacterType> for crate::clipref::character::CharacterType {
    fn from(c: CharacterType) -> Self {
        match c {
            CharacterType::FullBody => Self::FullBody,
            CharacterType::Hands => Self::Hands,
            CharacterType::ProductHands => Self::ProductHands,
        }
    }
}

/// Subcommands for `wavelet character`.
#[derive(Subcommand)]
pub enum CharacterOp {
    /// Define a character reference bundle — name + 1..N reference
    /// images. Emits a `character-ref` clip-HTML at
    /// `<workdir>/refs/character/<name>.clip.html`. The storyboard
    /// planner auto-discovers these refs and routes matching CHARACTER
    /// cues through `fal-veo3-ref` instead of stock-search.
    Define {
        /// Canonical character name. Will be normalized to the same
        /// keying `fountain::screenplay_characters` uses
        /// (uppercase + extension stripped) so screenplay cues and
        /// character refs share keys.
        name: String,
        /// Reference image — local path or HTTPS URL. Pass `--reference`
        /// multiple times for multiple refs (1..4 is the Fal Veo 3.1
        /// reference adapter's accepted range).
        #[arg(long, value_name = "PATH_OR_URL")]
        reference: Vec<PathBuf>,
        /// Character framing focus. Defaults to `full-body`.
        #[arg(long, value_enum, default_value = "full-body")]
        character_type: CharacterType,
        /// Workdir. Files land at `<workdir>/refs/character/`.
        /// Defaults to the current directory.
        #[arg(long)]
        workdir: Option<PathBuf>,
    },
}
