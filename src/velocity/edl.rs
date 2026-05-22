//! Edit Decision List emitter — turns detected music onsets into cut
//! markers that Final Cut Pro 7 / DaVinci Resolve / Premiere can
//! ingest. The "credible center" workflow per Curious Refuge: snap
//! title-card keyframes and shot boundaries to the nearest musical
//! onset within ±1 frame.
//!
//! EDL flavor is FCP7 / Resolve NON-DROP. Each row:
//!
//! ```text
//! 001  AX       V     C        00:00:00:00 00:00:01:15 00:00:00:00 00:00:01:15
//! ```
//!
//! Fields: cut number, reel (always `AX` — auxiliary), track (`V` —
//! video), edit type (`C` — straight cut), source-in, source-out,
//! record-in, record-out. Source = record (no clip retiming; the EDL
//! is a list of cut points on the timeline, not a conform).

use crate::audio::DecodedAudio;
use crate::query::beat::detect_onsets_interleaved;
use std::path::Path;

/// Format the timecode `HH:MM:SS:FF` for a given onset time in ms and
/// frame rate. Frames round half-to-even to avoid the +1/-1 jitter at
/// exact frame boundaries. Hour rolls over past 24:00 — Resolve
/// tolerates this, but we keep it within `u32` ms range either way.
pub fn format_timecode(ms: u32, fps: u32) -> String {
    let total_frames = ((ms as u64 * fps as u64) + 500) / 1000;
    let frames = (total_frames % fps as u64) as u32;
    let total_secs = total_frames / fps as u64;
    let secs = (total_secs % 60) as u32;
    let mins = ((total_secs / 60) % 60) as u32;
    let hours = (total_secs / 3600) as u32;
    format!("{:02}:{:02}:{:02}:{:02}", hours, mins, secs, frames)
}

/// Emit a Final Cut Pro 7 / Resolve EDL from a list of onset times.
///
/// Each consecutive pair of onsets becomes one cut row — onset N is
/// the record-in of cut N+1. The very first cut runs from 00:00:00:00
/// to the first onset; the last cut runs from the final onset to a
/// sentinel `duration_ms` (or the final onset if not provided, which
/// makes that row degenerate but harmless).
pub fn onsets_to_edl(
    onsets_ms: &[u32],
    fps: u32,
    title: &str,
    duration_ms: Option<u32>,
) -> String {
    let mut out = String::new();
    out.push_str(&format!("TITLE: {}\n", title));
    out.push_str("FCM: NON-DROP FRAME\n\n");

    let mut boundaries: Vec<u32> = Vec::with_capacity(onsets_ms.len() + 2);
    boundaries.push(0);
    for &o in onsets_ms {
        if Some(&o) != boundaries.last() {
            boundaries.push(o);
        }
    }
    if let Some(d) = duration_ms {
        if Some(&d) != boundaries.last() && d > *boundaries.last().unwrap_or(&0) {
            boundaries.push(d);
        }
    }

    for (i, pair) in boundaries.windows(2).enumerate() {
        let in_tc = format_timecode(pair[0], fps);
        let out_tc = format_timecode(pair[1], fps);
        out.push_str(&format!(
            "{:03}  AX       V     C        {} {} {} {}\n",
            i + 1,
            in_tc,
            out_tc,
            in_tc,
            out_tc,
        ));
    }
    out
}

/// Decode an audio file and detect onsets — same path the validator
/// uses, surfaced for the standalone `onsets-to-edl` CLI verb.
pub fn detect_onsets_ms(audio_path: &Path) -> Result<Vec<u32>, String> {
    let audio = DecodedAudio::decode(audio_path).map_err(|e| format!("decode: {e}"))?;
    Ok(detect_onsets_interleaved(&audio.samples, audio.sample_rate))
}

/// Parse an EDL produced by `onsets_to_edl` back into the record-in
/// timestamps (ms). Used by the round-trip test and by tooling that
/// wants to re-ingest a hand-edited EDL.
pub fn parse_edl_record_ins_ms(edl: &str, fps: u32) -> Vec<u32> {
    let mut out = Vec::new();
    for line in edl.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with("TITLE") || line.starts_with("FCM") {
            continue;
        }
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() < 8 {
            continue;
        }
        let record_in = parts[6];
        if let Some(ms) = parse_timecode_ms(record_in, fps) {
            out.push(ms);
        }
    }
    out
}

fn parse_timecode_ms(tc: &str, fps: u32) -> Option<u32> {
    let parts: Vec<&str> = tc.split(':').collect();
    if parts.len() != 4 {
        return None;
    }
    let h: u64 = parts[0].parse().ok()?;
    let m: u64 = parts[1].parse().ok()?;
    let s: u64 = parts[2].parse().ok()?;
    let f: u64 = parts[3].parse().ok()?;
    let total_frames = ((h * 3600) + (m * 60) + s) * fps as u64 + f;
    let ms = (total_frames * 1000 + (fps as u64 / 2)) / fps as u64;
    Some(ms as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timecode_at_30fps() {
        assert_eq!(format_timecode(0, 30), "00:00:00:00");
        assert_eq!(format_timecode(1000, 30), "00:00:01:00");
        assert_eq!(format_timecode(1500, 30), "00:00:01:15");
        assert_eq!(format_timecode(60_000, 30), "00:01:00:00");
    }

    #[test]
    fn timecode_at_24fps() {
        assert_eq!(format_timecode(1000, 24), "00:00:01:00");
        // 500ms × 24fps = 12 frames.
        assert_eq!(format_timecode(500, 24), "00:00:00:12");
    }

    #[test]
    fn timecode_hour_rollover() {
        let one_hour_ms = 3600 * 1000;
        assert_eq!(format_timecode(one_hour_ms, 30), "01:00:00:00");
        assert_eq!(format_timecode(one_hour_ms + 1500, 30), "01:00:01:15");
    }

    #[test]
    fn three_onsets_yield_three_cuts_at_30fps() {
        // Onsets at 0.5s, 1.5s, 2.5s → boundaries [0, 500, 1500, 2500]
        // → 3 cuts.
        let edl = onsets_to_edl(&[500, 1500, 2500], 30, "test", None);
        let cuts: Vec<&str> = edl.lines().filter(|l| l.starts_with("00")).collect();
        assert_eq!(cuts.len(), 3, "got EDL:\n{edl}");
        assert!(edl.contains("00:00:00:00 00:00:00:15"));
        assert!(edl.contains("00:00:00:15 00:00:01:15"));
        assert!(edl.contains("00:00:01:15 00:00:02:15"));
    }

    #[test]
    fn three_onsets_yield_three_cuts_at_24fps() {
        let edl = onsets_to_edl(&[500, 1500, 2500], 24, "test", None);
        let cuts: Vec<&str> = edl.lines().filter(|l| l.starts_with("00")).collect();
        assert_eq!(cuts.len(), 3);
        // 500ms × 24fps = 12 frames.
        assert!(edl.contains("00:00:00:12"));
    }

    #[test]
    fn duration_appends_final_cut() {
        let edl = onsets_to_edl(&[1000, 2000], 30, "test", Some(5000));
        let cuts: Vec<&str> = edl.lines().filter(|l| l.starts_with("00")).collect();
        // Boundaries [0, 1000, 2000, 5000] → 3 cuts.
        assert_eq!(cuts.len(), 3);
        assert!(edl.contains("00:00:05:00"));
    }

    #[test]
    fn round_trips_record_ins() {
        let original = vec![500u32, 1500, 2500, 4000];
        let edl = onsets_to_edl(&original, 30, "rt", None);
        let parsed = parse_edl_record_ins_ms(&edl, 30);
        // parsed yields the record-in of each row: 0, then each onset.
        assert_eq!(parsed, vec![0, 500, 1500, 2500]);
    }

    #[test]
    fn header_includes_title_and_non_drop() {
        let edl = onsets_to_edl(&[100], 30, "wavelet-cuts", None);
        assert!(edl.starts_with("TITLE: wavelet-cuts\n"));
        assert!(edl.contains("FCM: NON-DROP FRAME"));
    }

    #[test]
    fn empty_onsets_emits_header_only() {
        let edl = onsets_to_edl(&[], 30, "empty", None);
        assert!(edl.contains("TITLE: empty"));
        let cuts: Vec<&str> = edl.lines().filter(|l| l.starts_with("00")).collect();
        assert!(cuts.is_empty());
    }
}
