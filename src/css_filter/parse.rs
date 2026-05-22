//! Parser for CSS `filter:` value strings.

use super::types::{FilterFn, FilterParseError, Length, LengthUnit};


/// Parse a CSS `filter:` value (the substring after the `:`) into a
/// chain of [`FilterFn`]s. Whitespace between function calls is
/// optional; case is normalized for function names.
///
/// Example:
/// ```
/// use crate::css_filter::{parse_filter_value, FilterFn};
/// let chain = parse_filter_value("blur(8px) brightness(0.85)").unwrap();
/// assert_eq!(chain.len(), 2);
/// matches!(chain[0], FilterFn::Blur(_));
/// matches!(chain[1], FilterFn::Brightness(_));
/// ```
pub fn parse_filter_value(raw: &str) -> Result<Vec<FilterFn>, FilterParseError> {
    let s = raw.trim();
    if s.is_empty() || s.eq_ignore_ascii_case("none") {
        return Err(FilterParseError::Empty);
    }
    let mut out = Vec::new();
    let mut cursor = 0;
    let bytes = s.as_bytes();
    while cursor < bytes.len() {
        // skip whitespace + commas (CSS allows both as separators)
        while cursor < bytes.len() && matches!(bytes[cursor], b' ' | b'\t' | b'\n' | b',') {
            cursor += 1;
        }
        if cursor >= bytes.len() {
            break;
        }
        // function name: ident chars up to '('
        let name_start = cursor;
        while cursor < bytes.len() && bytes[cursor] != b'(' {
            cursor += 1;
        }
        if cursor >= bytes.len() {
            return Err(FilterParseError::Unterminated(
                s[name_start..].to_string(),
            ));
        }
        let name = s[name_start..cursor].trim().to_ascii_lowercase();
        cursor += 1; // skip '('
        // find matching ')' — drop-shadow may contain rgba(...) which has
        // nested parens; count depth.
        let args_start = cursor;
        let mut depth = 1usize;
        while cursor < bytes.len() && depth > 0 {
            match bytes[cursor] {
                b'(' => depth += 1,
                b')' => depth -= 1,
                _ => {}
            }
            cursor += 1;
        }
        if depth != 0 {
            return Err(FilterParseError::Unterminated(name));
        }
        let args = &s[args_start..cursor - 1]; // everything between ( and )
        out.push(parse_one(&name, args.trim())?);
    }
    Ok(out)
}

fn parse_one(name: &str, args: &str) -> Result<FilterFn, FilterParseError> {
    match name {
        "blur" => Ok(FilterFn::Blur(parse_length(name, args)?)),
        "brightness" => Ok(FilterFn::Brightness(parse_number_or_percent(name, args)?)),
        "contrast" => Ok(FilterFn::Contrast(parse_number_or_percent(name, args)?)),
        "saturate" => Ok(FilterFn::Saturate(parse_number_or_percent(name, args)?)),
        "grayscale" => Ok(FilterFn::Grayscale(parse_number_or_percent(name, args)?.min(1.0))),
        "sepia" => Ok(FilterFn::Sepia(parse_number_or_percent(name, args)?.min(1.0))),
        "invert" => Ok(FilterFn::Invert(parse_number_or_percent(name, args)?.min(1.0))),
        "opacity" => Ok(FilterFn::Opacity(parse_number_or_percent(name, args)?.min(1.0))),
        "hue-rotate" => Ok(FilterFn::HueRotate(parse_angle(name, args)?)),
        "drop-shadow" => parse_drop_shadow(args),
        other => Err(FilterParseError::UnknownFunction(other.to_string())),
    }
}

fn parse_length(func: &str, raw: &str) -> Result<Length, FilterParseError> {
    let s = raw.trim();
    let invalid = || FilterParseError::InvalidArgument {
        func: func.to_string(),
        arg: raw.to_string(),
    };
    // Split numeric prefix from unit suffix.
    let split_at = s
        .find(|c: char| !c.is_ascii_digit() && c != '.' && c != '-' && c != '+')
        .unwrap_or(s.len());
    let (num_str, unit_str) = s.split_at(split_at);
    let value: f32 = num_str.parse().map_err(|_| invalid())?;
    let unit = match unit_str.trim() {
        "" | "px" => LengthUnit::Px,
        "em" => LengthUnit::Em,
        "rem" => LengthUnit::Rem,
        "vh" => LengthUnit::Vh,
        "vw" => LengthUnit::Vw,
        "%" => LengthUnit::Percent,
        _ => return Err(invalid()),
    };
    Ok(Length { value, unit })
}

fn parse_number_or_percent(func: &str, raw: &str) -> Result<f32, FilterParseError> {
    let s = raw.trim();
    let invalid = || FilterParseError::InvalidArgument {
        func: func.to_string(),
        arg: raw.to_string(),
    };
    if let Some(stripped) = s.strip_suffix('%') {
        let v: f32 = stripped.trim().parse().map_err(|_| invalid())?;
        Ok(v / 100.0)
    } else if s.is_empty() {
        // CSS spec: `brightness()` with empty args = 1.0 (identity).
        Ok(1.0)
    } else {
        s.parse::<f32>().map_err(|_| invalid())
    }
}

fn parse_angle(func: &str, raw: &str) -> Result<f32, FilterParseError> {
    let s = raw.trim();
    let invalid = || FilterParseError::InvalidArgument {
        func: func.to_string(),
        arg: raw.to_string(),
    };
    if s.is_empty() {
        return Ok(0.0);
    }
    // Find the unit (deg / rad / turn / grad), parse remainder as number.
    let split_at = s
        .find(|c: char| c.is_ascii_alphabetic())
        .unwrap_or(s.len());
    let (num_str, unit) = s.split_at(split_at);
    let v: f32 = num_str.trim().parse().map_err(|_| invalid())?;
    let degrees = match unit.trim() {
        "" | "deg" => v,
        "rad" => v.to_degrees(),
        "turn" => v * 360.0,
        "grad" => v * 360.0 / 400.0,
        _ => return Err(invalid()),
    };
    Ok(degrees)
}

fn parse_drop_shadow(args: &str) -> Result<FilterFn, FilterParseError> {
    let func = "drop-shadow";
    let invalid = |what: &str| FilterParseError::InvalidArgument {
        func: func.to_string(),
        arg: what.to_string(),
    };
    // drop-shadow has at most 4 components: 2-3 lengths + optional color.
    // The color may contain spaces (rgba(0, 0, 0, 0.5)) so we tokenize
    // carefully: split on whitespace but treat `(...)` as a single token.
    let mut tokens: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut depth = 0usize;
    for ch in args.chars() {
        match ch {
            '(' => {
                depth += 1;
                cur.push(ch);
            }
            ')' => {
                depth = depth.saturating_sub(1);
                cur.push(ch);
            }
            ' ' | '\t' | '\n' if depth == 0 => {
                if !cur.is_empty() {
                    tokens.push(std::mem::take(&mut cur));
                }
            }
            _ => cur.push(ch),
        }
    }
    if !cur.is_empty() {
        tokens.push(cur);
    }
    // Now: identify which tokens are lengths and which is a color.
    // CSS spec lets the color come first OR last; we handle both.
    let mut lengths: Vec<Length> = Vec::new();
    let mut color: Option<[f32; 4]> = None;
    for tok in &tokens {
        if looks_like_color(tok) {
            color = Some(parse_color(tok).ok_or_else(|| invalid(tok))?);
        } else {
            lengths.push(parse_length(func, tok)?);
        }
    }
    if lengths.len() < 2 || lengths.len() > 3 {
        return Err(invalid(args));
    }
    let offset_x = lengths[0];
    let offset_y = lengths[1];
    let blur_radius = lengths.get(2).copied().unwrap_or(Length {
        value: 0.0,
        unit: LengthUnit::Px,
    });
    Ok(FilterFn::DropShadow {
        offset_x,
        offset_y,
        blur_radius,
        color: color.unwrap_or([0.0, 0.0, 0.0, 1.0]),
    })
}

fn looks_like_color(tok: &str) -> bool {
    let t = tok.trim();
    t.starts_with('#')
        || t.eq_ignore_ascii_case("transparent")
        || t.eq_ignore_ascii_case("currentcolor")
        || t.to_ascii_lowercase().starts_with("rgb(")
        || t.to_ascii_lowercase().starts_with("rgba(")
        || t.to_ascii_lowercase().starts_with("hsl(")
        || t.to_ascii_lowercase().starts_with("hsla(")
        // Named CSS colors — common ones; expand if scenes start using
        // exotic names. We deliberately don't ship the full ~150-entry
        // named-color table since drop-shadow in practice uses hex or
        // rgba.
        || matches!(
            t.to_ascii_lowercase().as_str(),
            "black" | "white" | "red" | "green" | "blue" | "yellow" |
            "cyan" | "magenta" | "gray" | "grey" | "orange" | "purple"
        )
}

fn parse_color(raw: &str) -> Option<[f32; 4]> {
    let t = raw.trim();
    if let Some(hex) = t.strip_prefix('#') {
        return parse_hex_color(hex);
    }
    let lower = t.to_ascii_lowercase();
    if lower == "transparent" {
        return Some([0.0, 0.0, 0.0, 0.0]);
    }
    // Named-color shortlist.
    let named = match lower.as_str() {
        "black" => Some([0.0, 0.0, 0.0, 1.0]),
        "white" => Some([1.0, 1.0, 1.0, 1.0]),
        "red" => Some([1.0, 0.0, 0.0, 1.0]),
        "green" => Some([0.0, 0.5, 0.0, 1.0]),
        "blue" => Some([0.0, 0.0, 1.0, 1.0]),
        "yellow" => Some([1.0, 1.0, 0.0, 1.0]),
        "cyan" => Some([0.0, 1.0, 1.0, 1.0]),
        "magenta" => Some([1.0, 0.0, 1.0, 1.0]),
        "gray" | "grey" => Some([0.5, 0.5, 0.5, 1.0]),
        "orange" => Some([1.0, 0.647, 0.0, 1.0]),
        "purple" => Some([0.5, 0.0, 0.5, 1.0]),
        _ => None,
    };
    if named.is_some() {
        return named;
    }
    // rgba(R, G, B, A) — R/G/B as 0-255 or N%, A as 0-1 or N%.
    if let Some(inner) = lower
        .strip_prefix("rgba(")
        .or_else(|| lower.strip_prefix("rgb("))
        .and_then(|s| s.strip_suffix(')'))
    {
        let parts: Vec<&str> = inner.split(',').map(str::trim).collect();
        if parts.len() < 3 || parts.len() > 4 {
            return None;
        }
        let rgb: Vec<f32> = parts[..3]
            .iter()
            .map(|p| {
                if let Some(pct) = p.strip_suffix('%') {
                    pct.parse::<f32>().ok().map(|v| v / 100.0)
                } else {
                    p.parse::<f32>().ok().map(|v| v / 255.0)
                }
            })
            .collect::<Option<_>>()?;
        let a = parts
            .get(3)
            .map(|p| {
                if let Some(pct) = p.strip_suffix('%') {
                    pct.parse::<f32>().ok().map(|v| v / 100.0)
                } else {
                    p.parse::<f32>().ok()
                }
            })
            .unwrap_or(Some(1.0))?;
        return Some([rgb[0], rgb[1], rgb[2], a]);
    }
    None
}

fn parse_hex_color(hex: &str) -> Option<[f32; 4]> {
    let bytes = hex.as_bytes();
    let parse_pair = |a: u8, b: u8| -> Option<f32> {
        let val = u8::from_str_radix(&format!("{}{}", a as char, b as char), 16).ok()?;
        Some(val as f32 / 255.0)
    };
    let parse_single = |a: u8| -> Option<f32> {
        let val = u8::from_str_radix(&format!("{}{}", a as char, a as char), 16).ok()?;
        Some(val as f32 / 255.0)
    };
    match bytes.len() {
        3 => {
            let r = parse_single(bytes[0])?;
            let g = parse_single(bytes[1])?;
            let b = parse_single(bytes[2])?;
            Some([r, g, b, 1.0])
        }
        4 => {
            let r = parse_single(bytes[0])?;
            let g = parse_single(bytes[1])?;
            let b = parse_single(bytes[2])?;
            let a = parse_single(bytes[3])?;
            Some([r, g, b, a])
        }
        6 => {
            let r = parse_pair(bytes[0], bytes[1])?;
            let g = parse_pair(bytes[2], bytes[3])?;
            let b = parse_pair(bytes[4], bytes[5])?;
            Some([r, g, b, 1.0])
        }
        8 => {
            let r = parse_pair(bytes[0], bytes[1])?;
            let g = parse_pair(bytes[2], bytes[3])?;
            let b = parse_pair(bytes[4], bytes[5])?;
            let a = parse_pair(bytes[6], bytes[7])?;
            Some([r, g, b, a])
        }
        _ => None,
    }
}
