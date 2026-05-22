//! Validate a velocity profile against a music track's detected BPM.
//!
//! Strategy: detect onsets in the audio, then for each anchor in the
//! profile measure the actual BPM in a ±window around the anchor's
//! time. Compare per-anchor and report a global verdict.

use crate::audio::DecodedAudio;
use crate::query::beat::detect_onsets_interleaved;
use crate::velocity::VelocityProfile;
use serde::{Deserialize, Serialize};
use std::path::Path;

/// One per-anchor finding.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationFinding {
    /// Anchor index in the profile.
    pub anchor_index: usize,
    /// Anchor time in seconds.
    pub t: f32,
    /// Proposed BPM at this anchor.
    pub proposed_bpm: f32,
    /// BPM measured from onset density in a window around the anchor.
    /// None when no onsets fell inside the window.
    pub detected_bpm: Option<f32>,
    /// Signed delta `detected - proposed`. None when no detection.
    pub delta_bpm: Option<f32>,
    /// True when `|delta| <= tolerance_bpm`.
    pub within_tolerance: bool,
}

/// Full validation report.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationReport {
    /// True when every anchor is within tolerance.
    pub ok: bool,
    /// Anchors compared.
    pub total: usize,
    /// Anchors within tolerance.
    pub aligned: usize,
    /// Largest absolute |delta_bpm| across all anchors.
    pub worst_delta_bpm: f32,
    /// Per-anchor findings.
    pub findings: Vec<ValidationFinding>,
    /// Tolerance applied (BPM).
    pub tolerance_bpm: f32,
    /// Window radius applied (seconds) — onsets within ±radius of an
    /// anchor's time were counted toward its detected BPM.
    pub window_radius_secs: f32,
    /// Total onsets detected across the audio.
    pub onset_count: usize,
    /// Onset times (milliseconds from start) detected in the music
    /// audio. Empty when no audio was inspected. Use these as
    /// snap-targets for title-card keyframes and shot boundaries —
    /// per Curious Refuge, "snap title cards to the loudest onset
    /// within ±1 frame".
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub detected_onsets_ms: Vec<u32>,
}

/// Validate `profile` against the audio at `audio_path`. Tolerance and
/// window-radius are caller-tunable; sensible defaults are 5 BPM and
/// 2.0s.
pub fn validate_against_bpm(
    profile: &VelocityProfile,
    audio_path: &Path,
    tolerance_bpm: f32,
    window_radius_secs: f32,
) -> Result<ValidationReport, String> {
    let audio = DecodedAudio::decode(audio_path).map_err(|e| format!("decode: {e}"))?;
    let onsets = detect_onsets_interleaved(&audio.samples, audio.sample_rate);

    let mut findings = Vec::with_capacity(profile.anchors.len());
    let mut aligned = 0usize;
    let mut worst = 0.0f32;

    for (i, anchor) in profile.anchors.iter().enumerate() {
        let center_ms = (anchor.t * 1000.0) as i64;
        let radius_ms = (window_radius_secs * 1000.0) as i64;
        let lo = (center_ms - radius_ms).max(0) as u32;
        let hi = (center_ms + radius_ms).max(0) as u32;
        let in_window = onsets.iter().filter(|&&m| m >= lo && m <= hi).count();

        let detected = if in_window >= 2 && hi > lo {
            // BPM = onsets / window_seconds × 60
            let secs = (hi - lo) as f32 / 1000.0;
            Some((in_window as f32 / secs) * 60.0)
        } else {
            None
        };

        let delta = detected.map(|d| d - anchor.bpm);
        let within = delta.map(|d| d.abs() <= tolerance_bpm).unwrap_or(false);
        if within {
            aligned += 1;
        }
        if let Some(d) = delta {
            let a = d.abs();
            if a > worst {
                worst = a;
            }
        }
        findings.push(ValidationFinding {
            anchor_index: i,
            t: anchor.t,
            proposed_bpm: anchor.bpm,
            detected_bpm: detected,
            delta_bpm: delta,
            within_tolerance: within,
        });
    }

    let total = findings.len();
    Ok(ValidationReport {
        ok: aligned == total && total > 0,
        total,
        aligned,
        worst_delta_bpm: worst,
        findings,
        tolerance_bpm,
        window_radius_secs,
        onset_count: onsets.len(),
        detected_onsets_ms: onsets,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::velocity::{Anchor, VelocityProfile};

    /// Round-trip the data shapes through serde. (Audio decode is
    /// covered by the wavelet audio tests; this just locks the wire format.)
    #[test]
    fn report_round_trips() {
        let r = ValidationReport {
            ok: true,
            total: 2,
            aligned: 2,
            worst_delta_bpm: 1.2,
            findings: vec![ValidationFinding {
                anchor_index: 0,
                t: 0.0,
                proposed_bpm: 90.0,
                detected_bpm: Some(91.0),
                delta_bpm: Some(1.0),
                within_tolerance: true,
            }],
            tolerance_bpm: 5.0,
            window_radius_secs: 2.0,
            onset_count: 50,
            detected_onsets_ms: vec![100, 350, 600],
        };
        let json = serde_json::to_string(&r).unwrap();
        let back: ValidationReport = serde_json::from_str(&json).unwrap();
        assert_eq!(back.total, 2);
        assert_eq!(back.detected_onsets_ms.len(), 3);
    }

    /// Empty `detected_onsets_ms` is skipped by serde and re-defaulted
    /// on parse — locks the wire-format optionality.
    #[test]
    fn empty_onsets_round_trips_via_default() {
        let r = ValidationReport {
            ok: true,
            total: 0,
            aligned: 0,
            worst_delta_bpm: 0.0,
            findings: vec![],
            tolerance_bpm: 5.0,
            window_radius_secs: 2.0,
            onset_count: 0,
            detected_onsets_ms: vec![],
        };
        let json = serde_json::to_string(&r).unwrap();
        assert!(!json.contains("detected_onsets_ms"), "got {json}");
        let back: ValidationReport = serde_json::from_str(&json).unwrap();
        assert!(back.detected_onsets_ms.is_empty());
    }

    #[test]
    fn profile_bpm_lookup_for_validator() {
        // Smoke test that the profile→anchor enumeration we rely on is
        // stable.
        let p = VelocityProfile {
            duration_secs: 10.0,
            mean_bpm: 0.0,
            anchors: vec![
                Anchor { t: 0.0, bpm: 90.0, label: None },
                Anchor { t: 5.0, bpm: 120.0, label: None },
            ],
        };
        assert_eq!(p.anchors.len(), 2);
        assert!((p.bpm_at(2.5) - 105.0).abs() < 0.1);
    }
}
