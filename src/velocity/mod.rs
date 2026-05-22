//! Velocity profile — piecewise-linear BPM curve over time.
//!
//! Drives cut frequency, transition aggression, motion intensity, and
//! BPM-aware music generation. See `docs/research/screenplay-to-mp4-prd.md` §2.
//!
//! The profile is a sorted list of `(t_secs, bpm)` anchors. Between
//! anchors BPM interpolates linearly. The agent (or a heuristic
//! proposer) writes the profile; downstream tools read it.

use serde::{Deserialize, Serialize};

pub mod edl;
pub mod propose;
pub mod validate;
pub mod render_curve;

pub use edl::{detect_onsets_ms, onsets_to_edl, parse_edl_record_ins_ms};
pub use propose::propose_from_screenplay;
pub use validate::{validate_against_bpm, ValidationReport, ValidationFinding};
pub use render_curve::render_curve_svg;

/// A piecewise-linear velocity profile.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VelocityProfile {
    /// Total duration of the profile, seconds.
    pub duration_secs: f32,
    /// Pre-computed mean BPM weighted by segment duration. Surfaced as a
    /// top-level field so agents can read the target tempo without
    /// having to integrate the anchor curve themselves. Producers set
    /// this; consumers can also call `mean_bpm()` to recompute.
    #[serde(default)]
    pub mean_bpm: f32,
    /// Anchors sorted by `t` ascending. At least two are required for a
    /// useful profile (start + end). `bpm_at` handles the degenerate
    /// 0/1-anchor cases by clamping.
    pub anchors: Vec<Anchor>,
}

/// One velocity keyframe.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Anchor {
    /// Time in seconds from the start of the composition.
    pub t: f32,
    /// Beats per minute target at this anchor.
    pub bpm: f32,
    /// Optional human label — what the agent is going for at this
    /// moment ("drop", "lull", "subject lands").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

impl VelocityProfile {
    /// Interpolated BPM at time `t_secs`. Clamped to the first/last
    /// anchor outside the profile range.
    pub fn bpm_at(&self, t_secs: f32) -> f32 {
        match self.anchors.as_slice() {
            [] => 0.0,
            [only] => only.bpm,
            anchors => {
                if t_secs <= anchors[0].t {
                    return anchors[0].bpm;
                }
                if t_secs >= anchors[anchors.len() - 1].t {
                    return anchors[anchors.len() - 1].bpm;
                }
                // Find the segment containing t.
                for pair in anchors.windows(2) {
                    let a = &pair[0];
                    let b = &pair[1];
                    if t_secs >= a.t && t_secs <= b.t {
                        let span = (b.t - a.t).max(f32::EPSILON);
                        let u = (t_secs - a.t) / span;
                        return a.bpm + u * (b.bpm - a.bpm);
                    }
                }
                anchors[anchors.len() - 1].bpm
            }
        }
    }

    /// Recompute the mean BPM directly from the anchors via duration-
    /// weighted trapezoidal integration. Producers call this after
    /// mutating anchors so the `mean_bpm` field stays in sync.
    pub fn compute_mean_bpm(&self) -> f32 {
        if self.anchors.len() < 2 {
            return self.anchors.first().map(|a| a.bpm).unwrap_or(0.0);
        }
        let mut sum = 0.0;
        let mut span = 0.0;
        for pair in self.anchors.windows(2) {
            let a = &pair[0];
            let b = &pair[1];
            let dt = b.t - a.t;
            sum += 0.5 * (a.bpm + b.bpm) * dt;
            span += dt;
        }
        if span > 0.0 {
            sum / span
        } else {
            self.anchors[0].bpm
        }
    }

    /// Refresh the stored `mean_bpm` field from the current anchors.
    /// Call after editing anchors so consumers reading the JSON output
    /// see the right value.
    pub fn refresh_mean_bpm(&mut self) {
        self.mean_bpm = self.compute_mean_bpm();
    }

    /// Sample the profile at uniform intervals — useful for plotting or
    /// for music-gen backends that want a fixed-rate BPM signal.
    pub fn sample(&self, samples: usize) -> Vec<(f32, f32)> {
        if samples == 0 || self.duration_secs <= 0.0 {
            return Vec::new();
        }
        let mut out = Vec::with_capacity(samples);
        for i in 0..samples {
            let t = if samples == 1 {
                0.0
            } else {
                (i as f32 / (samples - 1) as f32) * self.duration_secs
            };
            out.push((t, self.bpm_at(t)));
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn profile() -> VelocityProfile {
        VelocityProfile {
            duration_secs: 10.0,
            mean_bpm: 0.0,
            anchors: vec![
                Anchor { t: 0.0, bpm: 60.0, label: None },
                Anchor { t: 5.0, bpm: 120.0, label: None },
                Anchor { t: 10.0, bpm: 90.0, label: None },
            ],
        }
    }

    #[test]
    fn bpm_at_clamps_outside() {
        let p = profile();
        assert_eq!(p.bpm_at(-1.0), 60.0);
        assert_eq!(p.bpm_at(20.0), 90.0);
    }

    #[test]
    fn bpm_at_interpolates() {
        let p = profile();
        assert!((p.bpm_at(2.5) - 90.0).abs() < 0.01);
        assert!((p.bpm_at(7.5) - 105.0).abs() < 0.01);
    }

    #[test]
    fn mean_bpm_weighted() {
        let p = profile();
        // Trapezoid: (60+120)/2 * 5 + (120+90)/2 * 5 = 450 + 525 = 975 over 10 = 97.5
        assert!((p.compute_mean_bpm() - 97.5).abs() < 0.01);
    }

    #[test]
    fn sample_endpoints() {
        let p = profile();
        let samples = p.sample(11);
        assert_eq!(samples.len(), 11);
        assert!((samples[0].1 - 60.0).abs() < 0.01);
        assert!((samples[10].1 - 90.0).abs() < 0.01);
    }
}
