//! Screenplay AST types. Designed to round-trip JSON for downstream
//! gamut tools (`gamut velocity propose`, `gamut storyboard plan`, …).

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Top-level screenplay. `title_page` is optional — a Fountain document
/// may omit it; `elements` is the ordered body.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Screenplay {
    /// Title-page metadata, if present.
    pub title_page: Option<TitlePage>,
    /// Ordered list of body elements: scene headings, action, dialogue,
    /// transitions, sections, synopses, page breaks.
    pub elements: Vec<Element>,
}

/// Title-page metadata. The common Fountain keys are exposed as named
/// fields; anything else lands in `other`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TitlePage {
    /// `Title:` — usually the screenplay's name.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// `Author:` or `Authors:`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    /// `Credit:` — typical content "Written by" / "Screenplay by".
    #[serde(skip_serializing_if = "Option::is_none")]
    pub credit: Option<String>,
    /// `Source:` — adaptation source.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    /// `Draft date:` — free-form date string; we don't parse it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub draft_date: Option<String>,
    /// `Contact:` — contact block, may span lines.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contact: Option<String>,
    /// `Copyright:`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub copyright: Option<String>,
    /// Any other key:value pair from the title page.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub other: BTreeMap<String, String>,
}

/// One body element.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Element {
    /// `INT. KITCHEN - DAY` etc. Parsed into the slugline string plus
    /// best-effort structured fields. Scenes are the primary unit
    /// downstream tools key off.
    SceneHeading {
        /// The full slugline as written (uppercased per convention but
        /// preserved verbatim).
        slugline: String,
        /// `INT.` / `EXT.` / `INT./EXT.` / `EST.` if cleanly extractable.
        #[serde(skip_serializing_if = "Option::is_none")]
        ie: Option<InteriorExterior>,
        /// Best-effort location string (after the IE prefix, before the
        /// `-` time-of-day separator).
        #[serde(skip_serializing_if = "Option::is_none")]
        location: Option<String>,
        /// Best-effort time of day ("DAY", "NIGHT", "MAGIC HOUR", etc.).
        #[serde(skip_serializing_if = "Option::is_none")]
        time_of_day: Option<String>,
    },
    /// Descriptive paragraph. May be centered (`> text <`).
    Action {
        /// The paragraph text, joined across hard-wrapped lines.
        text: String,
        /// True when the source used `> text <` for centering.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        centered: bool,
    },
    /// Character cue + their dialogue (and parentheticals / lyrics
    /// nested under them).
    Dialogue {
        /// The character name as written (typically ALL CAPS).
        character: String,
        /// Cue extension parsed from `CHARACTER (V.O.)` /
        /// `CHARACTER (O.S.)` / `CHARACTER (CONT'D)`. The character
        /// field has the extension stripped; this field carries it.
        #[serde(skip_serializing_if = "Option::is_none")]
        extension: Option<String>,
        /// True iff the cue ended with `^` (dual-dialogue marker). The
        /// dual-dialogue partner is the next `Dialogue` element.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        dual: bool,
        /// True when the source-side cue was `(V.O.)` or `(O.S.)` —
        /// derived from `extension` for downstream tools that just want
        /// "is this voiceover?".
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        is_voiceover: bool,
        /// True when extension is `(O.S.)` (off-screen / off-stage).
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        is_off_screen: bool,
        /// Ordered list of dialogue / parenthetical / lyric lines under
        /// the cue.
        lines: Vec<DialogueLine>,
    },
    /// `CUT TO:` / `DISSOLVE TO:` / `FADE IN:` / `FADE OUT.` — anything
    /// the parser recognizes as a transition cue.
    Transition(Transition),
    /// `===` page break.
    PageBreak,
    /// `# Section`, `## Subsection`, etc. Useful for structural anchors;
    /// not rendered in the output script.
    Section {
        /// 1 for `#`, 2 for `##`, …
        level: u8,
        /// The section text (after the `#`s and one space).
        text: String,
    },
    /// `= synopsis line` — single-line synopsis.
    Synopsis {
        /// Synopsis text (sans the leading `=`).
        text: String,
    },
    /// `~ lyric line` — standalone lyric (when not inside dialogue).
    Lyric {
        /// Lyric text (sans the leading `~`).
        text: String,
    },
}

/// Interior / exterior prefix on a scene heading.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InteriorExterior {
    /// `INT.`
    Int,
    /// `EXT.`
    Ext,
    /// `INT./EXT.` (split scene)
    IntExt,
    /// `EST.` (establishing shot)
    Est,
}

/// One line under a Dialogue cue.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DialogueLine {
    /// Spoken text.
    Text(String),
    /// `(parenthetical)` — performance / direction note.
    Parenthetical(String),
    /// `~ lyric` under the cue.
    Lyric(String),
}

/// Transition cue. The raw `text` field is preserved so consumers can
/// see the source-side spelling (e.g. `MATCH CUT TO:` vs `MATCH CUT:`);
/// `kind` classifies into the shady transition vocabulary.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Transition {
    /// Source-side text (e.g. "CUT TO:", "FADE IN:").
    pub text: String,
    /// Classified kind. Drives downstream shady-transition selection.
    pub kind: TransitionKind,
}

/// Classified transition. Mapped onto shady's transition vocabulary
/// in the screenplay-to-MP4 PRD §5.1.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TransitionKind {
    /// `FADE IN:` — opens from black.
    FadeIn,
    /// `FADE OUT.` / `FADE TO BLACK.` — closes to black.
    FadeOut,
    /// `FADE TO ...` — fade to a non-black target color (rare).
    FadeTo,
    /// `CUT TO:` — hard cut (default for any unspecified transition).
    Cut,
    /// `DISSOLVE TO:` — temporal/dreamlike transition.
    Dissolve,
    /// `MATCH CUT TO:` — visual/audio match between shots.
    MatchCut,
    /// `SMASH CUT TO:` — abrupt, jarring cut.
    SmashCut,
    /// `JUMP CUT TO:` — discontinuous cut within a scene.
    JumpCut,
    /// `WHIP PAN TO:` — fast pan-blur between shots.
    WhipPan,
    /// `J-CUT TO:` — audio of next shot leads visual (audio-lead).
    JCut,
    /// `L-CUT TO:` — audio of previous shot trails visual.
    LCut,
    /// Anything else recognized as a transition by syntax but not by
    /// vocabulary — carried through as opaque.
    Other,
}
