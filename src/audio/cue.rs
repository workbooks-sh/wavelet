//! [`AudioCue`] — one schedule entry for the audio mixer.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// One audio cue scheduled on the wavelet timeline.
///
/// Mirrors `gamut_ir::AudioCue` but adds the resolved `asset_path` and a
/// stable `id` for cross-cue ducking references.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AudioCue {
    /// Stable cue id, referenced by other cues' `duck_targets`.
    pub id: String,

    /// Resolved path to the audio file. Decoded by symphonia.
    pub asset_path: PathBuf,

    /// Composition output frame where the cue starts.
    pub start_frame: u64,

    /// How many composition frames the cue plays for.
    pub duration_frames: u64,

    /// Linear volume multiplier. 1.0 = unchanged. Clipped at mixer output.
    pub volume: f32,

    /// Stereo pan. -1.0 = full left, 0 = center, 1.0 = full right.
    pub pan: f32,

    /// Fade-in ramp in output frames. 0 = no fade.
    pub fade_in_frames: u64,

    /// Fade-out ramp in output frames. 0 = no fade.
    pub fade_out_frames: u64,

    /// Ids of OTHER cues to duck while this cue is active.
    pub duck_targets: Vec<String>,

    /// How many dB to reduce ducked cues by. Typical: 12.0.
    pub duck_db: f32,

    /// When true, the mixer snaps this cue's `start_frame` to the
    /// nearest detected onset of the `music`-tagged cue (within a
    /// ±0.3s window). Off by default — set per-cue for narration that
    /// should land on a drum hit.
    #[serde(default, skip_serializing_if = "is_false")]
    pub align_to_beat: bool,
}

fn is_false(b: &bool) -> bool {
    !*b
}

impl AudioCue {
    /// Compute the linear amplitude multiplier for this cue at a given output
    /// frame number. Combines fade-in + fade-out + base volume. Returns 0.0
    /// when the cue isn't active at this frame.
    pub fn envelope_at(&self, frame: u64) -> f32 {
        if frame < self.start_frame {
            return 0.0;
        }
        let local = frame - self.start_frame;
        if local >= self.duration_frames {
            return 0.0;
        }

        let mut amp = self.volume;

        if self.fade_in_frames > 0 && local < self.fade_in_frames {
            amp *= local as f32 / self.fade_in_frames as f32;
        }

        let frames_from_end = self.duration_frames - local;
        if self.fade_out_frames > 0 && frames_from_end <= self.fade_out_frames {
            amp *= frames_from_end as f32 / self.fade_out_frames as f32;
        }

        amp
    }

    /// Whether this cue is producing audio (envelope > 0) at the given frame.
    pub fn is_active_at(&self, frame: u64) -> bool {
        frame >= self.start_frame && frame < self.start_frame + self.duration_frames
    }

    /// Left/right channel gains from this cue's pan. Equal-power pan law —
    /// constant perceived loudness across the stereo field.
    pub fn pan_gains(&self) -> (f32, f32) {
        let p = self.pan.clamp(-1.0, 1.0);
        // Map -1..1 → 0..π/2
        let angle = (p + 1.0) * std::f32::consts::FRAC_PI_4;
        (angle.cos(), angle.sin())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn mk_cue(start: u64, dur: u64) -> AudioCue {
        AudioCue {
            id: "test".into(),
            asset_path: PathBuf::from("x"),
            start_frame: start,
            duration_frames: dur,
            volume: 1.0,
            pan: 0.0,
            fade_in_frames: 0,
            fade_out_frames: 0,
            duck_targets: vec![],
            duck_db: 0.0,
            align_to_beat: false,
        }
    }

    #[test]
    fn serde_round_trip_align_to_beat_false_is_omitted() {
        let cue = mk_cue(0, 10);
        let json = serde_json::to_string(&cue).unwrap();
        assert!(
            !json.contains("align_to_beat"),
            "align_to_beat=false should be omitted via skip_serializing_if; got: {json}",
        );
        let back: AudioCue = serde_json::from_str(&json).unwrap();
        assert!(!back.align_to_beat);
    }

    #[test]
    fn serde_round_trip_align_to_beat_true_is_emitted() {
        let mut cue = mk_cue(0, 10);
        cue.align_to_beat = true;
        let json = serde_json::to_string(&cue).unwrap();
        assert!(
            json.contains("\"align_to_beat\":true"),
            "expected align_to_beat=true to round-trip; got: {json}",
        );
        let back: AudioCue = serde_json::from_str(&json).unwrap();
        assert!(back.align_to_beat);
    }

    #[test]
    fn serde_default_align_to_beat_when_missing() {
        let json = r#"{
            "id": "t", "asset_path": "x",
            "start_frame": 0, "duration_frames": 10,
            "volume": 1.0, "pan": 0.0,
            "fade_in_frames": 0, "fade_out_frames": 0,
            "duck_targets": [], "duck_db": 0.0
        }"#;
        let cue: AudioCue = serde_json::from_str(json).unwrap();
        assert!(!cue.align_to_beat);
    }

    #[test]
    fn envelope_outside_cue_window_is_zero() {
        let cue = mk_cue(10, 5);
        assert_eq!(cue.envelope_at(9), 0.0);
        assert_eq!(cue.envelope_at(15), 0.0);
        assert_eq!(cue.envelope_at(100), 0.0);
    }

    #[test]
    fn envelope_inside_window_at_base_volume() {
        let mut cue = mk_cue(0, 10);
        cue.volume = 0.5;
        assert_eq!(cue.envelope_at(0), 0.5);
        assert_eq!(cue.envelope_at(5), 0.5);
        assert_eq!(cue.envelope_at(9), 0.5);
    }

    #[test]
    fn fade_in_ramps_from_zero() {
        let mut cue = mk_cue(0, 10);
        cue.fade_in_frames = 5;
        assert_eq!(cue.envelope_at(0), 0.0);
        assert_eq!(cue.envelope_at(1), 0.2);
        assert_eq!(cue.envelope_at(2), 0.4);
        assert_eq!(cue.envelope_at(4), 0.8);
        assert_eq!(cue.envelope_at(5), 1.0);
    }

    #[test]
    fn fade_out_ramps_to_zero() {
        let mut cue = mk_cue(0, 10);
        cue.fade_out_frames = 5;
        // last 5 frames (5..10) ramp from 1.0 down to 0.2 (at frame 9, 1 frame from end)
        assert!((cue.envelope_at(5) - 1.0).abs() < 1e-5);
        assert!((cue.envelope_at(7) - 0.6).abs() < 1e-5); // 3 frames from end
        assert!((cue.envelope_at(9) - 0.2).abs() < 1e-5); // 1 frame from end
    }

    #[test]
    fn fade_in_and_out_both_apply() {
        let mut cue = mk_cue(0, 10);
        cue.fade_in_frames = 3;
        cue.fade_out_frames = 3;
        // Frame 1: fade in at 1/3, no fade-out active
        assert!((cue.envelope_at(1) - 1.0 / 3.0).abs() < 1e-5);
        // Frame 8: fade-out active (2 frames from end), no fade-in
        assert!((cue.envelope_at(8) - 2.0 / 3.0).abs() < 1e-5);
    }

    #[test]
    fn pan_center_equal_gains() {
        let cue = mk_cue(0, 10);
        let (l, r) = cue.pan_gains();
        assert!((l - r).abs() < 1e-5);
        // Both should be ~0.707 (equal-power center)
        assert!((l - 0.707).abs() < 0.01);
    }

    #[test]
    fn pan_full_left_quiets_right() {
        let mut cue = mk_cue(0, 10);
        cue.pan = -1.0;
        let (l, r) = cue.pan_gains();
        assert!((l - 1.0).abs() < 1e-5);
        assert!(r.abs() < 1e-5);
    }
}
