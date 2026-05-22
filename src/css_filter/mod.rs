//! CSS `filter:` parser, applier, and HTML-level rewriter for the
//! wavelet renderer. Blitz/Vello's native CSS filter implementation
//! hangs pathologically on certain inputs (blur radius >= 4px,
//! drop-shadow chains, filter on `<video>`); we strip filter
//! declarations from the source HTML, parse them with our own parser,
//! and apply them via our CPU/GPU paths.

pub mod apply;
pub mod hijack;
pub mod parse;
pub mod types;

pub use apply::{apply_chain_cpu, apply_chain_cpu_bbox};
pub use hijack::{hijack_filters_in_html, HijackResult};
pub use parse::parse_filter_value;
pub use types::{FilterFn, FilterParseError, Length, LengthUnit};
