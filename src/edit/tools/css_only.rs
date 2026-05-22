//! CSS-only edits — append a `<style>` block to a scene HTML and
//! re-render. Cheap, fast, no pixel models touched.

use std::path::{Path, PathBuf};

use crate::edit::plan::Step;
use crate::edit::EditError;

/// Inject the per-step CSS into a copy of the source HTML and return
/// the new HTML string. The injected rules are wrapped in a single
/// trailing `<style data-wavelet-edit>` block — the last `<style>` in
/// the document, so its declarations win cascade-wise.
pub fn apply_css_steps(html: &str, steps: &[Step]) -> Result<String, EditError> {
    let mut rules = String::new();
    let mut duration_override_secs: Option<f32> = None;
    for step in steps {
        match step {
            Step::CssFilter { target_selector, css } => {
                rules.push_str(&format!("{target_selector} {{ {css} }}\n"));
            }
            Step::CssAnimation { target_selector, css } => {
                // Author wrote the CSS already including @keyframes etc.
                // The selector binding for `animation:` should be in the
                // raw `css` blob too; if it's just declarations, wrap them.
                if css.contains('{') {
                    rules.push_str(css);
                    rules.push('\n');
                } else {
                    rules.push_str(&format!("{target_selector} {{ {css} }}\n"));
                }
            }
            Step::PlaybackRate { target_selector, value } => {
                // We can't multiply the existing `animation-duration`
                // declaratively, but `animation-duration` is settable
                // on the cascade. The planner is expected to compute
                // the new absolute duration when it knows the source
                // duration. For the generic case, emit a CSS variable
                // hook the scene can read.
                rules.push_str(&format!(
                    "{target_selector} {{ --wavelet-playback-rate: {value}; }}\n"
                ));
            }
            Step::DurationOverride { secs } => {
                duration_override_secs = Some(*secs);
            }
            Step::ReRender { duration_secs } => {
                if let Some(d) = duration_secs {
                    duration_override_secs = Some(*d);
                }
            }
            // Non-CSS steps are no-ops here; the executor routes them
            // to other tools.
            _ => {}
        }
    }
    let mut block = String::from("\n<style data-wavelet-edit>\n");
    block.push_str(&rules);
    if let Some(d) = duration_override_secs {
        block.push_str(&format!(
            "/* wavelet-edit duration override: {d}s — applied at re-render time */\n"
        ));
        block.push_str(&format!(
            ":root {{ --wavelet-duration-secs: {d}; }}\n"
        ));
    }
    block.push_str("</style>\n");

    // Inject immediately before </head> if present, else before </body>,
    // else append.
    let out = if let Some(idx) = find_case_insensitive(html, "</head>") {
        let mut s = String::with_capacity(html.len() + block.len());
        s.push_str(&html[..idx]);
        s.push_str(&block);
        s.push_str(&html[idx..]);
        s
    } else if let Some(idx) = find_case_insensitive(html, "</body>") {
        let mut s = String::with_capacity(html.len() + block.len());
        s.push_str(&html[..idx]);
        s.push_str(&block);
        s.push_str(&html[idx..]);
        s
    } else {
        let mut s = String::from(html);
        s.push_str(&block);
        s
    };
    Ok(out)
}

fn find_case_insensitive(haystack: &str, needle: &str) -> Option<usize> {
    let h = haystack.to_ascii_lowercase();
    let n = needle.to_ascii_lowercase();
    h.find(&n)
}

/// Write the edited HTML to a sibling file and render it to MP4.
///
/// `scene_html` is the path to the *original* scene HTML; the edited
/// copy is written next to it with a `.wavelet-edit.html` suffix. The
/// MP4 lands at `out_mp4`.
pub fn render_css_edit(
    scene_html: &Path,
    edited_html: &str,
    out_mp4: &Path,
) -> Result<PathBuf, EditError> {
    use crate::compose::load_index_html;
    use crate::render_offline::render_composition;

    let edit_path = scene_html.with_extension("wavelet-edit.html");
    std::fs::write(&edit_path, edited_html)
        .map_err(|e| EditError::Transport(format!("write edited html: {e}")))?;
    let comp = load_index_html(&edit_path)
        .map_err(|e| EditError::Transport(format!("load_index_html: {e}")))?;
    let root_dir = edit_path
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| std::path::Path::new(".").to_path_buf());
    render_composition(&comp, &root_dir, out_mp4)
        .map_err(|e| EditError::Transport(format!("render_composition: {e}")))?;
    Ok(out_mp4.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn css_filter_step_injects_style_block_in_head() {
        let html = "<!doctype html><html><head><title>x</title></head><body>hi</body></html>";
        let steps = vec![Step::CssFilter {
            target_selector: "body".into(),
            css: "filter: brightness(0.6) hue-rotate(20deg);".into(),
        }];
        let edited = apply_css_steps(html, &steps).unwrap();
        assert!(edited.contains("data-wavelet-edit"));
        assert!(edited.contains("filter: brightness(0.6) hue-rotate(20deg);"));
        // Block was inserted before </head>
        let style_idx = edited.find("data-wavelet-edit").unwrap();
        let head_close_idx = edited.find("</head>").unwrap();
        assert!(style_idx < head_close_idx);
    }

    #[test]
    fn duration_override_emits_css_var() {
        let html = "<!doctype html><html><head></head><body></body></html>";
        let steps = vec![Step::DurationOverride { secs: 8.0 }];
        let edited = apply_css_steps(html, &steps).unwrap();
        assert!(edited.contains("--wavelet-duration-secs: 8"));
    }

    #[test]
    fn playback_rate_emits_css_var() {
        let html = "<!doctype html><html><head></head><body></body></html>";
        let steps = vec![Step::PlaybackRate {
            target_selector: ".carafe".into(),
            value: 1.5,
        }];
        let edited = apply_css_steps(html, &steps).unwrap();
        assert!(edited.contains("--wavelet-playback-rate: 1.5"));
        assert!(edited.contains(".carafe"));
    }

    #[test]
    fn no_head_falls_back_to_body() {
        let html = "<html><body>hi</body></html>";
        let steps = vec![Step::CssFilter {
            target_selector: "body".into(),
            css: "color: red;".into(),
        }];
        let edited = apply_css_steps(html, &steps).unwrap();
        let style_idx = edited.find("data-wavelet-edit").unwrap();
        let body_close_idx = edited.find("</body>").unwrap();
        assert!(style_idx < body_close_idx);
    }
}
