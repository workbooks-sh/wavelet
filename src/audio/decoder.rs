//! [`DecodedAudio`] — symphonia wrapper that gives us full-file f32 stereo.
//!
//! For offline render, we decode each cue fully into memory once. Typical HF
//! compositions have 1-10 audio cues totaling under a few minutes — even
//! generous estimates put this at ~50 MB total RAM, which is fine for a
//! desktop offline render. Streaming-decode optimization is a v1 concern.

use super::errors::AudioError;
use std::fs::File;
use std::path::Path;
use symphonia::core::audio::{AudioBufferRef, Signal};
use symphonia::core::codecs::DecoderOptions;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

/// One fully-decoded audio asset, in f32 interleaved stereo.
///
/// `sample_rate` is whatever the source uses; the mixer resamples to project
/// rate as needed.
#[derive(Debug, Clone)]
pub struct DecodedAudio {
    /// Interleaved stereo samples: `[L0, R0, L1, R1, …]`.
    pub samples: Vec<f32>,
    /// Source sample rate (Hz).
    pub sample_rate: u32,
}

impl DecodedAudio {
    /// Decode an audio file at `path` fully into memory as f32 stereo.
    pub fn decode(path: impl AsRef<Path>) -> Result<Self, AudioError> {
        let path = path.as_ref();
        let file = File::open(path)?;
        let mss = MediaSourceStream::new(Box::new(file), Default::default());

        // Hint the probe with the file extension.
        let mut hint = Hint::new();
        if let Some(ext) = path.extension().and_then(|s| s.to_str()) {
            hint.with_extension(ext);
        }

        let probed = symphonia::default::get_probe().format(
            &hint,
            mss,
            &FormatOptions::default(),
            &MetadataOptions::default(),
        )?;
        let mut format = probed.format;

        // Pick the first audio track.
        let track = format
            .default_track()
            .ok_or_else(|| AudioError::UnsupportedFormat(format!("{}", path.display())))?;
        let track_id = track.id;
        let codec_params = track.codec_params.clone();
        let sample_rate = codec_params.sample_rate.unwrap_or(48_000);

        let mut decoder = symphonia::default::get_codecs()
            .make(&codec_params, &DecoderOptions::default())?;

        let mut samples_lr: Vec<f32> = Vec::new();

        loop {
            let packet = match format.next_packet() {
                Ok(p) => p,
                Err(symphonia::core::errors::Error::IoError(e))
                    if e.kind() == std::io::ErrorKind::UnexpectedEof =>
                {
                    break;
                }
                Err(e) => return Err(AudioError::Decode(e.to_string())),
            };
            if packet.track_id() != track_id {
                continue;
            }

            let decoded = match decoder.decode(&packet) {
                Ok(d) => d,
                Err(symphonia::core::errors::Error::DecodeError(_)) => continue, // skip
                Err(e) => return Err(AudioError::Decode(e.to_string())),
            };

            append_as_stereo_f32(decoded, &mut samples_lr);
        }

        Ok(Self {
            samples: samples_lr,
            sample_rate,
        })
    }

    /// Total duration in seconds.
    pub fn duration_secs(&self) -> f32 {
        if self.sample_rate == 0 {
            0.0
        } else {
            (self.samples.len() / 2) as f32 / self.sample_rate as f32
        }
    }

    /// Sample frame count (one pair of L+R samples per frame).
    pub fn sample_frames(&self) -> usize {
        self.samples.len() / 2
    }
}

/// Convert a symphonia AudioBufferRef into interleaved f32 stereo and append.
///
/// Handles common source layouts: mono (duplicated to both channels), stereo
/// (already correct), and multichannel (downmixed to L/R via simple averaging
/// of the first two channels for v0).
fn append_as_stereo_f32(decoded: AudioBufferRef, out: &mut Vec<f32>) {
    let spec = *decoded.spec();
    let channels = spec.channels.count();

    // We need f32 per-channel buffers. symphonia's buffer types vary; convert
    // each kind to f32.
    use symphonia::core::audio::AudioBuffer;
    use symphonia::core::sample::Sample;

    fn drain<S: Sample + symphonia::core::conv::IntoSample<f32>>(
        buf: &AudioBuffer<S>,
        channels: usize,
        out: &mut Vec<f32>,
    ) {
        let len = buf.frames();
        // Snapshot of each channel
        let ch_data: Vec<Vec<f32>> = (0..channels)
            .map(|c| buf.chan(c).iter().map(|s| (*s).into_sample()).collect())
            .collect();

        for i in 0..len {
            let l;
            let r;
            match channels {
                0 => {
                    l = 0.0;
                    r = 0.0;
                }
                1 => {
                    let s = ch_data[0][i];
                    l = s;
                    r = s;
                }
                _ => {
                    l = ch_data[0][i];
                    r = ch_data[1][i];
                }
            }
            out.push(l);
            out.push(r);
        }
    }

    match decoded {
        AudioBufferRef::U8(b) => drain(&b, channels, out),
        AudioBufferRef::U16(b) => drain(&b, channels, out),
        AudioBufferRef::U24(b) => drain(&b, channels, out),
        AudioBufferRef::U32(b) => drain(&b, channels, out),
        AudioBufferRef::S8(b) => drain(&b, channels, out),
        AudioBufferRef::S16(b) => drain(&b, channels, out),
        AudioBufferRef::S24(b) => drain(&b, channels, out),
        AudioBufferRef::S32(b) => drain(&b, channels, out),
        AudioBufferRef::F32(b) => drain(&b, channels, out),
        AudioBufferRef::F64(b) => drain(&b, channels, out),
    }
}
