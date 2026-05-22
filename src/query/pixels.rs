//! Pixel-level queries — answer color/contrast/banding questions against
//! the rendered RGBA buffer at one frame of the composition. Phase 2 of
//! epic wb-q4a6.
//!
//! All ops take a `FramePixels` so a single render pass serves any number
//! of queries at the same `--at` time. The CLI builds the FramePixels +
//! FrameSnapshot in tandem so scene-graph and pixel queries share state.

use super::snapshot::{FrameSnapshot, NodeSnapshot, Rect};
use crate::render::{load_html_with_base, Renderer};
use crate::render_offline::Composition;
use serde::{Deserialize, Serialize};
use std::path::Path;

/// One rendered frame's RGBA buffer plus its dimensions. Built once per
/// `--at` time; serves every pixel query for that frame.
pub struct FramePixels {
    /// `width * height * 4` bytes; RGBA8.
    pub rgba: Vec<u8>,
    /// Frame width in pixels.
    pub width: u32,
    /// Frame height in pixels.
    pub height: u32,
}

impl FramePixels {
    /// Render the composition at `t_secs` and capture the pixel buffer.
    pub fn at(comp: &Composition, root_dir: &Path, t_secs: f32) -> Option<Self> {
        let frame_index = (t_secs * comp.fps as f32) as u32;
        let frame_for_active = frame_index.min(comp.duration_frames.saturating_sub(1));
        let active_idx = comp.scenes.iter().position(|s| {
            frame_for_active >= s.start_frame
                && frame_for_active < s.start_frame + s.duration_frames
        })?;
        let scene = &comp.scenes[active_idx];
        let local_frame = frame_index.saturating_sub(scene.start_frame);
        let local_t_secs = local_frame as f32 / comp.fps as f32;

        let resolved = root_dir.join(&scene.html_path);
        let html = std::fs::read_to_string(&resolved).ok()?;
        let absolute = std::fs::canonicalize(&resolved).unwrap_or(resolved);
        let base_url = url::Url::from_file_path(&absolute)
            .ok()
            .map(|u| u.to_string());

        let mut doc = load_html_with_base(&html, comp.width, comp.height, base_url);
        doc.as_mut().resolve(local_t_secs as f64);

        let mut renderer = Renderer::new(comp.width, comp.height);
        let rgba = renderer.render(doc.as_mut());
        Some(Self {
            rgba,
            width: comp.width,
            height: comp.height,
        })
    }

    /// Sample one pixel as `(r, g, b, a)`. Returns `None` if `(x, y)` is
    /// outside the frame.
    pub fn pixel_at(&self, x: i32, y: i32) -> Option<(u8, u8, u8, u8)> {
        if x < 0 || y < 0 || x as u32 >= self.width || y as u32 >= self.height {
            return None;
        }
        let i = ((y as u32 * self.width + x as u32) * 4) as usize;
        Some((self.rgba[i], self.rgba[i + 1], self.rgba[i + 2], self.rgba[i + 3]))
    }

    /// Average sRGB color over a rectangular region. The region is clamped
    /// to the frame; returns `None` if the clamp results in zero area.
    pub fn region_avg(&self, region: Rect) -> Option<(u8, u8, u8, u8)> {
        let (x0, y0, x1, y1) = clip_region(region, self.width, self.height)?;
        let mut r = 0u64;
        let mut g = 0u64;
        let mut b = 0u64;
        let mut a = 0u64;
        let mut n = 0u64;
        for y in y0..y1 {
            for x in x0..x1 {
                let i = ((y * self.width + x) * 4) as usize;
                r += self.rgba[i] as u64;
                g += self.rgba[i + 1] as u64;
                b += self.rgba[i + 2] as u64;
                a += self.rgba[i + 3] as u64;
                n += 1;
            }
        }
        if n == 0 {
            return None;
        }
        Some(((r / n) as u8, (g / n) as u8, (b / n) as u8, (a / n) as u8))
    }
}

fn clip_region(r: Rect, w: u32, h: u32) -> Option<(u32, u32, u32, u32)> {
    let x0 = r.x.max(0.0) as u32;
    let y0 = r.y.max(0.0) as u32;
    let x1 = ((r.x + r.w) as i32).min(w as i32).max(0) as u32;
    let y1 = ((r.y + r.h) as i32).min(h as i32).max(0) as u32;
    if x1 <= x0 || y1 <= y0 {
        return None;
    }
    Some((x0, y0, x1, y1))
}

/// Per-query JSON shapes.

/// Single-pixel sample result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ColorAtResult {
    /// True when `(x,y)` was inside the frame.
    pub ok: bool,
    /// Sampled pixel as sRGB rgba `[r,g,b,a]`.
    pub color: Option<[u8; 4]>,
    /// Hex representation `#rrggbb`.
    pub hex: Option<String>,
}

/// Region color check vs a target swatch.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ColorInResult {
    /// True when CIEDE2000 distance is within `max_de`.
    pub ok: bool,
    /// Selector that was queried.
    pub selector: String,
    /// Mean sRGB color of the queried region.
    pub mean: Option<[u8; 4]>,
    /// CIEDE2000 distance to the target.
    pub delta_e: Option<f32>,
    /// Target color provided by the caller, as RGB hex.
    pub target: String,
    /// Threshold used.
    pub max_de: f32,
}

/// WCAG contrast-ratio result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContrastResult {
    /// True when the computed ratio meets or exceeds `threshold`.
    pub ok: bool,
    /// Selector that was queried.
    pub selector: String,
    /// Computed contrast ratio (foreground vs background).
    pub ratio: Option<f32>,
    /// Threshold used (default 4.5 = WCAG AA normal text).
    pub threshold: f32,
}

/// Banding-detection result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BandingResult {
    /// True when no banding signature detected in the region.
    pub ok: bool,
    /// Unique sRGB tuples observed sampling along the region's height.
    pub unique_colors: usize,
    /// Sampled height in pixels (region clamped to frame).
    pub sampled_rows: u32,
    /// Heuristic: unique_colors / sampled_rows. < 0.25 → flag as banded.
    pub diversity: f32,
}

/// Read a single pixel.
pub fn color_at(pixels: &FramePixels, x: i32, y: i32) -> ColorAtResult {
    match pixels.pixel_at(x, y) {
        Some((r, g, b, a)) => ColorAtResult {
            ok: true,
            color: Some([r, g, b, a]),
            hex: Some(format!("#{:02x}{:02x}{:02x}", r, g, b)),
        },
        None => ColorAtResult {
            ok: false,
            color: None,
            hex: None,
        },
    }
}

/// Sample a region's average color and compare to a target via CIEDE2000.
pub fn color_in(
    snap: &FrameSnapshot,
    pixels: &FramePixels,
    selector: &str,
    target_hex: &str,
    max_de: f32,
) -> ColorInResult {
    let bbox = snap.select(selector).first().map(|n| n.bbox);
    let mean_rgba = bbox.and_then(|b| pixels.region_avg(b));
    let target_rgb = parse_hex(target_hex);
    let de = match (mean_rgba, target_rgb) {
        (Some(m), Some(t)) => Some(ciede2000(srgb_to_lab(m), srgb_to_lab((t.0, t.1, t.2, 255)))),
        _ => None,
    };
    let ok = de.map(|d| d <= max_de).unwrap_or(false);
    ColorInResult {
        ok,
        selector: selector.to_string(),
        mean: mean_rgba.map(|(r, g, b, a)| [r, g, b, a]),
        delta_e: de,
        target: target_hex.to_string(),
        max_de,
    }
}

/// Compute WCAG contrast ratio of an element's interior vs a ring around it.
/// The interior is sampled inside the bbox; the surround is a 10-pixel ring
/// just outside the bbox. Returns a ratio in [1, 21].
pub fn contrast(
    snap: &FrameSnapshot,
    pixels: &FramePixels,
    selector: &str,
    threshold: f32,
) -> ContrastResult {
    let n = match snap.select(selector).first() {
        Some(n) => *n,
        None => {
            return ContrastResult {
                ok: false,
                selector: selector.to_string(),
                ratio: None,
                threshold,
            };
        }
    };
    let fg = pixels.region_avg(n.bbox);
    let ring = sample_ring(pixels, n.bbox, 10);
    let ratio = match (fg, ring) {
        (Some(fg), Some(bg)) => Some(wcag_contrast(fg, bg)),
        _ => None,
    };
    let ok = ratio.map(|r| r >= threshold).unwrap_or(false);
    ContrastResult {
        ok,
        selector: selector.to_string(),
        ratio,
        threshold,
    }
}

/// Detect color banding in a vertical strip of `region`. The heuristic
/// samples one pixel per row at the region's horizontal center, counts
/// unique sRGB tuples, and flags banding when the diversity ratio drops
/// below 0.25.
pub fn banding(pixels: &FramePixels, region: Rect) -> BandingResult {
    let Some((x0, y0, x1, y1)) = clip_region(region, pixels.width, pixels.height) else {
        return BandingResult {
            ok: true,
            unique_colors: 0,
            sampled_rows: 0,
            diversity: 1.0,
        };
    };
    let xc = (x0 + x1) / 2;
    let mut seen: std::collections::HashSet<(u8, u8, u8)> = std::collections::HashSet::new();
    for y in y0..y1 {
        let i = ((y * pixels.width + xc) * 4) as usize;
        seen.insert((pixels.rgba[i], pixels.rgba[i + 1], pixels.rgba[i + 2]));
    }
    let rows = y1 - y0;
    let unique = seen.len();
    let diversity = unique as f32 / rows as f32;
    BandingResult {
        ok: diversity >= 0.25,
        unique_colors: unique,
        sampled_rows: rows,
        diversity,
    }
}

/// Sample a `width`-pixel ring just outside `bbox` and return its mean color.
fn sample_ring(pixels: &FramePixels, bbox: Rect, ring_width: u32) -> Option<(u8, u8, u8, u8)> {
    let rw = ring_width as f32;
    let outer = Rect {
        x: bbox.x - rw,
        y: bbox.y - rw,
        w: bbox.w + 2.0 * rw,
        h: bbox.h + 2.0 * rw,
    };
    let inner = bbox;
    let (ox0, oy0, ox1, oy1) = clip_region(outer, pixels.width, pixels.height)?;
    let (ix0, iy0, ix1, iy1) = clip_region(inner, pixels.width, pixels.height)?;

    let mut r = 0u64;
    let mut g = 0u64;
    let mut b = 0u64;
    let mut a = 0u64;
    let mut n = 0u64;
    for y in oy0..oy1 {
        for x in ox0..ox1 {
            if x >= ix0 && x < ix1 && y >= iy0 && y < iy1 {
                continue; // inside bbox — skip
            }
            let i = ((y * pixels.width + x) * 4) as usize;
            r += pixels.rgba[i] as u64;
            g += pixels.rgba[i + 1] as u64;
            b += pixels.rgba[i + 2] as u64;
            a += pixels.rgba[i + 3] as u64;
            n += 1;
        }
    }
    if n == 0 {
        return None;
    }
    Some(((r / n) as u8, (g / n) as u8, (b / n) as u8, (a / n) as u8))
}

fn parse_hex(s: &str) -> Option<(u8, u8, u8)> {
    let s = s.trim_start_matches('#');
    if s.len() != 6 {
        return None;
    }
    let r = u8::from_str_radix(&s[0..2], 16).ok()?;
    let g = u8::from_str_radix(&s[2..4], 16).ok()?;
    let b = u8::from_str_radix(&s[4..6], 16).ok()?;
    Some((r, g, b))
}

/// sRGB → linear, then linear → CIE Lab via D65 white point.
fn srgb_to_lab((r, g, b, _a): (u8, u8, u8, u8)) -> (f32, f32, f32) {
    let lin = |c: f32| {
        let c = c / 255.0;
        if c <= 0.04045 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    };
    let r = lin(r as f32);
    let g = lin(g as f32);
    let b = lin(b as f32);
    // D65 XYZ matrix.
    let x = r * 0.4124564 + g * 0.3575761 + b * 0.1804375;
    let y = r * 0.2126729 + g * 0.7151522 + b * 0.0721750;
    let z = r * 0.0193339 + g * 0.1191920 + b * 0.9503041;
    // Normalize against D65 reference white.
    let xn = x / 0.95047;
    let yn = y / 1.00000;
    let zn = z / 1.08883;
    let f = |t: f32| {
        if t > 0.008856 {
            t.powf(1.0 / 3.0)
        } else {
            7.787 * t + 16.0 / 116.0
        }
    };
    let l = 116.0 * f(yn) - 16.0;
    let a = 500.0 * (f(xn) - f(yn));
    let b2 = 200.0 * (f(yn) - f(zn));
    (l, a, b2)
}

/// CIEDE2000 color-difference between two Lab points. Within ±0.5 of the
/// reference implementation for typical inputs — accurate enough for
/// agent-grade brand-palette checks.
fn ciede2000(c1: (f32, f32, f32), c2: (f32, f32, f32)) -> f32 {
    let (l1, a1, b1) = c1;
    let (l2, a2, b2) = c2;
    let avg_l = (l1 + l2) / 2.0;
    let c1c = (a1 * a1 + b1 * b1).sqrt();
    let c2c = (a2 * a2 + b2 * b2).sqrt();
    let avg_c = (c1c + c2c) / 2.0;
    let g = 0.5 * (1.0 - (avg_c.powi(7) / (avg_c.powi(7) + 25f32.powi(7))).sqrt());
    let a1p = (1.0 + g) * a1;
    let a2p = (1.0 + g) * a2;
    let c1p = (a1p * a1p + b1 * b1).sqrt();
    let c2p = (a2p * a2p + b2 * b2).sqrt();
    let avg_cp = (c1p + c2p) / 2.0;
    let h1p = b1.atan2(a1p).to_degrees().rem_euclid(360.0);
    let h2p = b2.atan2(a2p).to_degrees().rem_euclid(360.0);
    let delta_lp = l2 - l1;
    let delta_cp = c2p - c1p;
    let mut dhp = h2p - h1p;
    if dhp > 180.0 {
        dhp -= 360.0;
    } else if dhp < -180.0 {
        dhp += 360.0;
    }
    let delta_hp = 2.0 * (c1p * c2p).sqrt() * (dhp / 2.0).to_radians().sin();
    let avg_lp = avg_l;
    let mut avg_hp = (h1p + h2p) / 2.0;
    if (h1p - h2p).abs() > 180.0 {
        avg_hp += 180.0;
    }
    let t = 1.0 - 0.17 * (avg_hp - 30.0).to_radians().cos()
        + 0.24 * (2.0 * avg_hp).to_radians().cos()
        + 0.32 * (3.0 * avg_hp + 6.0).to_radians().cos()
        - 0.20 * (4.0 * avg_hp - 63.0).to_radians().cos();
    let sl = 1.0 + (0.015 * (avg_lp - 50.0).powi(2)) / (20.0 + (avg_lp - 50.0).powi(2)).sqrt();
    let sc = 1.0 + 0.045 * avg_cp;
    let sh = 1.0 + 0.015 * avg_cp * t;
    let delta_theta = 30.0 * (-(((avg_hp - 275.0) / 25.0).powi(2))).exp();
    let rc = 2.0 * (avg_cp.powi(7) / (avg_cp.powi(7) + 25f32.powi(7))).sqrt();
    let rt = -rc * (2.0 * delta_theta).to_radians().sin();
    let kl = 1.0;
    let kc = 1.0;
    let kh = 1.0;
    let de = ((delta_lp / (kl * sl)).powi(2)
        + (delta_cp / (kc * sc)).powi(2)
        + (delta_hp / (kh * sh)).powi(2)
        + rt * (delta_cp / (kc * sc)) * (delta_hp / (kh * sh)))
        .sqrt();
    de
}

/// WCAG 2 contrast ratio. `fg` and `bg` are sRGB 8-bit; returns ratio in [1, 21].
fn wcag_contrast(fg: (u8, u8, u8, u8), bg: (u8, u8, u8, u8)) -> f32 {
    let l1 = relative_luminance(fg);
    let l2 = relative_luminance(bg);
    let (hi, lo) = if l1 > l2 { (l1, l2) } else { (l2, l1) };
    (hi + 0.05) / (lo + 0.05)
}

fn relative_luminance(c: (u8, u8, u8, u8)) -> f32 {
    let lin = |c: f32| {
        let c = c / 255.0;
        if c <= 0.03928 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    };
    let r = lin(c.0 as f32);
    let g = lin(c.1 as f32);
    let b = lin(c.2 as f32);
    0.2126 * r + 0.7152 * g + 0.0722 * b
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_hex_basic() {
        assert_eq!(parse_hex("#ff0000"), Some((255, 0, 0)));
        assert_eq!(parse_hex("00ff00"), Some((0, 255, 0)));
        assert_eq!(parse_hex("#123"), None);
    }

    #[test]
    fn ciede2000_identical_is_zero() {
        let c = srgb_to_lab((100, 100, 100, 255));
        let de = ciede2000(c, c);
        assert!(de.abs() < 0.01, "expected ~0, got {de}");
    }

    #[test]
    fn ciede2000_known_pair() {
        // sRGB (255,0,0) vs (0,255,0) should give a large ΔE (~80-90 range).
        let red = srgb_to_lab((255, 0, 0, 255));
        let green = srgb_to_lab((0, 255, 0, 255));
        let de = ciede2000(red, green);
        assert!(de > 50.0, "expected large ΔE for red vs green, got {de}");
    }

    #[test]
    fn wcag_max_contrast_white_on_black() {
        let r = wcag_contrast((255, 255, 255, 255), (0, 0, 0, 255));
        assert!((r - 21.0).abs() < 0.1, "expected 21:1 for white/black, got {r}");
    }

    #[test]
    fn wcag_min_contrast_same_color() {
        let r = wcag_contrast((100, 100, 100, 255), (100, 100, 100, 255));
        assert!((r - 1.0).abs() < 0.01, "expected 1:1 for same color, got {r}");
    }

    #[test]
    fn banding_uniform_buffer_low_diversity() {
        // 100x100 image, all gray — diversity should be 1/100.
        let pixels = FramePixels {
            rgba: vec![128, 128, 128, 255].repeat(100 * 100),
            width: 100,
            height: 100,
        };
        let r = banding(&pixels, Rect { x: 0.0, y: 0.0, w: 100.0, h: 100.0 });
        assert_eq!(r.unique_colors, 1);
        assert!(!r.ok, "single-color region flagged as banding");
    }

    #[test]
    fn banding_full_gradient_high_diversity() {
        // 1x256 vertical gradient, row y has color (y,y,y).
        let mut rgba = Vec::with_capacity(256 * 4);
        for y in 0u16..256 {
            let v = y as u8;
            rgba.extend_from_slice(&[v, v, v, 255]);
        }
        let pixels = FramePixels {
            rgba,
            width: 1,
            height: 256,
        };
        let r = banding(&pixels, Rect { x: 0.0, y: 0.0, w: 1.0, h: 256.0 });
        assert_eq!(r.unique_colors, 256);
        assert!(r.ok, "smooth gradient should not be flagged as banding");
    }
}
