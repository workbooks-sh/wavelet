//! Detect leading / trailing static frames in an AI-generated MP4 and
//! compute a trim range that keeps only the motion-bearing portion.
//!
//! Why: Veo (and similar txt2vid models) often emit clips that start
//! with ~0.5–1.5 s of frozen frames before the action begins, and
//! sometimes a freeze at the tail too. Compositing those frames into
//! the final cut wastes screen time and reads as a stutter at the cut
//! points.
//!
//! How: shell out to ffmpeg's `freezedetect` filter, which is the
//! built-in primitive for this. It reports `freeze_start` /
//! `freeze_duration` / `freeze_end` markers on stderr. We parse those,
//! derive a `(start_s, end_s)` keep-range, and either emit the JSON
//! report or stream-copy the trimmed clip to a new file (lossless —
//! no re-encode).
//!
//! Tuning:
//! - `n=-60dB` matches ffmpeg's default noise floor (very generous —
//!   true freezes only). Bumping to `-50dB` catches near-freezes (the
//!   "almost still" frames Veo sometimes emits where the model is
//!   updating one pixel of noise per frame but visually static).
//! - `d=0.4` requires a freeze to last ≥ 0.4 s to count — short enough
//!   to catch the leading freeze on a 5 s clip, long enough to not
//!   chop the natural pause between two motion beats inside a clip.

use serde::{Deserialize, Serialize};
use std::path::Path;
use std::process::{Command, Stdio};

/// Default freezedetect noise threshold in dB. Negative values; lower
/// is stricter (only catches true frozen frames). `-60` is ffmpeg's
/// default; `-50` is a useful "near-freeze" alternative.
pub const DEFAULT_NOISE_DB: f32 = -60.0;

/// Default minimum freeze duration in seconds. Below this, the
/// detector ignores the freeze — keeps natural pauses in motion.
pub const DEFAULT_MIN_FREEZE_SECS: f32 = 0.4;

/// Smallest motion run worth keeping. If the detected motion span is
/// shorter than this, the clip is effectively all-static and we
/// refuse to trim (the agent should re-roll, not ship a 0.2 s clip).
pub const MIN_MOTION_SECS: f32 = 1.0;

/// Result of analyzing one clip.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrimReport {
    /// Total duration of the input clip (s).
    pub input_duration_s: f32,
    /// Detected leading freeze in seconds — 0 if none.
    pub leading_freeze_s: f32,
    /// Detected trailing freeze in seconds — 0 if none.
    pub trailing_freeze_s: f32,
    /// Recommended trim-start in seconds (offset from clip start).
    pub trim_start_s: f32,
    /// Recommended trim-end in seconds (offset from clip start).
    pub trim_end_s: f32,
    /// Duration of the kept motion span (trim_end - trim_start).
    pub motion_duration_s: f32,
    /// True when the motion span is too short to use — caller should
    /// re-roll the clip rather than ship a degenerate trim.
    pub unusable: bool,
    /// Freeze events detected (start_s, duration_s), in clip-time.
    pub events: Vec<FreezeEvent>,
}

/// One freeze event reported by ffmpeg `freezedetect`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FreezeEvent {
    /// Start of the freeze in clip-time (s).
    pub start_s: f32,
    /// Duration of the freeze (s).
    pub duration_s: f32,
}

/// Detection parameters.
#[derive(Debug, Clone, Copy)]
pub struct DetectParams {
    /// Noise threshold in dB (negative). See `DEFAULT_NOISE_DB`.
    pub noise_db: f32,
    /// Minimum freeze duration to count (s). See `DEFAULT_MIN_FREEZE_SECS`.
    pub min_freeze_secs: f32,
}

impl Default for DetectParams {
    fn default() -> Self {
        Self {
            noise_db: DEFAULT_NOISE_DB,
            min_freeze_secs: DEFAULT_MIN_FREEZE_SECS,
        }
    }
}

/// Probe a clip and return the trim recommendation. Shells out to
/// `ffmpeg` + `ffprobe`. Returns `Err` on missing binary or unparseable
/// output — caller should treat that as "don't trim" rather than fail.
pub fn analyze(input: &Path, params: DetectParams) -> Result<TrimReport, String> {
    let duration = probe_duration(input)
        .map_err(|e| format!("ffprobe duration: {e}"))?;
    let events = detect_freezes(input, params)?;
    let (leading, trailing) = classify(&events, duration);
    let trim_start = leading;
    let trim_end = (duration - trailing).max(trim_start);
    let motion = (trim_end - trim_start).max(0.0);
    let unusable = motion < MIN_MOTION_SECS;
    Ok(TrimReport {
        input_duration_s: duration,
        leading_freeze_s: leading,
        trailing_freeze_s: trailing,
        trim_start_s: trim_start,
        trim_end_s: trim_end,
        motion_duration_s: motion,
        unusable,
        events,
    })
}

/// Lossless stream-copy trim. Cuts `input` to `[trim_start_s, trim_end_s]`
/// and writes the result to `output`. Uses `-c copy` so no re-encode
/// happens — fast and bit-identical for the kept range.
///
/// Returns `Err` on ffmpeg failure. Caller should already have a
/// `TrimReport` and have decided the trim is worth applying (not
/// `unusable`, not a no-op).
pub fn apply_trim(
    input: &Path,
    output: &Path,
    trim_start_s: f32,
    trim_end_s: f32,
) -> Result<(), String> {
    let status = Command::new("ffmpeg")
        .args([
            "-nostdin",
            "-hide_banner",
            "-loglevel",
            "error",
            "-y",
            "-ss",
            &format!("{trim_start_s:.3}"),
            "-to",
            &format!("{trim_end_s:.3}"),
            "-i",
        ])
        .arg(input)
        .args(["-c", "copy", "-avoid_negative_ts", "make_zero"])
        .arg(output)
        .status()
        .map_err(|e| format!("spawn ffmpeg: {e}"))?;
    if !status.success() {
        return Err(format!("ffmpeg trim exited {status}"));
    }
    Ok(())
}

fn probe_duration(input: &Path) -> Result<f32, String> {
    let out = Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-show_entries",
            "format=duration",
            "-of",
            "default=noprint_wrappers=1:nokey=1",
        ])
        .arg(input)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .map_err(|e| format!("spawn ffprobe: {e}"))?;
    if !out.status.success() {
        return Err(format!("ffprobe exited {}", out.status));
    }
    std::str::from_utf8(&out.stdout)
        .map_err(|e| e.to_string())?
        .trim()
        .parse::<f32>()
        .map_err(|e| format!("parse duration: {e}"))
}

fn detect_freezes(input: &Path, params: DetectParams) -> Result<Vec<FreezeEvent>, String> {
    // freezedetect emits markers on stderr via `metadata=print`. We
    // run with `-f null -` so no output file is written — we only
    // care about the stderr log.
    let filter = format!(
        "freezedetect=n={:.1}dB:d={:.3}",
        params.noise_db, params.min_freeze_secs
    );
    let out = Command::new("ffmpeg")
        .args([
            "-nostdin",
            "-hide_banner",
            "-loglevel",
            "info",
            "-i",
        ])
        .arg(input)
        .args(["-vf", &filter, "-an", "-f", "null", "-"])
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .map_err(|e| format!("spawn ffmpeg freezedetect: {e}"))?;
    if !out.status.success() {
        return Err(format!("ffmpeg freezedetect exited {}", out.status));
    }
    let stderr = String::from_utf8_lossy(&out.stderr);
    Ok(parse_freezedetect_log(&stderr))
}

/// Parse `freezedetect` stderr log into a list of freeze events.
/// Public so unit tests can exercise the parser without ffmpeg.
pub fn parse_freezedetect_log(log: &str) -> Vec<FreezeEvent> {
    // freezedetect emits lines like:
    //   [freezedetect @ 0x...] lavfi.freezedetect.freeze_start: 0.000000
    //   [freezedetect @ 0x...] lavfi.freezedetect.freeze_duration: 1.234000
    //   [freezedetect @ 0x...] lavfi.freezedetect.freeze_end: 1.234000
    // Events arrive grouped — we pair start+duration as we see them.
    let mut events = Vec::new();
    let mut current_start: Option<f32> = None;
    for line in log.lines() {
        if let Some(rest) = line.find("freeze_start:").map(|i| &line[i + "freeze_start:".len()..]) {
            if let Ok(v) = rest.trim().parse::<f32>() {
                current_start = Some(v);
            }
        } else if let Some(rest) =
            line.find("freeze_duration:").map(|i| &line[i + "freeze_duration:".len()..])
        {
            if let Ok(dur) = rest.trim().parse::<f32>() {
                if let Some(start) = current_start.take() {
                    events.push(FreezeEvent { start_s: start, duration_s: dur });
                }
            }
        }
    }
    events
}

/// Classify the detected freezes into leading and trailing slices.
/// Anything that abuts the start of the clip (within 0.05 s) counts
/// as leading; anything whose end abuts the clip end counts as
/// trailing. Freezes in the middle are kept (those might be authored
/// holds the agent wants).
fn classify(events: &[FreezeEvent], duration: f32) -> (f32, f32) {
    let mut leading = 0.0f32;
    let mut trailing = 0.0f32;
    for e in events {
        if e.start_s <= 0.05 {
            // Leading freeze. Keep the largest leading event (in
            // case the filter splits a single freeze into multiple
            // adjacent reports).
            leading = leading.max(e.start_s + e.duration_s);
        } else if (e.start_s + e.duration_s) >= duration - 0.05 {
            // Trailing freeze.
            trailing = trailing.max(duration - e.start_s);
        }
    }
    (leading, trailing)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_one_leading_freeze() {
        let log = r#"
[freezedetect @ 0x600003c98000] lavfi.freezedetect.freeze_start: 0.000000
[freezedetect @ 0x600003c98000] lavfi.freezedetect.freeze_duration: 1.250000
[freezedetect @ 0x600003c98000] lavfi.freezedetect.freeze_end: 1.250000
"#;
        let events = parse_freezedetect_log(log);
        assert_eq!(events.len(), 1);
        assert!((events[0].start_s - 0.0).abs() < 1e-6);
        assert!((events[0].duration_s - 1.25).abs() < 1e-6);
    }

    #[test]
    fn parses_leading_plus_trailing() {
        let log = r#"
[freezedetect @ 0x1] lavfi.freezedetect.freeze_start: 0.000000
[freezedetect @ 0x1] lavfi.freezedetect.freeze_duration: 0.800000
[freezedetect @ 0x1] lavfi.freezedetect.freeze_end: 0.800000
[freezedetect @ 0x1] lavfi.freezedetect.freeze_start: 4.500000
[freezedetect @ 0x1] lavfi.freezedetect.freeze_duration: 0.500000
[freezedetect @ 0x1] lavfi.freezedetect.freeze_end: 5.000000
"#;
        let events = parse_freezedetect_log(log);
        assert_eq!(events.len(), 2);
        assert!((events[1].start_s - 4.5).abs() < 1e-6);
        assert!((events[1].duration_s - 0.5).abs() < 1e-6);
    }

    #[test]
    fn classify_isolates_leading_and_trailing() {
        let events = vec![
            FreezeEvent { start_s: 0.0, duration_s: 1.2 }, // leading
            FreezeEvent { start_s: 2.3, duration_s: 0.4 }, // mid — kept
            FreezeEvent { start_s: 4.5, duration_s: 0.5 }, // trailing
        ];
        let (leading, trailing) = classify(&events, 5.0);
        assert!((leading - 1.2).abs() < 1e-6);
        assert!((trailing - 0.5).abs() < 1e-6);
    }

    #[test]
    fn classify_handles_no_freezes() {
        let (leading, trailing) = classify(&[], 5.0);
        assert!((leading - 0.0).abs() < 1e-6);
        assert!((trailing - 0.0).abs() < 1e-6);
    }

    #[test]
    fn classify_collapses_back_to_back_leading_events() {
        // Sometimes freezedetect splits a single freeze into adjacent
        // reports when the noise threshold flutters. The longest-end
        // event wins.
        let events = vec![
            FreezeEvent { start_s: 0.0, duration_s: 0.5 },
            FreezeEvent { start_s: 0.0, duration_s: 1.1 },
        ];
        let (leading, trailing) = classify(&events, 5.0);
        assert!((leading - 1.1).abs() < 1e-6);
        assert_eq!(trailing, 0.0);
    }
}
