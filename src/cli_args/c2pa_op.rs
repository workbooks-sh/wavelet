//! C2paOp — extracted from the wavelet CLI clap definitions.

use std::path::PathBuf;
use clap::{Parser, Subcommand, ValueEnum, Args};



/// C2PA subcommands. Sign attaches a signed manifest declaring AI-generation
/// provenance + ingredient list; Verify parses a signed MP4 and reports
/// validation state + ingredients + assertions.
#[derive(Subcommand)]
pub enum C2paOp {
    /// Sign an existing MP4 with a C2PA manifest built from a comp.json + the
    /// backend cache. Useful for retroactively signing files that were rendered
    /// before `wavelet render --sign-c2pa` existed.
    Sign {
        /// Input MP4 path.
        input: PathBuf,
        /// Output MP4 path (signed).
        #[arg(short, long)]
        out: PathBuf,
        /// Composition JSON whose scenes + cache become the manifest's
        /// actions + ingredients.
        #[arg(long)]
        comp: PathBuf,
        /// CreativeWork title. Defaults to the composition file stem.
        #[arg(long)]
        title: Option<String>,
        /// CreativeWork author.
        #[arg(long)]
        author: Option<String>,
        /// Backend cache root for ingredient discovery.
        #[arg(long)]
        cache_root: Option<PathBuf>,
        /// Signing-cert chain PEM. Defaults to the bundled dev cert (untrusted
        /// signer; OK for development, fails C2PA trust UIs).
        #[arg(long, requires = "signing_key")]
        signing_cert: Option<PathBuf>,
        /// Private key PEM (paired with --signing-cert).
        #[arg(long, requires = "signing_cert")]
        signing_key: Option<PathBuf>,
    },
    /// Read a signed MP4 and report its C2PA manifest: validation state,
    /// claim generator, ingredient list, assertion labels.
    Verify {
        /// Signed MP4 to inspect.
        input: PathBuf,
        /// Emit the full manifest store JSON instead of the one-line summary.
        #[arg(long)]
        json: bool,
    },
}
