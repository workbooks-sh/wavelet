//! Color-grade-coherence rule — flags shot sequences where adjacent
//! clips read as "different cameras" (luminance, color cast, contrast).
//! Operates on rendered MP4 shots, not on resolved layout. Sampling
//! happens at each shot's midpoint frame; the per-shot signature is the
//! mean CIELAB L*/a*/b* plus a P95-P5 luma contrast. A single finding
//! is emitted carrying the full pairwise table so the agent gets one
//! actionable nudge rather than N near-duplicate ones.

use super::report::{LintFinding, Severity};
use crate::query::diff::decode_rgba_frames;
use crate::query::Rect;
use std::path::{Path, PathBuf};

/// Identifier emitted in `LintFinding.rule`.
pub const RULE: &str = "color-grade-coherence";

/// deltaE above this is an Error (very-perceivable drift, bag-of-clips).
pub const ERROR_DELTA_E: f32 = 25.0;

/// deltaE above this is a Warn (perceivable, possibly intentional).
pub const WARN_DELTA_E: f32 = 12.0;

/// Downsample target. The signature stats are robust to bilinear-ish
/// downsampling; 256x256 keeps the LAB loop fast on a 1080p frame.
const DOWNSAMPLE_TARGET: u32 = 256;

/// Per-shot color signature used for pairwise drift detection.
#[derive(Debug, Clone, Copy)]
pub struct ColorGradeSignature {
    /// Mean BT.601 Y on gamma-encoded sRGB, 0..255. Treated as L for
    /// deltaE purposes — see note on `delta_e_cie76`.
    pub mean_luma: f32,
    /// Mean CIELAB a* (red-green axis).
    pub mean_a: f32,
    /// Mean CIELAB b* (yellow-blue axis).
    pub mean_b: f32,
    /// P95 luma minus P5 luma on BT.601 Y; 0..255.
    pub contrast: f32,
}

/// Result of running the rule against a directory of shots.
#[derive(Debug, Clone)]
pub struct CoherenceOutcome {
    /// The shots directory that was inspected.
    pub shots_dir: PathBuf,
    /// Per-shot signatures, in sorted-name order. Empty when the rule
    /// short-circuited (no shots discovered, single shot, etc.).
    pub signatures: Vec<(PathBuf, ColorGradeSignature)>,
    /// Findings to merge into the parent `LintReport`. At most one
    /// Error/Warn finding is emitted per run; Info findings are used
    /// for the short-circuit cases.
    pub findings: Vec<LintFinding>,
}

/// Locate the `shots/` directory given a `<PATH>` argument to
/// `wavelet lint`. Returns `Ok(None)` if no shots dir is discoverable;
/// the caller short-circuits to Info in that case.
///
/// PATH resolution:
/// - `commercial.html` → sibling `shots/`
/// - directory containing `shots/` → that subdir
/// - directory containing `shot-*.mp4` directly → itself
/// - anything else → None
pub fn discover_shots_dir(path: &Path) -> Option<PathBuf> {
    if path.is_file() {
        let parent = path.parent()?;
        let sib = parent.join("shots");
        if sib.is_dir() {
            return Some(sib);
        }
        return None;
    }
    if path.is_dir() {
        let candidate = path.join("shots");
        if candidate.is_dir() {
            return Some(candidate);
        }
        if has_shot_mp4(path) {
            return Some(path.to_path_buf());
        }
    }
    None
}

fn has_shot_mp4(dir: &Path) -> bool {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return false;
    };
    for entry in rd.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with("shot-") && name.ends_with(".mp4") {
            return true;
        }
    }
    false
}

/// Walk `shots_dir`, sorted, returning every `shot-*.mp4`.
pub fn list_shots(shots_dir: &Path) -> Result<Vec<PathBuf>, String> {
    let mut out: Vec<PathBuf> = Vec::new();
    for entry in std::fs::read_dir(shots_dir).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let p = entry.path();
        let Some(name) = p.file_name().and_then(|s| s.to_str()) else {
            continue;
        };
        if name.starts_with("shot-") && name.ends_with(".mp4") {
            out.push(p);
        }
    }
    out.sort();
    Ok(out)
}

/// Run the rule end-to-end starting from a `<PATH>` arg. Decodes each
/// shot's midpoint frame, computes signatures, and returns a single
/// finding if any pair exceeds the warn threshold. When no shots are
/// discovered, returns an Info-level "not applicable" finding.
pub fn run(path: &Path) -> Result<CoherenceOutcome, String> {
    let Some(shots_dir) = discover_shots_dir(path) else {
        return Ok(CoherenceOutcome {
            shots_dir: path.to_path_buf(),
            signatures: Vec::new(),
            findings: vec![informational(
                path,
                "no shots discovered — coherence not applicable",
            )],
        });
    };

    let shots = list_shots(&shots_dir)?;
    if shots.is_empty() {
        return Ok(CoherenceOutcome {
            shots_dir,
            signatures: Vec::new(),
            findings: vec![informational(
                path,
                "no shot-*.mp4 files in shots directory",
            )],
        });
    }
    if shots.len() == 1 {
        return Ok(CoherenceOutcome {
            shots_dir,
            signatures: Vec::new(),
            findings: vec![informational(
                &shots[0],
                "single shot, coherence not applicable",
            )],
        });
    }

    let mut signatures: Vec<(PathBuf, ColorGradeSignature)> = Vec::with_capacity(shots.len());
    for shot in &shots {
        let sig = signature_for_shot(shot)?;
        signatures.push((shot.clone(), sig));
    }

    let findings = build_findings(&shots_dir, &signatures);

    Ok(CoherenceOutcome {
        shots_dir,
        signatures,
        findings,
    })
}

fn informational(scope: &Path, message: &str) -> LintFinding {
    LintFinding {
        rule: RULE.to_string(),
        severity: Severity::Info,
        scene_path: scope.to_path_buf(),
        t_secs: 0.0,
        element_selector: String::from("shots/"),
        element_bbox: Rect { x: 0.0, y: 0.0, w: 0.0, h: 0.0 },
        message: message.to_string(),
        fix_hint: String::new(),
        subkind: None,
    }
}

/// Decode the midpoint frame and reduce to a `ColorGradeSignature`.
pub fn signature_for_shot(path: &Path) -> Result<ColorGradeSignature, String> {
    let (w, h, _fps, frames) = decode_rgba_frames(path)?;
    if frames.is_empty() {
        return Err(format!("no decoded frames in {}", path.display()));
    }
    let mid = frames.len() / 2;
    Ok(signature_from_rgba(&frames[mid], w, h))
}

/// Compute the signature from a single RGBA frame. Downsamples to ~256x256
/// before the per-pixel sRGB→LAB pass; the stats are robust to the
/// resampling and the smaller buffer keeps the inner loop cheap.
pub fn signature_from_rgba(frame: &[u8], w: u32, h: u32) -> ColorGradeSignature {
    let (small, sw, sh) = downsample(frame, w, h, DOWNSAMPLE_TARGET);
    let n = (sw as usize) * (sh as usize);
    if n == 0 {
        return ColorGradeSignature {
            mean_luma: 0.0,
            mean_a: 0.0,
            mean_b: 0.0,
            contrast: 0.0,
        };
    }

    let mut sum_l = 0.0f64;
    let mut sum_a = 0.0f64;
    let mut sum_b = 0.0f64;
    let mut sum_y = 0.0f64;
    let mut lumas: Vec<u8> = Vec::with_capacity(n);

    for i in 0..n {
        let r = small[i * 4];
        let g = small[i * 4 + 1];
        let b = small[i * 4 + 2];
        let (la, aa, ba) = srgb_u8_to_lab(r, g, b);
        let y = bt601_luma(r, g, b);
        sum_l += la as f64;
        sum_a += aa as f64;
        sum_b += ba as f64;
        sum_y += y as f64;
        lumas.push(y);
    }

    let mean_l = (sum_l / n as f64) as f32;
    let mean_a = (sum_a / n as f64) as f32;
    let mean_b = (sum_b / n as f64) as f32;
    let mean_y = (sum_y / n as f64) as f32;
    let _ = mean_y;
    let contrast = p95_minus_p5(&mut lumas);

    ColorGradeSignature {
        mean_luma: mean_l,
        mean_a,
        mean_b,
        contrast,
    }
}

/// Bilinear-ish box-downsample to a `target x target` square (or smaller
/// when the input is already small). Pure nearest-neighbour averaging
/// over integer blocks — adequate for histogram stats.
fn downsample(rgba: &[u8], w: u32, h: u32, target: u32) -> (Vec<u8>, u32, u32) {
    if w == 0 || h == 0 {
        return (Vec::new(), 0, 0);
    }
    if w <= target && h <= target {
        return (rgba.to_vec(), w, h);
    }
    let sx = (w / target).max(1);
    let sy = (h / target).max(1);
    let nw = w / sx;
    let nh = h / sy;
    let mut out = Vec::with_capacity((nw * nh * 4) as usize);
    for ny in 0..nh {
        for nx in 0..nw {
            let mut r = 0u32;
            let mut g = 0u32;
            let mut b = 0u32;
            let mut count = 0u32;
            for dy in 0..sy {
                let src_y = ny * sy + dy;
                if src_y >= h {
                    continue;
                }
                for dx in 0..sx {
                    let src_x = nx * sx + dx;
                    if src_x >= w {
                        continue;
                    }
                    let i = ((src_y * w + src_x) * 4) as usize;
                    r += rgba[i] as u32;
                    g += rgba[i + 1] as u32;
                    b += rgba[i + 2] as u32;
                    count += 1;
                }
            }
            if count == 0 {
                out.extend_from_slice(&[0, 0, 0, 255]);
            } else {
                out.push((r / count) as u8);
                out.push((g / count) as u8);
                out.push((b / count) as u8);
                out.push(255);
            }
        }
    }
    (out, nw, nh)
}

fn bt601_luma(r: u8, g: u8, b: u8) -> u8 {
    let y = 0.299 * r as f32 + 0.587 * g as f32 + 0.114 * b as f32;
    y.clamp(0.0, 255.0) as u8
}

fn p95_minus_p5(lumas: &mut [u8]) -> f32 {
    if lumas.is_empty() {
        return 0.0;
    }
    lumas.sort_unstable();
    let n = lumas.len();
    let p5 = lumas[(n as f32 * 0.05) as usize];
    let p95 = lumas[((n as f32 * 0.95) as usize).min(n - 1)];
    (p95 as f32 - p5 as f32).max(0.0)
}

/// sRGB EOTF: gamma-encoded 8-bit channel → linear-light float.
fn srgb_to_linear(c: f32) -> f32 {
    let cs = c / 255.0;
    if cs <= 0.04045 {
        cs / 12.92
    } else {
        ((cs + 0.055) / 1.055).powf(2.4)
    }
}

/// sRGB 8-bit → CIELAB. Returns `(L*, a*, b*)` with L in 0..100 and
/// a*/b* roughly in -128..127. D65 reference white throughout.
pub fn srgb_u8_to_lab(r: u8, g: u8, b: u8) -> (f32, f32, f32) {
    let rl = srgb_to_linear(r as f32);
    let gl = srgb_to_linear(g as f32);
    let bl = srgb_to_linear(b as f32);

    let x = 0.4124564 * rl + 0.3575761 * gl + 0.1804375 * bl;
    let y = 0.2126729 * rl + 0.7151522 * gl + 0.0721750 * bl;
    let z = 0.0193339 * rl + 0.1191920 * gl + 0.9503041 * bl;

    // D65 reference white.
    let xn = 0.95047_f32;
    let yn = 1.00000_f32;
    let zn = 1.08883_f32;

    let fx = lab_f(x / xn);
    let fy = lab_f(y / yn);
    let fz = lab_f(z / zn);

    let l = 116.0 * fy - 16.0;
    let a = 500.0 * (fx - fy);
    let bb = 200.0 * (fy - fz);
    (l, a, bb)
}

fn lab_f(t: f32) -> f32 {
    let delta = 6.0_f32 / 29.0;
    if t > delta.powi(3) {
        t.cbrt()
    } else {
        t / (3.0 * delta * delta) + 4.0 / 29.0
    }
}

/// Simple deltaE between two signatures. v1 treats `mean_luma`
/// (BT.601 Y, 0..255) as the L term rather than mean CIELAB L*. The
/// thresholds (25/12) are set against this combined value, not a
/// perceptual CIE76 unit, so the dimensions match in practice.
pub fn delta_e_cie76(a: &ColorGradeSignature, b: &ColorGradeSignature) -> f32 {
    let dl = a.mean_luma - b.mean_luma;
    let da = a.mean_a - b.mean_a;
    let db = a.mean_b - b.mean_b;
    (dl * dl + da * da + db * db).sqrt()
}

fn build_findings(
    shots_dir: &Path,
    signatures: &[(PathBuf, ColorGradeSignature)],
) -> Vec<LintFinding> {
    let mut over_threshold: Vec<(usize, usize, f32)> = Vec::new();
    for i in 0..signatures.len() {
        for j in (i + 1)..signatures.len() {
            let de = delta_e_cie76(&signatures[i].1, &signatures[j].1);
            if de > WARN_DELTA_E {
                over_threshold.push((i, j, de));
            }
        }
    }
    if over_threshold.is_empty() {
        return Vec::new();
    }

    let worst_de = over_threshold
        .iter()
        .map(|(_, _, d)| *d)
        .fold(0.0_f32, f32::max);
    let severity = if worst_de > ERROR_DELTA_E {
        Severity::Error
    } else {
        Severity::Warn
    };

    let mut over_sorted = over_threshold.clone();
    over_sorted.sort_by(|a, b| b.2.partial_cmp(&a.2).unwrap());
    let (wi, wj, _) = over_sorted[0];

    let mut message = format!("{} shots compared\n", signatures.len());
    for (path, sig) in signatures {
        let name = path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("<shot>");
        message.push_str(&format!(
            "         {} -> luma={}  a*={}  b*={}  contrast={}\n",
            name,
            sig.mean_luma.round() as i32,
            sig.mean_a.round() as i32,
            sig.mean_b.round() as i32,
            sig.contrast.round() as i32,
        ));
    }
    let threshold_label = if severity == Severity::Error {
        ERROR_DELTA_E
    } else {
        WARN_DELTA_E
    };
    message.push_str(&format!(
        "         worst pair: {} vs {}  deltaE={:.1} (threshold {})",
        short(&signatures[wi].0),
        short(&signatures[wj].0),
        over_sorted[0].2,
        threshold_label as i32,
    ));
    if over_sorted.len() > 1 {
        message.push_str("\n         additional pairs over threshold:");
        for (i, j, de) in over_sorted.iter().skip(1) {
            message.push_str(&format!(
                "\n           {} vs {}  deltaE={:.1}",
                short(&signatures[*i].0),
                short(&signatures[*j].0),
                de,
            ));
        }
    }

    let fix_hint = String::from(
        "the shot-N Veo prompts must repeat the same cinematography \
         vocabulary verbatim — same camera (\"35 mm anamorphic\"), \
         same lens character, same lighting key, same grade language \
         (\"A24-style\", \"amber tungsten key\", \"warm window light\"). \
         Today's prompts differ on lighting time-of-day or palette; \
         edit storyboard.json before re-rolling shots.",
    );

    vec![LintFinding {
        rule: RULE.to_string(),
        severity,
        scene_path: shots_dir.to_path_buf(),
        t_secs: 0.0,
        element_selector: String::from("shots/"),
        element_bbox: Rect { x: 0.0, y: 0.0, w: 0.0, h: 0.0 },
        message,
        fix_hint,
        subkind: None,
    }]
}

fn short(p: &Path) -> String {
    p.file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("<shot>")
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid(rgba: [u8; 4], w: u32, h: u32) -> Vec<u8> {
        rgba.iter().cycle().copied().take((w * h * 4) as usize).collect()
    }

    #[test]
    fn solid_gray_has_zero_contrast() {
        let f = solid([128, 128, 128, 255], 64, 64);
        let sig = signature_from_rgba(&f, 64, 64);
        assert!(sig.contrast.abs() < 1.0);
        assert!(sig.mean_a.abs() < 2.0);
        assert!(sig.mean_b.abs() < 2.0);
    }

    #[test]
    fn dark_vs_bright_solids_exceed_error_threshold() {
        let dark = solid([20, 20, 20, 255], 64, 64);
        let bright = solid([220, 220, 220, 255], 64, 64);
        let a = signature_from_rgba(&dark, 64, 64);
        let b = signature_from_rgba(&bright, 64, 64);
        let de = delta_e_cie76(&a, &b);
        assert!(de > ERROR_DELTA_E, "expected error-level drift, got {de}");
    }

    #[test]
    fn warm_vs_cool_casts_exceed_warn_threshold() {
        let warm = solid([200, 140, 80, 255], 64, 64);
        let cool = solid([80, 140, 200, 255], 64, 64);
        let a = signature_from_rgba(&warm, 64, 64);
        let b = signature_from_rgba(&cool, 64, 64);
        let de = delta_e_cie76(&a, &b);
        assert!(de > WARN_DELTA_E, "expected warn-level drift, got {de}");
    }

    #[test]
    fn discover_shots_dir_finds_sibling() {
        let dir = std::env::temp_dir().join("wavelet-lint-coherence-discover");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("shots")).unwrap();
        std::fs::write(dir.join("commercial.html"), "<!doctype html>").unwrap();
        std::fs::write(dir.join("shots").join("shot-1.mp4"), []).unwrap();
        let found = discover_shots_dir(&dir.join("commercial.html")).unwrap();
        assert_eq!(found, dir.join("shots"));
    }

    #[test]
    fn discover_shots_dir_accepts_direct_dir() {
        let dir = std::env::temp_dir().join("wavelet-lint-coherence-direct");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("shot-1.mp4"), []).unwrap();
        let found = discover_shots_dir(&dir).unwrap();
        assert_eq!(found, dir);
    }

    #[test]
    fn single_shot_short_circuits_info() {
        let dir = std::env::temp_dir().join("wavelet-lint-coherence-single");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("shots")).unwrap();
        std::fs::write(dir.join("commercial.html"), "<!doctype html>").unwrap();
        std::fs::write(dir.join("shots").join("shot-1.mp4"), []).unwrap();
        let outcome = run(&dir.join("commercial.html")).unwrap();
        assert_eq!(outcome.findings.len(), 1);
        assert_eq!(outcome.findings[0].severity, Severity::Info);
        assert!(outcome.findings[0]
            .message
            .contains("single shot"));
    }

    #[test]
    fn no_shots_at_all_short_circuits_info() {
        let dir = std::env::temp_dir().join("wavelet-lint-coherence-none");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let outcome = run(&dir).unwrap();
        assert_eq!(outcome.findings.len(), 1);
        assert_eq!(outcome.findings[0].severity, Severity::Info);
    }

    #[test]
    fn build_findings_emits_error_for_large_drift() {
        let sigs = vec![
            (
                PathBuf::from("shot-1.mp4"),
                ColorGradeSignature {
                    mean_luma: 60.0,
                    mean_a: 14.0,
                    mean_b: 22.0,
                    contrast: 180.0,
                },
            ),
            (
                PathBuf::from("shot-2.mp4"),
                ColorGradeSignature {
                    mean_luma: 210.0,
                    mean_a: -2.0,
                    mean_b: 4.0,
                    contrast: 120.0,
                },
            ),
        ];
        let findings = build_findings(Path::new("shots/"), &sigs);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].severity, Severity::Error);
        assert!(findings[0].message.contains("shot-1.mp4"));
        assert!(findings[0].message.contains("shot-2.mp4"));
        assert!(findings[0].message.contains("worst pair"));
    }

    #[test]
    fn build_findings_silent_when_within_threshold() {
        let sigs = vec![
            (
                PathBuf::from("shot-1.mp4"),
                ColorGradeSignature {
                    mean_luma: 120.0,
                    mean_a: 2.0,
                    mean_b: 4.0,
                    contrast: 140.0,
                },
            ),
            (
                PathBuf::from("shot-2.mp4"),
                ColorGradeSignature {
                    mean_luma: 124.0,
                    mean_a: 3.0,
                    mean_b: 5.0,
                    contrast: 138.0,
                },
            ),
        ];
        let findings = build_findings(Path::new("shots/"), &sigs);
        assert!(findings.is_empty());
    }

    #[test]
    fn build_findings_warn_when_between_thresholds() {
        let sigs = vec![
            (
                PathBuf::from("shot-1.mp4"),
                ColorGradeSignature {
                    mean_luma: 100.0,
                    mean_a: 4.0,
                    mean_b: 6.0,
                    contrast: 140.0,
                },
            ),
            (
                PathBuf::from("shot-2.mp4"),
                ColorGradeSignature {
                    mean_luma: 115.0,
                    mean_a: 7.0,
                    mean_b: 8.0,
                    contrast: 130.0,
                },
            ),
        ];
        let findings = build_findings(Path::new("shots/"), &sigs);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].severity, Severity::Warn);
    }
}
