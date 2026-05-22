//! Backend-shared utilities — image format sniffing + minimal base64
//! codec, plus the ffmpeg-based last-frame extractor used by the
//! inter-shot frame-chaining path (wb-6msu). Kept dep-free so the
//! runtime stays small and these helpers can be reused by the CLI
//! layer to normalize local file paths into `data:` URLs before
//! adapters see them.

use crate::backends::BackendError;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Sniff the image file extension from magic bytes. Falls back to png
/// when unrecognized.
pub fn sniff_image_ext(bytes: &[u8]) -> &'static str {
    if bytes.len() >= 3 && &bytes[..3] == b"\xff\xd8\xff" {
        "jpg"
    } else if bytes.len() >= 8 && &bytes[..8] == b"\x89PNG\r\n\x1a\n" {
        "png"
    } else if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        "webp"
    } else if bytes.len() >= 6 && (&bytes[..6] == b"GIF87a" || &bytes[..6] == b"GIF89a") {
        "gif"
    } else {
        "png"
    }
}

/// Pick an audio file extension from a MIME / content-type string.
/// Case-insensitive. Falls back to "mp3" on unknown — the most common
/// audio container is the safest default downstream (ffmpeg can
/// transcode from "mp3" extension regardless of true format better
/// than from an unknown extension like "audio").
pub fn pick_audio_ext_from_mime(mime: &str) -> &'static str {
    let lower = mime.to_ascii_lowercase();
    if lower.contains("mpeg") || lower.contains("mp3") {
        "mp3"
    } else if lower.contains("wav") {
        "wav"
    } else if lower.contains("ogg") {
        "ogg"
    } else if lower.contains("flac") {
        "flac"
    } else {
        "mp3"
    }
}

/// Pick an image file extension from a MIME / content-type string.
/// Accepts `Option<&str>` so callers that thread through HTTP
/// `Content-Type` headers don't have to unwrap. Falls back to "png"
/// on unknown or missing input. Case-insensitive.
pub fn pick_image_ext_from_mime(mime: Option<&str>) -> &'static str {
    let Some(ct) = mime else { return "png"; };
    let lower = ct.to_ascii_lowercase();
    if lower.contains("jpeg") || lower.contains("jpg") {
        "jpg"
    } else if lower.contains("webp") {
        "webp"
    } else {
        "png"
    }
}

/// Map a sniffed image extension to its MIME type for `data:` URLs.
pub fn ext_to_mime(ext: &str) -> &'static str {
    match ext {
        "jpg" | "jpeg" => "image/jpeg",
        "webp" => "image/webp",
        "gif" => "image/gif",
        _ => "image/png",
    }
}

/// Minimal base64 decoder (no external dep). Handles the standard
/// alphabet with optional `=` padding.
pub fn base64_decode(s: &str) -> Result<Vec<u8>, String> {
    let mut out = Vec::with_capacity(s.len() * 3 / 4);
    let mut buf: u32 = 0;
    let mut bits = 0u8;
    for c in s.bytes() {
        let v: u8 = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            b'=' | b'\n' | b'\r' | b' ' | b'\t' => continue,
            other => return Err(format!("invalid base64 char {:#02x}", other)),
        };
        buf = (buf << 6) | v as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((buf >> bits) as u8);
            buf &= (1 << bits) - 1;
        }
    }
    Ok(out)
}

/// Minimal base64 encoder (no external dep). Standard alphabet with
/// `=` padding.
pub fn base64_encode(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] =
        b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity((bytes.len() + 2) / 3 * 4);
    let mut i = 0;
    while i + 3 <= bytes.len() {
        let n = ((bytes[i] as u32) << 16) | ((bytes[i + 1] as u32) << 8) | (bytes[i + 2] as u32);
        out.push(ALPHABET[((n >> 18) & 0x3f) as usize] as char);
        out.push(ALPHABET[((n >> 12) & 0x3f) as usize] as char);
        out.push(ALPHABET[((n >> 6) & 0x3f) as usize] as char);
        out.push(ALPHABET[(n & 0x3f) as usize] as char);
        i += 3;
    }
    let rem = bytes.len() - i;
    if rem == 1 {
        let n = (bytes[i] as u32) << 16;
        out.push(ALPHABET[((n >> 18) & 0x3f) as usize] as char);
        out.push(ALPHABET[((n >> 12) & 0x3f) as usize] as char);
        out.push('=');
        out.push('=');
    } else if rem == 2 {
        let n = ((bytes[i] as u32) << 16) | ((bytes[i + 1] as u32) << 8);
        out.push(ALPHABET[((n >> 18) & 0x3f) as usize] as char);
        out.push(ALPHABET[((n >> 12) & 0x3f) as usize] as char);
        out.push(ALPHABET[((n >> 6) & 0x3f) as usize] as char);
        out.push('=');
    }
    out
}

/// Extract the final frame of an MP4 as a PNG. Used by the
/// frame-chaining executor: shot N's last frame becomes shot N+1's
/// `start_image_url` so the cut between them looks continuous.
///
/// `-sseof -1` seeks to one second before EOF then walks forward
/// (decoding the keyframe + GOP), which is fast and "close enough" —
/// the visible end-frame is within ~1s of the absolute EOF. `-update 1`
/// emits a single image; `-q:v 1` picks the highest-quality png.
///
/// Returns the on-disk PNG path on success.
pub fn extract_last_frame(video_path: &Path) -> Result<PathBuf, BackendError> {
    if !video_path.exists() {
        return Err(BackendError::InvalidRequest(format!(
            "extract_last_frame: video not found at {}",
            video_path.display()
        )));
    }

    let stem = video_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("frame");
    let parent = video_path
        .parent()
        .unwrap_or_else(|| Path::new("."));
    let out_path = parent.join(format!("{stem}.last.png"));

    let output = Command::new("ffmpeg")
        .args(["-y", "-sseof", "-1", "-i"])
        .arg(video_path)
        .args(["-update", "1", "-q:v", "1", "-frames:v", "1"])
        .arg(&out_path)
        .output()
        .map_err(|e| {
            BackendError::Transport(format!(
                "ffmpeg spawn failed (install with `brew install ffmpeg`): {e}"
            ))
        })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(BackendError::Transport(format!(
            "ffmpeg failed extracting last frame from {}: {}",
            video_path.display(),
            stderr.lines().rev().take(3).collect::<Vec<_>>().join(" | ")
        )));
    }

    if !out_path.exists() {
        return Err(BackendError::Transport(format!(
            "ffmpeg reported success but {} doesn't exist",
            out_path.display()
        )));
    }

    Ok(out_path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sniff_png() {
        assert_eq!(sniff_image_ext(b"\x89PNG\r\n\x1a\n....."), "png");
    }

    #[test]
    fn sniff_jpeg() {
        assert_eq!(sniff_image_ext(b"\xff\xd8\xff....."), "jpg");
    }

    #[test]
    fn sniff_webp() {
        assert_eq!(sniff_image_ext(b"RIFF\0\0\0\0WEBP...."), "webp");
    }

    #[test]
    fn base64_roundtrip() {
        for sample in [
            &b""[..],
            b"f",
            b"fo",
            b"foo",
            b"foob",
            b"fooba",
            b"foobar",
            b"\x89PNG\r\n\x1a\n\x00\x00\x00\rIHDR",
        ] {
            let encoded = base64_encode(sample);
            let decoded = base64_decode(&encoded).unwrap();
            assert_eq!(decoded, sample, "roundtrip failed for {:?}", sample);
        }
    }

    #[test]
    fn ext_mime_mapping() {
        assert_eq!(ext_to_mime("png"), "image/png");
        assert_eq!(ext_to_mime("jpg"), "image/jpeg");
        assert_eq!(ext_to_mime("jpeg"), "image/jpeg");
        assert_eq!(ext_to_mime("webp"), "image/webp");
        assert_eq!(ext_to_mime("gif"), "image/gif");
        assert_eq!(ext_to_mime("unknown"), "image/png");
    }

    #[test]
    fn pick_audio_ext_from_mime_handles_common_types() {
        assert_eq!(pick_audio_ext_from_mime("audio/mpeg"), "mp3");
        assert_eq!(pick_audio_ext_from_mime("audio/mp3"), "mp3");
        assert_eq!(pick_audio_ext_from_mime("audio/wav"), "wav");
        assert_eq!(pick_audio_ext_from_mime("audio/ogg"), "ogg");
        assert_eq!(pick_audio_ext_from_mime("audio/flac"), "flac");
        assert_eq!(pick_audio_ext_from_mime("AUDIO/MPEG"), "mp3");
        assert_eq!(pick_audio_ext_from_mime("application/octet-stream"), "mp3");
    }

    #[test]
    fn pick_image_ext_from_mime_handles_common_types() {
        assert_eq!(pick_image_ext_from_mime(Some("image/png")), "png");
        assert_eq!(pick_image_ext_from_mime(Some("image/jpeg")), "jpg");
        assert_eq!(pick_image_ext_from_mime(Some("image/webp")), "webp");
        assert_eq!(pick_image_ext_from_mime(Some("IMAGE/JPEG")), "jpg");
        assert_eq!(pick_image_ext_from_mime(Some("application/octet-stream")), "png");
        assert_eq!(pick_image_ext_from_mime(None), "png");
    }

    #[test]
    fn extract_last_frame_rejects_missing_file() {
        let err = extract_last_frame(Path::new("/nonexistent/missing-xyz123.mp4")).unwrap_err();
        match err {
            BackendError::InvalidRequest(msg) => assert!(msg.contains("not found")),
            other => panic!("expected InvalidRequest, got {other:?}"),
        }
    }
}
