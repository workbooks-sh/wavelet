//! [`AudioMixer`] — sum decoded + resampled cues into a final stereo buffer.
//!
//! Per-cue envelope: volume × fade-in × fade-out × pan.
//! Cross-cue ducking: while cue A is active, all cues in `A.duck_targets`
//! get scaled by `10^(-A.duck_db/20)` per sample.

use super::cue::AudioCue;
use super::decoder::DecodedAudio;
use super::errors::AudioError;
use super::resample::resample_stereo;
use std::collections::HashMap;

/// Tolerance window for snapping a `align_to_beat` cue's start frame
/// to a music onset. Matches the storyboard planner's tolerance so the
/// two layers stay in step.
const BEAT_ALIGN_TOLERANCE_SECS: f32 = 0.3;

/// Multi-cue audio mixer.
///
/// Renders a stereo f32 buffer for a composition. Cues are loaded once (with
/// implicit resample to project rate), then mixed once per render via
/// [`AudioMixer::render`].
pub struct AudioMixer {
    sample_rate: u32,
    fps: u32,
    cues: Vec<LoadedCue>,
}

struct LoadedCue {
    cue: AudioCue,
    /// Decoded samples in project sample-rate, interleaved stereo.
    samples: Vec<f32>,
}

impl AudioMixer {
    /// Create a new mixer for the given output rate + framerate.
    /// `sample_rate` is the audio rate (e.g. 48_000). `fps` is the video rate.
    pub fn new(sample_rate: u32, fps: u32) -> Self {
        Self {
            sample_rate,
            fps,
            cues: Vec::new(),
        }
    }

    /// Add a cue. Decodes the asset file + resamples to project rate.
    pub fn add_cue(&mut self, mut cue: AudioCue) -> Result<(), AudioError> {
        if self.cues.iter().any(|c| c.cue.id == cue.id) {
            return Err(AudioError::DuplicateCue(cue.id.clone()));
        }
        let decoded = DecodedAudio::decode(&cue.asset_path)?;
        let samples = resample_stereo(&decoded.samples, decoded.sample_rate, self.sample_rate)?;
        if cue.align_to_beat {
            self.snap_cue_to_music_onset(&mut cue);
        }
        self.cues.push(LoadedCue { cue, samples });
        Ok(())
    }

    /// Add a cue with pre-decoded samples — useful for tests + advanced callers
    /// that want to control the decode path themselves.
    pub fn add_cue_with_samples(
        &mut self,
        mut cue: AudioCue,
        samples: Vec<f32>,
        source_rate: u32,
    ) -> Result<(), AudioError> {
        if self.cues.iter().any(|c| c.cue.id == cue.id) {
            return Err(AudioError::DuplicateCue(cue.id.clone()));
        }
        let resampled = resample_stereo(&samples, source_rate, self.sample_rate)?;
        if cue.align_to_beat {
            self.snap_cue_to_music_onset(&mut cue);
        }
        self.cues.push(LoadedCue { cue, samples: resampled });
        Ok(())
    }

    /// If a music cue is loaded, snap `cue.start_frame` to its nearest
    /// detected onset within [`BEAT_ALIGN_TOLERANCE_SECS`]. No-op when
    /// no music cue is present or no onset is close enough.
    ///
    /// Music identification: cue id == "music" (preferred) or any cue
    /// id containing the substring "music" (case-insensitive).
    fn snap_cue_to_music_onset(&self, cue: &mut AudioCue) {
        let Some(music) = self.find_music_cue() else { return };
        let onsets_ms = crate::query::beat::detect_onsets_interleaved(
            &music.samples,
            self.sample_rate,
        );
        if onsets_ms.is_empty() {
            return;
        }
        let cue_start_ms =
            (cue.start_frame as f64 * 1000.0 / self.fps as f64) as i64;
        let music_start_ms =
            (music.cue.start_frame as f64 * 1000.0 / self.fps as f64) as i64;
        let tolerance_ms = (BEAT_ALIGN_TOLERANCE_SECS * 1000.0) as i64;

        let mut best: Option<(i64, i64)> = None;
        for &o in &onsets_ms {
            let abs_onset_ms = music_start_ms + o as i64;
            let d = (abs_onset_ms - cue_start_ms).abs();
            if d <= tolerance_ms && best.map(|(bd, _)| d < bd).unwrap_or(true) {
                best = Some((d, abs_onset_ms));
            }
        }
        if let Some((_, target_ms)) = best {
            let target_frame = (target_ms.max(0) as f64 * self.fps as f64 / 1000.0) as u64;
            cue.start_frame = target_frame;
        }
    }

    fn find_music_cue(&self) -> Option<&LoadedCue> {
        if let Some(exact) = self.cues.iter().find(|c| c.cue.id == "music") {
            return Some(exact);
        }
        self.cues
            .iter()
            .find(|c| c.cue.id.to_ascii_lowercase().contains("music"))
    }

    /// Render the full mix for `duration_frames` of composition output.
    /// Returns interleaved stereo f32 samples at the mixer's sample rate.
    pub fn render(&self, duration_frames: u64) -> Result<Vec<f32>, AudioError> {
        let total_samples = self.frames_to_samples(duration_frames);
        let mut out = vec![0.0f32; total_samples * 2];

        // Pre-compute per-cue id index for ducking lookup.
        let id_to_idx: HashMap<&str, usize> = self
            .cues
            .iter()
            .enumerate()
            .map(|(i, lc)| (lc.cue.id.as_str(), i))
            .collect();

        // For each output sample, compute the cue envelopes + ducking.
        // Per-sample frame-number lookup avoids per-frame iteration overhead
        // at the cost of one division per sample. For typical compositions
        // (~10 cues × 30 minutes @ 48kHz = 86 million samples × 10 = 860M
        // float ops), this is fast enough.
        let samples_per_frame = self.sample_rate as f64 / self.fps as f64;

        for sample_idx in 0..total_samples {
            // Which video frame this sample falls on.
            let video_frame = (sample_idx as f64 / samples_per_frame) as u64;

            // Per-cue envelope at this frame.
            let mut cue_envs: Vec<f32> = self
                .cues
                .iter()
                .map(|lc| lc.cue.envelope_at(video_frame))
                .collect();

            // Apply ducking: for each active cue, scale its duck_targets.
            //
            // Duck DEPTH is modulated by the ducking cue's own envelope. So
            // when the VO is mid-fade-in at env=0.5, the music drops by only
            // half the dB depth too. This inherits the cue's `fade_in_frames`
            // / `fade_out_frames` as the duck attack/release without any new
            // parameters — and avoids the hard-switch artifact where music
            // pops to full duck the instant a fading-in cue crosses zero.
            //
            // Interpolation is in dB-space (loudness-perceptual) rather than
            // linear amplitude, matching standard broadcast-ducker behavior.
            for (i, lc) in self.cues.iter().enumerate() {
                let driver_env = cue_envs[i];
                if driver_env <= 0.0 || lc.cue.duck_targets.is_empty() {
                    continue;
                }
                let effective_db = lc.cue.duck_db * driver_env;
                let duck_scale = 10f32.powf(-effective_db / 20.0);
                for target_id in &lc.cue.duck_targets {
                    if let Some(&target_idx) = id_to_idx.get(target_id.as_str()) {
                        cue_envs[target_idx] *= duck_scale;
                    }
                }
            }

            // Sum cue contributions into the output sample.
            let mut l = 0.0f32;
            let mut r = 0.0f32;
            for (i, lc) in self.cues.iter().enumerate() {
                let env = cue_envs[i];
                if env <= 0.0 {
                    continue;
                }
                // Which sample in the cue's local samples buffer?
                let cue_start_sample = self.frames_to_samples(lc.cue.start_frame);
                if sample_idx < cue_start_sample {
                    continue;
                }
                let local_sample = sample_idx - cue_start_sample;
                let lr_idx = local_sample * 2;
                if lr_idx + 1 >= lc.samples.len() {
                    continue;
                }
                let (pan_l, pan_r) = lc.cue.pan_gains();
                l += lc.samples[lr_idx] * env * pan_l;
                r += lc.samples[lr_idx + 1] * env * pan_r;
            }

            // Soft clip to [-1, 1] — naive hard limit; v1 could add a real
            // limiter. For typical mixes this is fine.
            out[sample_idx * 2] = l.clamp(-1.0, 1.0);
            out[sample_idx * 2 + 1] = r.clamp(-1.0, 1.0);
        }

        Ok(out)
    }

    /// How many interleaved-stereo sample frames correspond to `n` video frames.
    fn frames_to_samples(&self, video_frames: u64) -> usize {
        (video_frames as f64 * self.sample_rate as f64 / self.fps as f64) as usize
    }

    /// Number of loaded cues.
    pub fn cue_count(&self) -> usize {
        self.cues.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn mk_cue(id: &str, start: u64, dur: u64) -> AudioCue {
        AudioCue {
            id: id.into(),
            asset_path: PathBuf::from("/tmp/unused"),
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

    fn synth_sine(freq_hz: f32, dur_secs: f32, sample_rate: u32) -> Vec<f32> {
        let n = (dur_secs * sample_rate as f32) as usize;
        let mut out = Vec::with_capacity(n * 2);
        for i in 0..n {
            let t = i as f32 / sample_rate as f32;
            let s = (t * freq_hz * std::f32::consts::TAU).sin() * 0.5;
            out.push(s);
            out.push(s);
        }
        out
    }

    #[test]
    fn render_single_cue_full_duration() {
        let mut mixer = AudioMixer::new(48_000, 30);
        let cue = mk_cue("voice", 0, 30); // 1s at 30fps
        let samples = synth_sine(440.0, 1.0, 48_000);
        mixer.add_cue_with_samples(cue, samples, 48_000).unwrap();

        let out = mixer.render(30).unwrap();
        // Output should be 48000 stereo samples = 96000 floats.
        assert_eq!(out.len(), 96_000);
        // First sample should be ~0 (sine starts at 0).
        assert!(out[0].abs() < 0.01);
        // The maximum absolute sample anywhere should be close to 0.5
        // (synth_sine amplitude * equal-power-center pan ≈ 0.5 * 0.707 = 0.35).
        // Don't pick a single sample — 440Hz lands on sin-zeros at integer-fraction
        // sample positions; check the loudest sample instead.
        let max = out.iter().map(|s| s.abs()).fold(0.0f32, f32::max);
        assert!(max > 0.2, "expected non-trivial peak amplitude, got {}", max);
    }

    #[test]
    fn ducking_attenuates_target() {
        let mut mixer = AudioMixer::new(48_000, 30);
        // Music plays for 2s
        let music = AudioCue {
            id: "music".into(),
            asset_path: PathBuf::from("/tmp/unused"),
            start_frame: 0,
            duration_frames: 60,
            volume: 1.0,
            ..mk_cue("music", 0, 60)
        };
        let music_samples = synth_sine(220.0, 2.0, 48_000);
        mixer.add_cue_with_samples(music, music_samples, 48_000).unwrap();

        // Narration plays 0.5-1.5s and ducks "music" by 24 dB
        let mut narration = mk_cue("narration", 15, 30);
        narration.duck_targets = vec!["music".into()];
        narration.duck_db = 24.0;
        let nar_samples = vec![0.0f32; 48_000];  // silent narration (just for ducking)
        mixer
            .add_cue_with_samples(narration, nar_samples, 48_000)
            .unwrap();

        let out = mixer.render(60).unwrap();

        // Sample at t=0.25s (before narration) — music plays at full
        let s_before = out[(48_000 / 4) * 2];
        // Sample at t=1.0s (mid narration) — music should be heavily ducked
        let s_during = out[48_000 * 2];

        let ratio = s_during.abs() / s_before.abs().max(0.001);
        // 24 dB duck = 10^(-24/20) ≈ 0.063. Allow some slack.
        assert!(
            ratio < 0.15,
            "expected heavy duck, got ratio {} (before={}, during={})",
            ratio,
            s_before,
            s_during
        );
    }

    #[test]
    fn duplicate_id_rejected() {
        let mut mixer = AudioMixer::new(48_000, 30);
        let a = mk_cue("dup", 0, 10);
        let b = mk_cue("dup", 10, 10);
        let samples = vec![0.0f32; 16_000];
        mixer.add_cue_with_samples(a, samples.clone(), 48_000).unwrap();
        let r = mixer.add_cue_with_samples(b, samples, 48_000);
        assert!(matches!(r, Err(AudioError::DuplicateCue(_))));
    }

    /// Build a click-track-shaped signal — a sharp impulse at each
    /// onset_secs position with silence between. Spectral-flux fires
    /// hard on these.
    fn click_track(onsets_secs: &[f32], total_secs: f32, sample_rate: u32) -> Vec<f32> {
        let n = (total_secs * sample_rate as f32) as usize;
        let mut out = vec![0.0f32; n * 2];
        for &t in onsets_secs {
            let idx = (t * sample_rate as f32) as usize;
            // Sharp 5ms burst at full amplitude.
            let burst = (sample_rate as f32 * 0.005) as usize;
            for k in 0..burst {
                if idx + k >= n {
                    break;
                }
                let env = 1.0 - (k as f32 / burst as f32);
                let s = env * if k % 2 == 0 { 0.9 } else { -0.9 };
                out[(idx + k) * 2] = s;
                out[(idx + k) * 2 + 1] = s;
            }
        }
        out
    }

    #[test]
    fn align_to_beat_snaps_vo_to_nearest_onset() {
        let mut mixer = AudioMixer::new(48_000, 30);
        // Music with strong onsets at 0.5s, 1.0s, 1.5s, 2.0s.
        let music = mk_cue("music", 0, 60);
        let music_samples = click_track(&[0.5, 1.0, 1.5, 2.0], 2.0, 48_000);
        mixer
            .add_cue_with_samples(music, music_samples, 48_000)
            .unwrap();

        // VO requested to start at frame 33 (1.1s @ 30fps) — onset at
        // 1.0s is 0.1s away (within tolerance), should snap to frame 30.
        let mut vo = mk_cue("vo", 33, 30);
        vo.align_to_beat = true;
        let vo_samples = vec![0.0f32; 48_000];
        mixer.add_cue_with_samples(vo, vo_samples, 48_000).unwrap();

        let loaded = mixer
            .cues
            .iter()
            .find(|c| c.cue.id == "vo")
            .expect("vo loaded");
        assert!(
            loaded.cue.start_frame.abs_diff(30) <= 1,
            "vo should snap to frame ~30 (1.0s onset), got {}",
            loaded.cue.start_frame,
        );
    }

    #[test]
    fn align_to_beat_false_leaves_start_unchanged() {
        let mut mixer = AudioMixer::new(48_000, 30);
        let music = mk_cue("music", 0, 60);
        let music_samples = click_track(&[0.5, 1.0, 1.5, 2.0], 2.0, 48_000);
        mixer
            .add_cue_with_samples(music, music_samples, 48_000)
            .unwrap();

        let vo = mk_cue("vo", 33, 30);
        // Note: align_to_beat=false (the mk_cue default).
        let vo_samples = vec![0.0f32; 48_000];
        mixer.add_cue_with_samples(vo, vo_samples, 48_000).unwrap();

        let loaded = mixer.cues.iter().find(|c| c.cue.id == "vo").unwrap();
        assert_eq!(loaded.cue.start_frame, 33);
    }

    #[test]
    fn align_to_beat_no_music_leaves_start_unchanged() {
        let mut mixer = AudioMixer::new(48_000, 30);
        // No music cue loaded.
        let mut vo = mk_cue("vo", 33, 30);
        vo.align_to_beat = true;
        let vo_samples = vec![0.0f32; 48_000];
        mixer.add_cue_with_samples(vo, vo_samples, 48_000).unwrap();

        let loaded = mixer.cues.iter().find(|c| c.cue.id == "vo").unwrap();
        assert_eq!(loaded.cue.start_frame, 33);
    }

    #[test]
    fn empty_mixer_renders_silence() {
        let mixer = AudioMixer::new(48_000, 30);
        let out = mixer.render(30).unwrap();
        assert!(out.iter().all(|&s| s == 0.0));
    }
}
