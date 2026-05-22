//! Video crate errors.

use thiserror::Error;

/// Errors from video decode / encode operations.
#[derive(Debug, Error)]
pub enum VideoError {
    /// Underlying rsmpeg / libav error. Carries the message for diagnosis.
    #[error("ffmpeg error: {0}")]
    Ffmpeg(String),

    /// File couldn't be opened (path missing, permission denied, etc.).
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    /// No video stream in the input file.
    #[error("no video stream in '{0}'")]
    NoVideoStream(String),

    /// No decoder found for the source codec.
    #[error("no decoder for codec_id={0:?}")]
    NoDecoder(i32),

    /// Requested encoder isn't available in this FFmpeg build.
    #[error("no encoder for {codec:?} — rebuild FFmpeg with the codec enabled, or pick another")]
    NoEncoder {
        /// The codec that was requested.
        codec: super::codec::Codec,
    },

    /// Frame number out of range for the source.
    #[error("frame {frame} out of range (source has {total} frames)")]
    FrameOutOfRange {
        /// Requested frame.
        frame: u64,
        /// Total frame count in source.
        total: u64,
    },

    /// CString conversion failed (path contained a null byte).
    #[error("invalid path (contains null byte): {0}")]
    InvalidPath(String),
}

impl From<rsmpeg::error::RsmpegError> for VideoError {
    fn from(e: rsmpeg::error::RsmpegError) -> Self {
        VideoError::Ffmpeg(e.to_string())
    }
}

impl From<std::ffi::NulError> for VideoError {
    fn from(e: std::ffi::NulError) -> Self {
        VideoError::InvalidPath(e.to_string())
    }
}
