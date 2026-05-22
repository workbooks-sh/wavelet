//! DirectorOp — extracted from the wavelet CLI clap definitions.

use std::path::PathBuf;
use clap::{Parser, Subcommand, ValueEnum, Args};



#[derive(Subcommand)]
pub enum DirectorOp {
    /// Synthesize L-Storyboard `attributes` for every shot in a
    /// storyboard. Reads a brief (Markdown/plain text) and a
    /// storyboard JSON; sends one LLM call to `fal-ai/any-llm`
    /// (Gemini 2.5 Pro by default); merges returned attributes onto
    /// each shot; writes the populated storyboard.
    Synthesize {
        /// Path to the brief — Markdown, plain text, anything.
        brief: PathBuf,
        /// Path to the input storyboard JSON. Existing `attributes`
        /// blocks are overwritten.
        storyboard: PathBuf,
        /// Output path for the populated storyboard JSON.
        #[arg(short, long)]
        out: PathBuf,
        /// LLM choice: `gemini` (default, fal-ai/any-llm →
        /// google/gemini-2.5-pro) or `claude` (anthropic/claude-opus-4-7).
        #[arg(long, default_value = "gemini")]
        model: String,
        /// Optional style override applied to every shot (e.g.
        /// `"A24-flavored, 35mm grain, dusk palette"`).
        #[arg(long)]
        style_anchor: Option<String>,
        /// Pretty-print the emitted JSON.
        #[arg(long)]
        pretty: bool,
        /// Cache root for the underlying FAL client.
        #[arg(long, default_value = ".wavelet-cache")]
        cache: PathBuf,
    },
    /// Rubric-graded prompt mutation (wb-8ync). Given a failed shot's
    /// original prompt + the VLM verify findings, ask the LLM to
    /// rewrite the prompt so each FAIL/WARN is addressed. Caller is
    /// expected to cap `previous_mutations` at ~3 before escalating
    /// to surgical edit / human checkpoint.
    Grade {
        /// Path to the brief.
        brief: PathBuf,
        /// The original prompt that produced the failed gen.
        #[arg(long)]
        prompt: String,
        /// Path to the VisionVerifyResult JSON emitted by
        /// `wavelet image verify-shot` (or equivalent).
        #[arg(long)]
        findings: PathBuf,
        /// Earlier mutations the grader has already produced for this
        /// shot. Each --previous flag adds one entry. Default: none.
        #[arg(long = "previous", value_name = "PROMPT")]
        previous: Vec<String>,
        /// LLM choice — same routing as `director synthesize`.
        #[arg(long, default_value = "gemini")]
        model: String,
        /// Output path for the GraderResult JSON.
        #[arg(short, long)]
        out: PathBuf,
        /// Pretty-print the emitted JSON.
        #[arg(long)]
        pretty: bool,
        /// Cache root for the underlying FAL client.
        #[arg(long, default_value = ".wavelet-cache")]
        cache: PathBuf,
    },
}
