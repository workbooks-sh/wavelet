//! Fountain screenplay parser — fountain.io spec v1.1, MIT-equivalent licensing.
//!
//! Phase 0 of the screenplay-to-MP4 epic (wb-iv3c → wb-t4jq). The input
//! is a plain-text `.fountain` file; the output is a `Screenplay` AST that
//! downstream gamut tools (`gamut velocity propose`, `gamut storyboard
//! plan`, etc.) consume.
//!
//! ## Spec coverage (v0)
//!
//! Implemented:
//! - Title page (key:value pairs at the top, ends with a blank line)
//! - Scene headings (INT./EXT./EST./INT./EXT./I/E + forced `.` prefix)
//! - Action paragraphs (+ forced `!` prefix, + centered `> text <`)
//! - Character cues + dialogue + parentheticals (+ forced `@` prefix,
//!   dual-dialogue via trailing `^`)
//! - Transitions (lines ending in `TO:` or starting with `>`, + FADE
//!   IN: / FADE OUT.)
//! - Page breaks (`===`)
//! - Sections (`# Section`, `## Subsection`, etc.) — preserved as
//!   structural anchors
//! - Synopses (`= synopsis text`)
//! - Lyrics (`~ lyric line`)
//! - Notes (`[[ note ]]`) and boneyard (`/* comment */`) — stripped
//!   from output but preserved on `Element::Action.notes`
//!
//! Deferred:
//! - Inline formatting (`*italic*`, `**bold**`, `***bold italic***`,
//!   `_underline_`) — left as raw text; agents can render the markup
//!   downstream if they care
//! - Scene numbers (`#42#` trailing the slugline)
//! - Title-page custom keys beyond the common set

#![deny(missing_docs)]

pub mod ast;
pub mod characters;
pub mod parser;

pub use ast::{
    DialogueLine, Element, InteriorExterior, Screenplay, TitlePage, Transition, TransitionKind,
};
pub use characters::{canonicalize_name, screenplay_characters, CharacterEntry};
pub use parser::{parse, ParseError};
