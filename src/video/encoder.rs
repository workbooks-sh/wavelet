//! [`VideoEncoder`] — write RGBA frames to an MP4 via libx264/libx265/rav1e.

use super::codec::Codec;
use super::errors::VideoError;
use super::frame::RgbaFrame;
use rsmpeg::avcodec::{AVCodec, AVCodecContext};
use rsmpeg::avformat::AVFormatContextOutput;
use rsmpeg::avutil::{AVDictionary, AVFrame};
use rsmpeg::error::RsmpegError;
use rsmpeg::ffi;
use rsmpeg::swscale::SwsContext;
use std::ffi::CString;
use std::path::Path;

/// Frame encoder. Open once, push frames in order, finalize.
pub struct VideoEncoder {
    output_ctx: AVFormatContextOutput,
    enc_ctx: AVCodecContext,
    sws: SwsContext,
    width: i32,
    height: i32,
    fps: i32,
    next_pts: i64,
    enc_tb: ffi::AVRational,
    stream_tb: ffi::AVRational,
    finalized: bool,
}

impl VideoEncoder {
    /// Open an encoder writing to `out_path`. Width/height match what callers
    /// will push; mismatched RGBA frames panic at `push_frame` time.
    ///
    /// Codec defaults: H.264 ultrafast preset; H.265 medium preset; AV1 (if
    /// the `av1` feature is enabled) speed=8 (real-time encoding).
    pub fn open(
        out_path: impl AsRef<Path>,
        width: u32,
        height: u32,
        fps: u32,
        codec: Codec,
    ) -> Result<Self, VideoError> {
        let path_c = CString::new(out_path.as_ref().to_string_lossy().into_owned())?;
        let mut output_ctx = AVFormatContextOutput::create(&path_c)?;

        let encoder = AVCodec::find_encoder(codec.ffmpeg_id())
            .ok_or_else(|| VideoError::NoEncoder { codec })?;
        let mut enc_ctx = AVCodecContext::new(&encoder);
        enc_ctx.set_width(width as i32);
        enc_ctx.set_height(height as i32);
        enc_ctx.set_pix_fmt(ffi::AV_PIX_FMT_YUV420P);
        enc_ctx.set_time_base(ffi::AVRational {
            num: 1,
            den: fps as i32,
        });
        enc_ctx.set_framerate(ffi::AVRational {
            num: fps as i32,
            den: 1,
        });
        enc_ctx.set_gop_size(fps as i32);
        enc_ctx.set_max_b_frames(0);

        // Codec-specific opts.
        let opts = match codec {
            Codec::H264 => {
                let preset = CString::new("ultrafast")?;
                AVDictionary::new(&CString::new("preset")?, &preset, 0)
            }
            Codec::H265 => {
                let preset = CString::new("medium")?;
                AVDictionary::new(&CString::new("preset")?, &preset, 0)
            }
            #[cfg(feature = "av1")]
            Codec::Av1 => {
                let speed = CString::new("8")?;
                AVDictionary::new(&CString::new("speed")?, &speed, 0)
            }
        };

        enc_ctx.open(Some(opts))?;

        // Add the stream + copy codec params. Scope the stream borrow so
        // write_header has clean mutable access to output_ctx.
        //
        // CRITICAL: capture stream_tb AFTER write_header, not before.
        // libavformat normalizes the stream time_base in write_header
        // (e.g. MP4 typically rewrites to 1/15360 or similar high-resolution
        // container time-base). Capturing before means we rescale_ts using
        // the un-normalized {1, fps} and the packets land at PTS values that
        // produce a microsecond-long video duration. Capturing after picks
        // up the container's authoritative time_base.
        {
            let mut stream = output_ctx.new_stream();
            stream.set_time_base(ffi::AVRational {
                num: 1,
                den: fps as i32,
            });
            stream.set_codecpar(enc_ctx.extract_codecpar());
        }
        let enc_tb = enc_ctx.time_base;

        output_ctx.write_header(&mut None)?;

        // Read the stream's now-normalized time_base.
        let stream_tb = output_ctx.streams()[0].time_base;

        let sws = SwsContext::get_context(
            width as i32,
            height as i32,
            ffi::AV_PIX_FMT_RGBA,
            width as i32,
            height as i32,
            ffi::AV_PIX_FMT_YUV420P,
            ffi::SWS_BILINEAR,
            None,
            None,
            None,
        )
        .ok_or_else(|| VideoError::Ffmpeg("sws_getContext failed".into()))?;

        Ok(Self {
            output_ctx,
            enc_ctx,
            sws,
            width: width as i32,
            height: height as i32,
            fps: fps as i32,
            next_pts: 0,
            enc_tb,
            stream_tb,
            finalized: false,
        })
    }

    /// Push one RGBA frame. Frames must be in output order with monotonically
    /// increasing PTS — typical use is a `for i in 0..N { push_frame(frame_i) }` loop.
    pub fn push_frame(&mut self, frame: &RgbaFrame) -> Result<(), VideoError> {
        assert_eq!(frame.width as i32, self.width, "width mismatch");
        assert_eq!(frame.height as i32, self.height, "height mismatch");

        // Wrap pixels in an AVFrame.
        let mut rgba_frame = AVFrame::new();
        rgba_frame.set_width(self.width);
        rgba_frame.set_height(self.height);
        rgba_frame.set_format(ffi::AV_PIX_FMT_RGBA);
        rgba_frame.alloc_buffer()?;
        let stride = rgba_frame.linesize[0] as usize;
        let row_bytes = (self.width as usize) * 4;
        unsafe {
            for y in 0..self.height as usize {
                let dst = std::slice::from_raw_parts_mut(
                    rgba_frame.data[0].add(y * stride),
                    row_bytes,
                );
                let src_offset = y * row_bytes;
                dst.copy_from_slice(&frame.pixels[src_offset..src_offset + row_bytes]);
            }
        }

        // Convert RGBA → YUV420P.
        let mut yuv = AVFrame::new();
        yuv.set_width(self.width);
        yuv.set_height(self.height);
        yuv.set_format(ffi::AV_PIX_FMT_YUV420P);
        yuv.alloc_buffer()?;
        self.sws.scale_frame(&rgba_frame, 0, self.height, &mut yuv)?;
        yuv.set_pts(self.next_pts);
        self.next_pts += 1;

        // Send + drain encoder.
        self.enc_ctx.send_frame(Some(&yuv))?;
        self.drain()?;
        Ok(())
    }

    /// Flush + write the container trailer. Must be called before drop to
    /// produce a playable MP4 — partial files without the trailer are not
    /// valid.
    pub fn finalize(mut self) -> Result<(), VideoError> {
        self.finalize_inner()
    }

    fn finalize_inner(&mut self) -> Result<(), VideoError> {
        if self.finalized {
            return Ok(());
        }
        // Flush the encoder.
        self.enc_ctx.send_frame(None)?;
        self.drain()?;
        self.output_ctx.write_trailer()?;
        self.finalized = true;
        Ok(())
    }

    fn drain(&mut self) -> Result<(), VideoError> {
        loop {
            match self.enc_ctx.receive_packet() {
                Ok(mut packet) => {
                    packet.set_stream_index(0);
                    packet.rescale_ts(self.enc_tb, self.stream_tb);
                    self.output_ctx.write_frame(&mut packet)?;
                }
                Err(RsmpegError::EncoderDrainError) => return Ok(()),
                Err(RsmpegError::EncoderFlushedError) => return Ok(()),
                Err(e) => return Err(VideoError::Ffmpeg(e.to_string())),
            }
        }
    }

    /// Frames pushed so far. Equal to the next frame's PTS.
    pub fn frame_count(&self) -> u64 {
        self.next_pts as u64
    }
}

impl Drop for VideoEncoder {
    fn drop(&mut self) {
        // Best-effort finalize on drop. If the caller forgot to call
        // finalize(), we still write the trailer so the file is playable.
        if !self.finalized {
            let _ = self.finalize_inner();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn tmp_path(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("wavelet-video-test-{name}.mp4"))
    }

    #[test]
    fn encode_30_synthetic_frames_h264() {
        let out = tmp_path("h264");
        let _ = std::fs::remove_file(&out);
        let mut enc = VideoEncoder::open(&out, 64, 64, 30, Codec::H264).expect("open");

        for i in 0..30 {
            let mut frame = RgbaFrame::black(64, 64);
            // Make each frame visually distinct so the encoder isn't sending
            // identical content (which would over-compress and look like a bug).
            let v = (i * 8) as u8;
            for px in frame.pixels.chunks_exact_mut(4) {
                px[0] = v;
                px[3] = 255;
            }
            enc.push_frame(&frame).expect("push");
        }
        assert_eq!(enc.frame_count(), 30);
        enc.finalize().expect("finalize");

        let meta = std::fs::metadata(&out).expect("output file");
        assert!(meta.len() > 100, "output file is suspiciously small: {} bytes", meta.len());
    }

    #[test]
    fn encode_h265() {
        let out = tmp_path("h265");
        let _ = std::fs::remove_file(&out);
        let mut enc = VideoEncoder::open(&out, 64, 64, 30, Codec::H265).expect("open");
        for i in 0..15 {
            let mut frame = RgbaFrame::black(64, 64);
            for px in frame.pixels.chunks_exact_mut(4) {
                px[1] = (i * 16) as u8;
                px[3] = 255;
            }
            enc.push_frame(&frame).expect("push");
        }
        enc.finalize().expect("finalize");
        assert!(std::fs::metadata(&out).expect("file").len() > 100);
    }
}
