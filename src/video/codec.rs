//! Codec enum + hardware-decoder probe.

use rsmpeg::ffi;
use serde::{Deserialize, Serialize};

/// Output codec for [`crate::VideoEncoder`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Codec {
    /// H.264 (AVC) via libx264. Universal compatibility. Default.
    H264,
    /// H.265 (HEVC) via libx265. Higher compression than H.264 at the cost of
    /// slower encode and narrower playback support.
    H265,
    /// AV1 via rav1e. ~50% better compression than H.264 at quality parity.
    /// Behind the `av1` Cargo feature; off by default to keep build times low.
    #[cfg(feature = "av1")]
    Av1,
}

impl Codec {
    /// Map to the corresponding FFmpeg codec id.
    pub(crate) fn ffmpeg_id(self) -> ffi::AVCodecID {
        match self {
            Self::H264 => ffi::AV_CODEC_ID_H264,
            Self::H265 => ffi::AV_CODEC_ID_HEVC,
            #[cfg(feature = "av1")]
            Self::Av1 => ffi::AV_CODEC_ID_AV1,
        }
    }

    /// Default codec — H.264 for universal compatibility.
    pub fn default_for_mp4() -> Self {
        Self::H264
    }
}

/// Enumerate hardware-accelerated codecs available in this FFmpeg build.
///
/// Returns codec names. Useful for diagnostics and for selecting between
/// software and HW paths at runtime. Names typically end with `_videotoolbox`
/// (macOS), `_nvenc` / `_nvdec` (NVIDIA), `_qsv` (Intel), or `_vaapi` (Linux).
pub fn hw_decoders() -> Vec<String> {
    let mut out = Vec::new();
    unsafe {
        let mut it: *mut std::ffi::c_void = std::ptr::null_mut();
        loop {
            let codec = ffi::av_codec_iterate(&mut it);
            if codec.is_null() {
                break;
            }
            let name = std::ffi::CStr::from_ptr((*codec).name).to_string_lossy();
            let lower = name.to_lowercase();
            if lower.contains("videotoolbox")
                || lower.contains("_vt")
                || lower.contains("nvenc")
                || lower.contains("nvdec")
                || lower.contains("vaapi")
                || lower.contains("qsv")
            {
                out.push(name.into_owned());
            }
        }
    }
    out.sort();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_h264() {
        assert_eq!(Codec::default_for_mp4(), Codec::H264);
    }

    #[test]
    fn hw_decoders_returns_some_list() {
        let decoders = hw_decoders();
        // We don't assert specific names — depends on FFmpeg build.
        // Just ensure the call succeeds and returns a sorted unique list.
        for w in decoders.windows(2) {
            assert!(w[0] <= w[1], "decoders not sorted: {:?}", decoders);
        }
    }

    #[test]
    fn codec_serializes_snake_case() {
        let s = serde_json::to_string(&Codec::H264).unwrap();
        assert_eq!(s, "\"h264\"");
    }
}
