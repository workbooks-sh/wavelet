//! Resample helpers around rubato.

use super::errors::AudioError;
use rubato::{Resampler, SincFixedIn, SincInterpolationParameters, SincInterpolationType, WindowFunction};

/// Resample interleaved-stereo `samples` from `src_rate` Hz to `dst_rate` Hz.
/// Pass-through (no resample) if rates already match.
///
/// Uses rubato's `SincFixedIn` with high-quality SinC interpolation. For
/// offline render quality > speed, which is what we want.
pub fn resample_stereo(
    samples: &[f32],
    src_rate: u32,
    dst_rate: u32,
) -> Result<Vec<f32>, AudioError> {
    if src_rate == dst_rate || samples.is_empty() {
        return Ok(samples.to_vec());
    }

    // Split interleaved into per-channel.
    let frames = samples.len() / 2;
    let mut left = Vec::with_capacity(frames);
    let mut right = Vec::with_capacity(frames);
    for chunk in samples.chunks_exact(2) {
        left.push(chunk[0]);
        right.push(chunk[1]);
    }

    let params = SincInterpolationParameters {
        sinc_len: 256,
        f_cutoff: 0.95,
        oversampling_factor: 256,
        interpolation: SincInterpolationType::Linear,
        window: WindowFunction::BlackmanHarris2,
    };

    let chunk_size = 1024usize;
    let mut resampler = SincFixedIn::<f32>::new(
        dst_rate as f64 / src_rate as f64,
        2.0,
        params,
        chunk_size,
        2,
    )?;

    let mut out_left: Vec<f32> = Vec::new();
    let mut out_right: Vec<f32> = Vec::new();

    let mut offset = 0;
    while offset + chunk_size <= frames {
        let input = vec![
            left[offset..offset + chunk_size].to_vec(),
            right[offset..offset + chunk_size].to_vec(),
        ];
        let output = resampler.process(&input, None)?;
        out_left.extend_from_slice(&output[0]);
        out_right.extend_from_slice(&output[1]);
        offset += chunk_size;
    }

    // Flush the resampler with the remaining tail. rubato wants a final
    // process with zero-padding for any leftover samples.
    if offset < frames {
        let mut last_l = vec![0.0; chunk_size];
        let mut last_r = vec![0.0; chunk_size];
        let tail = frames - offset;
        last_l[..tail].copy_from_slice(&left[offset..]);
        last_r[..tail].copy_from_slice(&right[offset..]);
        let output = resampler.process(&[last_l, last_r], None)?;
        // Trust rubato's output length — for the partial chunk it'll be
        // proportional. We don't have exact-output sizing for the partial
        // case here, so we accept the rounded output.
        out_left.extend_from_slice(&output[0]);
        out_right.extend_from_slice(&output[1]);
    }

    // Re-interleave
    let mut out = Vec::with_capacity(out_left.len() * 2);
    for (l, r) in out_left.iter().zip(out_right.iter()) {
        out.push(*l);
        out.push(*r);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn passthrough_when_rates_match() {
        let input = vec![0.1, 0.2, 0.3, 0.4];
        let out = resample_stereo(&input, 48_000, 48_000).unwrap();
        assert_eq!(out, input);
    }

    #[test]
    fn resample_to_double_rate_roughly_doubles_length() {
        // 1 second of silence at 24kHz stereo = 48000 samples (interleaved).
        let input = vec![0.0; 48_000];
        let out = resample_stereo(&input, 24_000, 48_000).unwrap();
        // After 2× resample, output should be ~2× input length (allow some slack
        // for rubato's chunking).
        let ratio = out.len() as f32 / input.len() as f32;
        assert!(
            ratio > 1.8 && ratio < 2.2,
            "expected ~2× output length, got {} (in={}, out={})",
            ratio,
            input.len(),
            out.len()
        );
    }

    #[test]
    fn empty_input_returns_empty() {
        let out = resample_stereo(&[], 44_100, 48_000).unwrap();
        assert!(out.is_empty());
    }
}
