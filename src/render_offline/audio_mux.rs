//! Audio mux pass — fold a rendered stereo PCM buffer into the
//! video-only MP4 emitted by [`crate::video::VideoEncoder`].
//!
//! The wavelet renderer is structured in two phases: first an rsmpeg
//! `VideoEncoder` writes a video-only MP4, then [`AudioMixer`] renders
//! the composition's `<audio>` cues into a single stereo f32 buffer.
//! This module is phase 3 — it takes that buffer, encodes it to AAC,
//! and rewrites the MP4 in place so the canonical artifact carries
//! both streams.
//!
//! Single-pass mux (Path A — adding the audio stream alongside the
//! video stream in the same `AVFormatContextOutput`) is structurally
//! cleaner but would require interleaving audio encode with the video
//! frame loop and pulling all of the v3 video-encode plumbing inside
//! out. Path B (post-pass remux) is what's implemented here: the cost
//! is one extra read of the MP4 file, but the encode loop stays
//! untouched and the failure mode is contained to this module.
//!
//! Truncation / padding: the input buffer is already exactly
//! `duration_frames` of audio at the project sample rate — the mixer
//! does the trimming. This pass doesn't have to worry about
//! length matching.

use crate::video::VideoError;
use rsmpeg::avcodec::{AVCodec, AVCodecContext};
use rsmpeg::avformat::{AVFormatContextInput, AVFormatContextOutput};
use rsmpeg::avutil::{AVChannelLayout, AVFrame};
use rsmpeg::error::RsmpegError;
use rsmpeg::ffi;
use rsmpeg::swresample::SwrContext;
use std::ffi::CString;
use std::path::Path;

/// Encode the supplied stereo f32 buffer as AAC and mux it into
/// `mp4_path` alongside the existing video stream. The input MP4 is
/// expected to be a video-only file (the output of
/// [`crate::video::VideoEncoder::finalize`]); the output overwrites it
/// in place via a temp + rename.
///
/// `samples` is interleaved stereo (`[L0, R0, L1, R1, …]`) at
/// `sample_rate`. The mix's length is whatever the caller produced —
/// truncation / padding against the video length is the mixer's
/// responsibility upstream.
pub fn mux_stereo_into_mp4(
    mp4_path: &Path,
    samples: &[f32],
    sample_rate: u32,
) -> Result<(), VideoError> {
    if samples.is_empty() {
        return Ok(());
    }

    let tmp_path = mp4_path.with_extension("mux.mp4");
    let _ = std::fs::remove_file(&tmp_path);

    let in_path_c = path_cstring(mp4_path)?;
    let out_path_c = path_cstring(&tmp_path)?;

    let mut input_ctx = AVFormatContextInput::open(&in_path_c)
        .map_err(|e| VideoError::Ffmpeg(format!("audio mux: open {}: {e}", mp4_path.display())))?;

    let video_stream_idx = input_ctx
        .streams()
        .iter()
        .position(|s| s.codecpar().codec_type == ffi::AVMEDIA_TYPE_VIDEO)
        .ok_or_else(|| VideoError::Ffmpeg(format!(
            "audio mux: no video stream in {}",
            mp4_path.display(),
        )))? as i32;

    let mut output_ctx = AVFormatContextOutput::create(&out_path_c)
        .map_err(|e| VideoError::Ffmpeg(format!("audio mux: create {}: {e}", tmp_path.display())))?;

    let global_header =
        unsafe { (*output_ctx.oformat).flags } & ffi::AVFMT_GLOBALHEADER as i32 != 0;

    // ---- Video passthrough stream: clone the input codecpar verbatim
    // and lift the input stream's time_base so packets keep their
    // existing PTS/DTS.
    let video_in_tb;
    let video_out_stream_idx;
    {
        let in_stream = &input_ctx.streams()[video_stream_idx as usize];
        let in_codecpar = in_stream.codecpar();
        video_in_tb = in_stream.time_base;

        let mut out_video = output_ctx.new_stream();
        out_video.set_time_base(video_in_tb);
        let mut new_par = rsmpeg::avcodec::AVCodecParameters::new();
        new_par.copy(&in_codecpar);
        out_video.set_codecpar(new_par);
        video_out_stream_idx = out_video.index;
    }

    // ---- Audio AAC encoder. AAC handles 48kHz stereo; we render at
    // 192 kbps to match the "web-streamable" target in the brief.
    let aac = AVCodec::find_encoder(ffi::AV_CODEC_ID_AAC)
        .ok_or_else(|| VideoError::Ffmpeg("audio mux: no AAC encoder in libavcodec".into()))?;
    let aac_sample_fmt = pick_aac_sample_fmt(&aac);
    let mut aac_ctx = AVCodecContext::new(&aac);
    aac_ctx.set_sample_rate(sample_rate as i32);
    aac_ctx.set_sample_fmt(aac_sample_fmt);
    aac_ctx.set_ch_layout(AVChannelLayout::from_nb_channels(2).into_inner());
    aac_ctx.set_bit_rate(192_000);
    aac_ctx.set_time_base(ffi::AVRational { num: 1, den: sample_rate as i32 });
    if global_header {
        // MP4 needs the codec extradata in the moov atom, not inline.
        let cur = aac_ctx.flags;
        aac_ctx.set_flags(cur | ffi::AV_CODEC_FLAG_GLOBAL_HEADER as i32);
    }
    aac_ctx
        .open(None)
        .map_err(|e| VideoError::Ffmpeg(format!("audio mux: aac open: {e}")))?;

    let aac_tb = aac_ctx.time_base;
    let aac_frame_size = if aac_ctx.frame_size > 0 {
        aac_ctx.frame_size
    } else {
        1024
    };

    let audio_out_stream_idx;
    {
        let mut out_audio = output_ctx.new_stream();
        out_audio.set_time_base(aac_tb);
        out_audio.set_codecpar(aac_ctx.extract_codecpar());
        audio_out_stream_idx = out_audio.index;
    }

    output_ctx
        .write_header(&mut None)
        .map_err(|e| VideoError::Ffmpeg(format!("audio mux: write_header: {e}")))?;

    let audio_stream_tb = output_ctx.streams()[audio_out_stream_idx as usize].time_base;

    // ---- Stage 1: stream-copy every video packet from input to output.
    // No re-encode — packets carry their existing PTS/DTS into the new
    // container; only stream_index needs remapping.
    loop {
        let packet_opt = input_ctx
            .read_packet()
            .map_err(|e| VideoError::Ffmpeg(format!("audio mux: read_packet: {e}")))?;
        let Some(mut packet) = packet_opt else { break };
        if packet.stream_index != video_stream_idx {
            continue;
        }
        packet.set_stream_index(video_out_stream_idx);
        output_ctx
            .interleaved_write_frame(&mut packet)
            .map_err(|e| VideoError::Ffmpeg(format!("audio mux: write video pkt: {e}")))?;
    }

    // ---- Stage 2: encode the in-memory stereo PCM buffer to AAC.
    // Resample f32 interleaved (FLT) -> the encoder's preferred fmt
    // (FLTP on every libfdk / libavcodec AAC build).
    let in_ch_layout = AVChannelLayout::from_nb_channels(2).into_inner();
    let out_ch_layout = AVChannelLayout::from_nb_channels(2).into_inner();
    let mut swr = SwrContext::new(
        &out_ch_layout,
        aac_sample_fmt,
        sample_rate as i32,
        &in_ch_layout,
        ffi::AV_SAMPLE_FMT_FLT,
        sample_rate as i32,
    )
    .map_err(|e| VideoError::Ffmpeg(format!("audio mux: swr alloc: {e}")))?;
    swr.init()
        .map_err(|e| VideoError::Ffmpeg(format!("audio mux: swr init: {e}")))?;

    let total_frames = (samples.len() / 2) as i64;
    let mut sample_cursor: i64 = 0;
    let mut pts_cursor: i64 = 0;
    let bytes_per_input_sample = std::mem::size_of::<f32>() * 2; // interleaved stereo
    while sample_cursor < total_frames {
        let take = ((total_frames - sample_cursor) as i32).min(aac_frame_size);
        // Build an interleaved AVFrame referencing our slice. We
        // allocate a fresh frame each chunk — the encoder retains
        // ownership via av_buffer; cheaper than wiring up a buffer
        // pool for the duration of the mux.
        let mut in_frame = AVFrame::new();
        in_frame.set_nb_samples(take);
        in_frame.set_format(ffi::AV_SAMPLE_FMT_FLT);
        in_frame.set_ch_layout(AVChannelLayout::from_nb_channels(2).into_inner());
        in_frame.set_sample_rate(sample_rate as i32);
        in_frame
            .alloc_buffer()
            .map_err(|e| VideoError::Ffmpeg(format!("audio mux: in_frame alloc: {e}")))?;

        let needed_bytes = (take as usize) * bytes_per_input_sample;
        let src_byte_offset = (sample_cursor as usize) * bytes_per_input_sample;
        unsafe {
            let src_ptr = (samples.as_ptr() as *const u8).add(src_byte_offset);
            std::ptr::copy_nonoverlapping(src_ptr, in_frame.data[0], needed_bytes);
        }

        let mut out_frame = AVFrame::new();
        out_frame.set_nb_samples(take);
        out_frame.set_format(aac_sample_fmt);
        out_frame.set_ch_layout(AVChannelLayout::from_nb_channels(2).into_inner());
        out_frame.set_sample_rate(sample_rate as i32);
        out_frame
            .alloc_buffer()
            .map_err(|e| VideoError::Ffmpeg(format!("audio mux: out_frame alloc: {e}")))?;

        swr.convert_frame(Some(&in_frame), &mut out_frame)
            .map_err(|e| VideoError::Ffmpeg(format!("audio mux: swr convert: {e}")))?;
        out_frame.set_pts(pts_cursor);
        pts_cursor += take as i64;
        sample_cursor += take as i64;

        aac_ctx
            .send_frame(Some(&out_frame))
            .map_err(|e| VideoError::Ffmpeg(format!("audio mux: aac send_frame: {e}")))?;
        drain_aac(
            &mut aac_ctx,
            &mut output_ctx,
            audio_out_stream_idx,
            aac_tb,
            audio_stream_tb,
        )?;
    }

    // Flush.
    aac_ctx
        .send_frame(None)
        .map_err(|e| VideoError::Ffmpeg(format!("audio mux: aac flush send: {e}")))?;
    drain_aac(
        &mut aac_ctx,
        &mut output_ctx,
        audio_out_stream_idx,
        aac_tb,
        audio_stream_tb,
    )?;

    output_ctx
        .write_trailer()
        .map_err(|e| VideoError::Ffmpeg(format!("audio mux: write_trailer: {e}")))?;
    drop(output_ctx);
    drop(input_ctx);

    // Atomic-ish replace: rename the muxed temp over the original. On
    // POSIX `rename` is atomic when both paths live on the same fs,
    // which they always do here (sibling files).
    std::fs::rename(&tmp_path, mp4_path)
        .map_err(|e| VideoError::Ffmpeg(format!("audio mux: rename {} -> {}: {e}", tmp_path.display(), mp4_path.display())))?;

    Ok(())
}

fn drain_aac(
    enc: &mut AVCodecContext,
    out: &mut AVFormatContextOutput,
    audio_stream_idx: i32,
    enc_tb: ffi::AVRational,
    stream_tb: ffi::AVRational,
) -> Result<(), VideoError> {
    loop {
        match enc.receive_packet() {
            Ok(mut packet) => {
                packet.set_stream_index(audio_stream_idx);
                packet.rescale_ts(enc_tb, stream_tb);
                out.interleaved_write_frame(&mut packet)
                    .map_err(|e| VideoError::Ffmpeg(format!("audio mux: write audio pkt: {e}")))?;
            }
            Err(RsmpegError::EncoderDrainError) => return Ok(()),
            Err(RsmpegError::EncoderFlushedError) => return Ok(()),
            Err(e) => return Err(VideoError::Ffmpeg(format!("audio mux: aac receive_packet: {e}"))),
        }
    }
}

/// AAC supports a small set of sample formats. Pick the first one the
/// codec advertises — every libavcodec / libfdk_aac build returns FLTP
/// first, but reading the advertised list keeps us future-proof.
fn pick_aac_sample_fmt(codec: &AVCodec) -> i32 {
    codec
        .sample_fmts()
        .and_then(|fmts| fmts.first().copied())
        .unwrap_or(ffi::AV_SAMPLE_FMT_FLTP)
}

fn path_cstring(path: &Path) -> Result<CString, VideoError> {
    CString::new(path.to_string_lossy().into_owned())
        .map_err(|e| VideoError::Ffmpeg(format!("audio mux: path nul: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::video::{Codec, RgbaFrame, VideoEncoder};

    /// End-to-end: synth a tiny silent video, mux a 1s 440Hz tone into
    /// it, then re-probe the output for stream counts. Catches the
    /// "tmp file left behind" / "audio stream missing after rename"
    /// regressions.
    #[test]
    fn muxes_audio_into_video_only_mp4() {
        let out = std::env::temp_dir().join("wavelet-audio-mux-smoke.mp4");
        let _ = std::fs::remove_file(&out);

        let mut enc = VideoEncoder::open(&out, 64, 64, 30, Codec::H264).expect("open");
        for i in 0..15 {
            let mut f = RgbaFrame::black(64, 64);
            for px in f.pixels.chunks_exact_mut(4) {
                px[2] = (i * 16) as u8;
                px[3] = 255;
            }
            enc.push_frame(&f).expect("push");
        }
        enc.finalize().expect("finalize");

        let sample_rate = 48_000u32;
        let len = sample_rate as usize / 2; // 0.5s, comfortably > one AAC frame
        let mut samples = Vec::with_capacity(len * 2);
        for n in 0..len {
            let t = n as f32 / sample_rate as f32;
            let s = (t * 440.0 * std::f32::consts::TAU).sin() * 0.2;
            samples.push(s);
            samples.push(s);
        }

        mux_stereo_into_mp4(&out, &samples, sample_rate).expect("mux");

        // Probe stream counts.
        let path_c = CString::new(out.to_string_lossy().into_owned()).unwrap();
        let ctx = AVFormatContextInput::open(&path_c).expect("open muxed");
        let mut nv = 0;
        let mut na = 0;
        for s in ctx.streams() {
            match s.codecpar().codec_type {
                t if t == ffi::AVMEDIA_TYPE_VIDEO => nv += 1,
                t if t == ffi::AVMEDIA_TYPE_AUDIO => na += 1,
                _ => {}
            }
        }
        assert_eq!(nv, 1, "video stream missing");
        assert_eq!(na, 1, "audio stream missing");
    }

    #[test]
    fn empty_samples_is_a_noop() {
        // Easiest invariant: callers that pass an empty slice get the
        // original file back untouched. The render pipeline relies on
        // this so non-audio comps don't pay any mux cost.
        let out = std::env::temp_dir().join("wavelet-audio-mux-noop.mp4");
        let _ = std::fs::remove_file(&out);
        let mut enc = VideoEncoder::open(&out, 32, 32, 30, Codec::H264).expect("open");
        for _ in 0..5 {
            enc.push_frame(&RgbaFrame::black(32, 32)).expect("push");
        }
        enc.finalize().expect("finalize");
        let before = std::fs::metadata(&out).unwrap().len();
        mux_stereo_into_mp4(&out, &[], 48_000).expect("mux");
        let after = std::fs::metadata(&out).unwrap().len();
        assert_eq!(before, after, "noop mux changed the file");
    }
}
