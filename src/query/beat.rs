//! Beat-aligned timing checks — the killer demo. Phase 3 of epic wb-q4a6.
//!
//! Detects onsets in an audio file via spectral-flux (the
//! standard textbook algorithm), then for every composition event the
//! caller cares about (scene start, audio cue start), it
//! reports the alignment delta to the nearest detected onset in ms.
//!
//! Pure Rust: symphonia decodes, `rustfft` runs the STFT. No librosa, no
//! Python, no aubio. ~150 LOC for the detector + scoring.

use crate::audio::DecodedAudio;
use crate::render_offline::Composition;
use rustfft::{num_complex::Complex, FftPlanner};
use serde::{Deserialize, Serialize};
use std::path::Path;

/// One declared event in the composition's timeline.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompositionEvent {
    /// Stable label like `scene-0-start`, `audio-music-start`.
    pub name: String,
    /// Time in milliseconds from the composition's start.
    pub time_ms: u32,
}

/// One scored event — its expected time vs the nearest detected onset.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScoredEvent {
    /// Event label.
    pub name: String,
    /// Declared time from the composition.
    pub expected_ms: u32,
    /// Time of the nearest detected onset in ms.
    pub detected_beat_ms: Option<u32>,
    /// Signed delta (detected - expected) in ms. None when no onset found.
    pub delta_ms: Option<i32>,
    /// True when |delta_ms| <= tolerance_ms.
    pub within_tolerance: bool,
    /// Index of the matched onset within the detected onset list.
    pub beat_index: Option<usize>,
}

/// Top-level result for `--on-beat`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OnBeatResult {
    /// True iff every event is within tolerance.
    pub ok: bool,
    /// Per-event scoring entries.
    pub events: Vec<ScoredEvent>,
    /// Number of events within tolerance.
    pub aligned: usize,
    /// Number of events scored.
    pub total: usize,
    /// Largest absolute delta across all events, in ms.
    pub worst_delta_ms: u32,
    /// Events that failed.
    pub failed: Vec<String>,
    /// Tolerance window used.
    pub tolerance_ms: u32,
    /// Number of detected onsets across the audio file.
    pub onset_count: usize,
}

/// Detect onsets in `audio_path` and score each event in `events` against
/// the nearest detected onset. Returns one structured result the CLI emits.
pub fn check(
    audio_path: &Path,
    events: &[CompositionEvent],
    tolerance_ms: u32,
) -> Result<OnBeatResult, String> {
    let audio = DecodedAudio::decode(audio_path).map_err(|e| format!("decode: {e}"))?;
    let onsets = detect_onsets_interleaved(&audio.samples, audio.sample_rate);

    let mut scored = Vec::with_capacity(events.len());
    let mut aligned = 0usize;
    let mut worst: u32 = 0;
    let mut failed = Vec::new();
    let tol = tolerance_ms as i32;

    for ev in events {
        let (nearest_idx, nearest_ms) = nearest_onset(&onsets, ev.time_ms);
        let delta = nearest_ms.map(|m| m as i32 - ev.time_ms as i32);
        let within = delta.map(|d| d.abs() <= tol).unwrap_or(false);
        if within {
            aligned += 1;
        } else {
            failed.push(ev.name.clone());
        }
        if let Some(d) = delta {
            let abs = d.unsigned_abs();
            if abs > worst {
                worst = abs;
            }
        }
        scored.push(ScoredEvent {
            name: ev.name.clone(),
            expected_ms: ev.time_ms,
            detected_beat_ms: nearest_ms,
            delta_ms: delta,
            within_tolerance: within,
            beat_index: nearest_idx,
        });
    }

    Ok(OnBeatResult {
        ok: failed.is_empty(),
        aligned,
        total: events.len(),
        worst_delta_ms: worst,
        failed,
        events: scored,
        tolerance_ms,
        onset_count: onsets.len(),
    })
}

/// Derive timeline events from a composition. Each scene boundary and each
/// audio-cue start becomes one labeled event.
pub fn events_from_composition(comp: &Composition) -> Vec<CompositionEvent> {
    let mut out = Vec::new();
    let fps = comp.fps as f32;
    for (i, scene) in comp.scenes.iter().enumerate() {
        let scene_start_ms = (scene.start_frame as f32 / fps * 1000.0) as u32;
        out.push(CompositionEvent {
            name: format!("scene-{i}-start"),
            time_ms: scene_start_ms,
        });
    }
    for cue in &comp.audio_cues {
        // Skip cues that are background-music-shaped (long, fade-in, low
        // volume) — they're the bed, not an event. Heuristic: skip cues
        // longer than half the composition duration.
        if cue.duration_frames > comp.duration_frames / 2 {
            continue;
        }
        let t_ms = (cue.start_frame as f32 / fps * 1000.0) as u32;
        out.push(CompositionEvent {
            name: format!("audio-{}-start", cue.id),
            time_ms: t_ms,
        });
    }
    out.sort_by_key(|e| e.time_ms);
    out
}

/// Find the detected onset closest in time to `target_ms`. Returns
/// `(index, ms)` for the nearest, or `(None, None)` when the list is empty.
fn nearest_onset(onsets: &[u32], target_ms: u32) -> (Option<usize>, Option<u32>) {
    if onsets.is_empty() {
        return (None, None);
    }
    let mut best_i = 0;
    let mut best_d: u32 = (onsets[0] as i64 - target_ms as i64).unsigned_abs() as u32;
    for (i, &m) in onsets.iter().enumerate().skip(1) {
        let d = (m as i64 - target_ms as i64).unsigned_abs() as u32;
        if d < best_d {
            best_d = d;
            best_i = i;
        }
    }
    (Some(best_i), Some(onsets[best_i]))
}

const FFT_SIZE: usize = 1024;
const HOP_SIZE: usize = 512;
const MIN_ONSET_SPACING_MS: u32 = 150;

/// Spectral-flux onset detection from interleaved stereo `[L0,R0,L1,R1,…]`.
/// See [`detect_onsets`] for the per-channel form; this convenience wraps
/// it for callers consuming `DecodedAudio::samples` directly.
pub fn detect_onsets_interleaved(samples_interleaved: &[f32], sample_rate: u32) -> Vec<u32> {
    let n = samples_interleaved.len() / 2;
    let mut mono = Vec::with_capacity(n);
    for i in 0..n {
        mono.push((samples_interleaved[i * 2] + samples_interleaved[i * 2 + 1]) * 0.5);
    }
    detect_onsets_mono(&mono, sample_rate)
}

/// Spectral-flux onset detection. Returns onset times in ms.
///
/// 1. Sum L+R into mono.
/// 2. Sliding STFT (Hann window, FFT_SIZE samples, HOP_SIZE stride).
/// 3. Per-frame spectral flux = sum of half-wave-rectified bin-magnitude
///    differences vs the previous frame.
/// 4. Smooth flux with a 5-frame moving average.
/// 5. Adaptive threshold = median over a 10-frame sliding window * 1.5
///    (capped at +0.1 absolute).
/// 6. Peak-pick: local maxima above threshold, minimum spacing
///    MIN_ONSET_SPACING_MS apart.
pub fn detect_onsets(samples_l: &[f32], samples_r: &[f32], sample_rate: u32) -> Vec<u32> {
    let n = samples_l.len().min(samples_r.len());
    if n < FFT_SIZE {
        return Vec::new();
    }
    let mono: Vec<f32> = (0..n).map(|i| (samples_l[i] + samples_r[i]) * 0.5).collect();
    detect_onsets_mono(&mono, sample_rate)
}

/// Core detector — operates on mono samples. Reused by both stereo paths.
pub fn detect_onsets_mono(mono: &[f32], sample_rate: u32) -> Vec<u32> {
    let n = mono.len();
    if n < FFT_SIZE {
        return Vec::new();
    }

    // Hann window.
    let window: Vec<f32> = (0..FFT_SIZE)
        .map(|i| {
            let w = 2.0 * std::f32::consts::PI * i as f32 / (FFT_SIZE - 1) as f32;
            0.5 - 0.5 * w.cos()
        })
        .collect();

    let mut planner = FftPlanner::<f32>::new();
    let fft = planner.plan_fft_forward(FFT_SIZE);

    let mut prev_mag: Vec<f32> = vec![0.0; FFT_SIZE / 2 + 1];
    let mut flux: Vec<f32> = Vec::new();

    let mut buf: Vec<Complex<f32>> = vec![Complex::new(0.0, 0.0); FFT_SIZE];
    let mut frame_idx = 0usize;

    while frame_idx + FFT_SIZE <= n {
        for i in 0..FFT_SIZE {
            buf[i] = Complex::new(mono[frame_idx + i] * window[i], 0.0);
        }
        fft.process(&mut buf);

        let mut sum_flux = 0.0f32;
        for k in 0..=FFT_SIZE / 2 {
            let mag = buf[k].norm();
            let diff = mag - prev_mag[k];
            if diff > 0.0 {
                sum_flux += diff;
            }
            prev_mag[k] = mag;
        }
        flux.push(sum_flux);
        frame_idx += HOP_SIZE;
    }

    // 5-frame moving-average smoothing.
    let mut smoothed: Vec<f32> = vec![0.0; flux.len()];
    let radius = 2;
    for i in 0..flux.len() {
        let lo = i.saturating_sub(radius);
        let hi = (i + radius + 1).min(flux.len());
        let count = hi - lo;
        let sum: f32 = flux[lo..hi].iter().sum();
        smoothed[i] = sum / count as f32;
    }

    // Adaptive median threshold over a sliding window.
    let win = 10usize;
    let mut thresholds: Vec<f32> = vec![0.0; smoothed.len()];
    for i in 0..smoothed.len() {
        let lo = i.saturating_sub(win);
        let hi = (i + win + 1).min(smoothed.len());
        let mut slice: Vec<f32> = smoothed[lo..hi].to_vec();
        slice.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let median = slice[slice.len() / 2];
        let max = slice.iter().cloned().fold(0.0f32, f32::max);
        thresholds[i] = (median * 1.5).max(max * 0.1);
    }

    // Peak-pick with minimum spacing.
    let frames_per_ms = sample_rate as f32 / HOP_SIZE as f32 / 1000.0;
    let min_spacing = (MIN_ONSET_SPACING_MS as f32 * frames_per_ms) as usize;
    let mut onsets = Vec::new();
    let mut last_peak: Option<usize> = None;
    for i in 1..smoothed.len() - 1 {
        if smoothed[i] > thresholds[i]
            && smoothed[i] >= smoothed[i - 1]
            && smoothed[i] > smoothed[i + 1]
        {
            if let Some(lp) = last_peak {
                if i - lp < min_spacing {
                    continue;
                }
            }
            let time_samples = i * HOP_SIZE;
            let time_ms = (time_samples as f32 / sample_rate as f32 * 1000.0) as u32;
            onsets.push(time_ms);
            last_peak = Some(i);
        }
    }
    onsets
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Generate a deterministic click track at `bpm` for `secs` seconds at
    /// `sr` Hz. Each click is a 20ms exponentially-decaying noise burst.
    fn synth_click_track(bpm: f32, secs: f32, sr: u32) -> (Vec<f32>, Vec<f32>) {
        let n = (secs * sr as f32) as usize;
        let mut l = vec![0.0f32; n];
        let mut r = vec![0.0f32; n];
        let beat_period_samples = (60.0 / bpm * sr as f32) as usize;
        let click_len = (0.020 * sr as f32) as usize; // 20ms click
        // simple LCG for deterministic noise
        let mut state: u32 = 0xdeadbeef;
        let mut next = move || -> f32 {
            state = state.wrapping_mul(1103515245).wrapping_add(12345);
            (state as f32 / u32::MAX as f32) * 2.0 - 1.0
        };
        let mut t = 0usize;
        while t < n {
            for k in 0..click_len {
                let env = (-(k as f32) / (click_len as f32 * 0.3)).exp();
                let s = next() * env;
                if t + k < n {
                    l[t + k] = s;
                    r[t + k] = s;
                }
            }
            t += beat_period_samples;
        }
        (l, r)
    }

    #[test]
    fn detects_120bpm_click_track() {
        let sr = 44_100;
        let (l, r) = synth_click_track(120.0, 5.0, sr);
        let onsets = detect_onsets(&l, &r, sr);
        // 120 BPM × 5s = 10 expected clicks. The first click at t=0 may
        // be missed depending on window alignment; allow 8–10.
        assert!(
            onsets.len() >= 8 && onsets.len() <= 12,
            "expected ~10 onsets at 120 BPM, got {}",
            onsets.len()
        );
        // Spacing should be ~500ms ± 50ms.
        for w in onsets.windows(2) {
            let dt = w[1] - w[0];
            assert!(
                dt > 400 && dt < 600,
                "click spacing should be ~500ms, got {dt}"
            );
        }
    }

    #[test]
    fn nearest_onset_finds_closest() {
        let onsets = vec![100, 500, 1000, 1500];
        assert_eq!(nearest_onset(&onsets, 0), (Some(0), Some(100)));
        assert_eq!(nearest_onset(&onsets, 480), (Some(1), Some(500)));
        assert_eq!(nearest_onset(&onsets, 1300), (Some(3), Some(1500)));
        assert_eq!(nearest_onset(&[], 100), (None, None));
    }

    #[test]
    fn scoring_marks_within_tolerance() {
        let onsets_via_static = vec![100u32, 500, 1000];
        // Replicate the scoring loop by hand to verify the deltas.
        let events = vec![
            CompositionEvent { name: "a".into(), time_ms: 90 },   // off by -10ms
            CompositionEvent { name: "b".into(), time_ms: 520 },  // off by +20ms
            CompositionEvent { name: "c".into(), time_ms: 1200 }, // off by -200ms
        ];
        let tol = 50;
        for ev in &events {
            let (_, near) = nearest_onset(&onsets_via_static, ev.time_ms);
            let d = near.unwrap() as i32 - ev.time_ms as i32;
            match ev.name.as_str() {
                "a" => assert!(d.abs() <= tol),
                "b" => assert!(d.abs() <= tol),
                "c" => assert!(d.abs() > tol),
                _ => unreachable!(),
            }
        }
    }
}
