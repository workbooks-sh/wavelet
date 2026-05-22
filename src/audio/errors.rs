//! wavelet-audio errors.

use thiserror::Error;

/// Audio crate errors.
#[derive(Debug, Error)]
pub enum AudioError {
    /// I/O error opening or reading an audio file.
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    /// symphonia error decoding the source.
    #[error("decode error: {0}")]
    Decode(String),

    /// rubato error resampling.
    #[error("resample error: {0}")]
    Resample(String),

    /// File format isn't recognized by symphonia.
    #[error("unsupported audio format: {0}")]
    UnsupportedFormat(String),

    /// Duplicate cue id at registration time.
    #[error("duplicate cue id '{0}'")]
    DuplicateCue(String),
}

impl From<symphonia::core::errors::Error> for AudioError {
    fn from(e: symphonia::core::errors::Error) -> Self {
        Self::Decode(e.to_string())
    }
}

impl From<rubato::ResampleError> for AudioError {
    fn from(e: rubato::ResampleError) -> Self {
        Self::Resample(e.to_string())
    }
}

impl From<rubato::ResamplerConstructionError> for AudioError {
    fn from(e: rubato::ResamplerConstructionError) -> Self {
        Self::Resample(e.to_string())
    }
}
