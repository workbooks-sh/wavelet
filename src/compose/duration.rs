//! Small parsers for manifest scalar values.

/// Parse a duration string into seconds.
///
/// Accepted forms:
/// - `"3s"`, `"0.5s"` — seconds with explicit unit
/// - `"1500ms"`, `"250ms"` — milliseconds
/// - `"45"`, `"2.5"` — bare number is interpreted as seconds
///
/// Returns `None` if the value can't be parsed.
pub fn parse_duration(raw: &str) -> Option<f32> {
    let s = raw.trim();
    if s.is_empty() {
        return None;
    }
    if let Some(rest) = s.strip_suffix("ms") {
        let n: f32 = rest.trim().parse().ok()?;
        return Some(n / 1000.0);
    }
    if let Some(rest) = s.strip_suffix('s') {
        let n: f32 = rest.trim().parse().ok()?;
        return Some(n);
    }
    s.parse::<f32>().ok()
}

/// Parse a resolution string of the form `"1280x720"` into `(width, height)`.
/// Whitespace is trimmed; either separator `'x'` or `'X'` is accepted.
pub fn parse_resolution(raw: &str) -> Option<(u32, u32)> {
    let s = raw.trim();
    let (w, h) = s
        .split_once('x')
        .or_else(|| s.split_once('X'))?;
    let w: u32 = w.trim().parse().ok()?;
    let h: u32 = h.trim().parse().ok()?;
    if w == 0 || h == 0 {
        return None;
    }
    Some((w, h))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn duration_with_seconds_unit() {
        assert_eq!(parse_duration("3s"), Some(3.0));
        assert_eq!(parse_duration("0.5s"), Some(0.5));
    }

    #[test]
    fn duration_with_ms_unit() {
        assert_eq!(parse_duration("1500ms"), Some(1.5));
        assert_eq!(parse_duration("250ms"), Some(0.25));
    }

    #[test]
    fn duration_bare_number_is_seconds() {
        assert_eq!(parse_duration("45"), Some(45.0));
        assert_eq!(parse_duration("2.5"), Some(2.5));
    }

    #[test]
    fn duration_garbage_returns_none() {
        assert!(parse_duration("").is_none());
        assert!(parse_duration("abc").is_none());
        assert!(parse_duration("sx").is_none());
    }

    #[test]
    fn resolution_standard_form() {
        assert_eq!(parse_resolution("1280x720"), Some((1280, 720)));
        assert_eq!(parse_resolution("1920X1080"), Some((1920, 1080)));
    }

    #[test]
    fn resolution_rejects_zero() {
        assert!(parse_resolution("0x720").is_none());
        assert!(parse_resolution("1280x0").is_none());
    }

    #[test]
    fn resolution_rejects_garbage() {
        assert!(parse_resolution("hd").is_none());
        assert!(parse_resolution("1280").is_none());
    }
}
