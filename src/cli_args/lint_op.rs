//! LintOp — clap args for `wavelet lint`. The whole verb is flat;
//! the design doc treats lint as one entry point with a `--rules`
//! filter rather than per-rule subcommands.

use clap::Args;
use std::path::PathBuf;

#[derive(Args, Debug)]
/// Arguments for `wavelet lint`.
pub struct LintOp {
    /// HTML file or directory to lint. Accepts:
    /// - a single scene `.html` file
    /// - a directory of `scene-*.html` / `*.html` files
    /// - a `commercial.html` containing `<section data-scene-href="...">` refs
    pub path: PathBuf,

    /// Target platform — selects the safe-zone table. When unset, the
    /// safe-zone rule short-circuits to PASS for every scene.
    #[arg(long)]
    pub platform: Option<String>,

    /// Aspect override (`9:16` | `16:9` | `1:1` | `4:5`). When unset,
    /// inferred from the HTML viewport / `<meta name=resolution>`.
    #[arg(long)]
    pub aspect: Option<String>,

    /// Comma-list of rules to run. Available: `safe-zone`,
    /// `glyph-clip`. Default runs all rules.
    #[arg(long, value_delimiter = ',', default_value = "safe-zone,glyph-clip")]
    pub rules: Vec<String>,

    /// Output format. `text` is the default tee-friendly form;
    /// `json` emits a structured `LintReport`.
    #[arg(long, default_value = "text")]
    pub format: String,

    /// Frame timestamp to lint at, in seconds. Default: midpoint of
    /// each scene's duration (or t=1.0 when duration is unknown).
    #[arg(long)]
    pub at: Option<f32>,
}
