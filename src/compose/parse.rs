//! Thin tag scanner for the manifest HTML. We don't need a real DOM — the
//! manifest is structurally tiny (a flat list of `<meta>`, `<section>`, and
//! `<audio>` tags). A regex-free attribute scanner keeps this cheap and
//! avoids pulling in a full HTML5 parser at parse time.

/// One element we care about from the manifest body.
#[derive(Debug, Clone)]
pub struct Element {
    /// Which kind of element this is (filters out everything else).
    pub kind: ElementKind,
    /// Attributes in the order they appear on the source tag.
    pub attrs: Vec<(String, String)>,
}

impl Element {
    /// Get an attribute by name (case-insensitive). Returns `None` if absent.
    pub fn attr(&self, key: &str) -> Option<&str> {
        self.attrs
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(key))
            .map(|(_, v)| v.as_str())
    }
}

/// Manifest body elements we extract.
#[derive(Debug, Clone, Copy)]
pub enum ElementKind {
    /// `<section>` — defines a scene.
    Section,
    /// `<audio>` — defines an audio cue.
    Audio,
    /// `<video>` — inline media element in a scene HTML (used by the
    /// render pre-flight to validate asset existence; not a manifest-
    /// level concept).
    Video,
}

/// One `<meta name="…" content="…">` tag we found in `<head>`.
#[derive(Debug, Clone)]
pub(super) struct MetaTag {
    pub name: String,
    pub content: String,
}

/// Collect all `<meta>` tags with both a `name` and a `content` attribute.
pub(super) fn collect_meta(html: &str) -> Vec<MetaTag> {
    let mut out = Vec::new();
    for tag in iter_tags(html) {
        if !tag.name.eq_ignore_ascii_case("meta") {
            continue;
        }
        let name = attr_value(&tag.body, "name");
        let content = attr_value(&tag.body, "content");
        if let (Some(name), Some(content)) = (name, content) {
            out.push(MetaTag { name, content });
        }
    }
    out
}

/// Collect every `<section>` and `<audio>` tag from the manifest, in source
/// order. Errors only on truly malformed input (e.g. unterminated tag).
pub fn collect_elements(html: &str) -> Result<Vec<Element>, String> {
    let mut out = Vec::new();
    for tag in iter_tags(html) {
        let kind = match tag.name.to_ascii_lowercase().as_str() {
            "section" => ElementKind::Section,
            "audio" => ElementKind::Audio,
            "video" => ElementKind::Video,
            _ => continue,
        };
        out.push(Element {
            kind,
            attrs: parse_attrs(&tag.body),
        });
    }
    Ok(out)
}

struct RawTag<'a> {
    name: &'a str,
    body: &'a str,
}

/// Walk every `<…>` tag in the source. Skips `<!doctype>`, comments, and
/// closing `</…>` tags. The body excludes the leading tag name and any
/// trailing `/`.
fn iter_tags(html: &str) -> Vec<RawTag<'_>> {
    let bytes = html.as_bytes();
    let mut out = Vec::new();
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] != b'<' {
            i += 1;
            continue;
        }
        // Comments / doctype — skip.
        if bytes[i..].starts_with(b"<!--") {
            if let Some(end) = find_subseq(&bytes[i + 4..], b"-->") {
                i += 4 + end + 3;
                continue;
            }
            break;
        }
        if i + 1 < bytes.len() && bytes[i + 1] == b'!' {
            if let Some(end) = bytes[i..].iter().position(|&c| c == b'>') {
                i += end + 1;
                continue;
            }
            break;
        }
        // Closing tag — skip.
        if i + 1 < bytes.len() && bytes[i + 1] == b'/' {
            if let Some(end) = bytes[i..].iter().position(|&c| c == b'>') {
                i += end + 1;
                continue;
            }
            break;
        }
        // Opening tag.
        let end = match bytes[i + 1..].iter().position(|&c| c == b'>') {
            Some(p) => i + 1 + p,
            None => break,
        };
        let inner = &html[i + 1..end];
        let name_end = inner
            .find(|c: char| c.is_ascii_whitespace() || c == '/')
            .unwrap_or(inner.len());
        let name = &inner[..name_end];
        if name.is_empty() {
            i = end + 1;
            continue;
        }
        let body = inner[name_end..].trim().trim_end_matches('/').trim();
        out.push(RawTag { name, body });
        i = end + 1;
    }
    out
}

fn find_subseq(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

/// Parse `key="value"` / `key='value'` / `key=value` / bare-flag attributes
/// from the inside of a tag (i.e. `body` from `RawTag`).
fn parse_attrs(body: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let bytes = body.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        // Skip whitespace.
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= bytes.len() {
            break;
        }
        // Read key.
        let key_start = i;
        while i < bytes.len()
            && !bytes[i].is_ascii_whitespace()
            && bytes[i] != b'='
            && bytes[i] != b'/'
        {
            i += 1;
        }
        if i == key_start {
            break;
        }
        let key = body[key_start..i].to_string();
        // Skip whitespace before '='.
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= bytes.len() || bytes[i] != b'=' {
            // Bare flag (e.g. `disabled`).
            out.push((key, String::new()));
            continue;
        }
        i += 1; // consume '='
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= bytes.len() {
            out.push((key, String::new()));
            break;
        }
        let quote = bytes[i];
        let value = if quote == b'"' || quote == b'\'' {
            i += 1;
            let start = i;
            while i < bytes.len() && bytes[i] != quote {
                i += 1;
            }
            let v = body[start..i].to_string();
            if i < bytes.len() {
                i += 1;
            }
            v
        } else {
            let start = i;
            while i < bytes.len() && !bytes[i].is_ascii_whitespace() {
                i += 1;
            }
            body[start..i].to_string()
        };
        out.push((key, decode_entities(&value)));
    }
    out
}

fn decode_entities(s: &str) -> String {
    // Cheap entity decode — manifest values almost never contain entities,
    // but `&amp;` in audio filenames or transition specs would silently
    // break things otherwise.
    s.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
}

fn attr_value(body: &str, key: &str) -> Option<String> {
    parse_attrs(body)
        .into_iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(key))
        .map(|(_, v)| v)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collects_section_and_audio_in_order() {
        let html = r#"<!doctype html><html><body>
<section data-scene-href="a.html"></section>
<audio src="x.wav"></audio>
<section data-scene-href="b.html"></section>
</body></html>"#;
        let els = collect_elements(html).unwrap();
        assert_eq!(els.len(), 3);
        assert!(matches!(els[0].kind, ElementKind::Section));
        assert!(matches!(els[1].kind, ElementKind::Audio));
        assert!(matches!(els[2].kind, ElementKind::Section));
        assert_eq!(els[0].attr("data-scene-href"), Some("a.html"));
    }

    #[test]
    fn meta_tags_extracted() {
        let html = r#"<head>
<meta name="resolution" content="1280x720">
<meta charset="utf-8">
<meta name="fps" content="30">
</head>"#;
        let metas = collect_meta(html);
        assert_eq!(metas.len(), 2);
        assert_eq!(metas[0].name, "resolution");
        assert_eq!(metas[0].content, "1280x720");
        assert_eq!(metas[1].name, "fps");
    }

    #[test]
    fn parses_single_and_double_quoted_attrs() {
        let html =
            r#"<section data-a='one' data-b="two" data-c=three></section>"#;
        let els = collect_elements(html).unwrap();
        assert_eq!(els[0].attr("data-a"), Some("one"));
        assert_eq!(els[0].attr("data-b"), Some("two"));
        assert_eq!(els[0].attr("data-c"), Some("three"));
    }

    #[test]
    fn ignores_comments_and_doctype() {
        let html = r#"<!doctype html>
<!-- <section data-scene-href="ignored.html"></section> -->
<section data-scene-href="kept.html"></section>"#;
        let els = collect_elements(html).unwrap();
        assert_eq!(els.len(), 1);
        assert_eq!(els[0].attr("data-scene-href"), Some("kept.html"));
    }

    #[test]
    fn self_closing_tag() {
        let html = r#"<audio src="x.wav" />"#;
        let els = collect_elements(html).unwrap();
        assert_eq!(els.len(), 1);
        assert_eq!(els[0].attr("src"), Some("x.wav"));
    }
}
