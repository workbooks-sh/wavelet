//! RgbaFrame — owned RGBA8 pixel buffer with dimensions.

/// One frame of RGBA8 pixel data with its dimensions.
///
/// Tightly packed (no row stride padding). Layout: row 0 first, then row 1, etc.
/// Within each row: R G B A R G B A …
#[derive(Debug, Clone)]
pub struct RgbaFrame {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// Raw pixel data, `width * height * 4` bytes.
    pub pixels: Vec<u8>,
}

impl RgbaFrame {
    /// Create a new RGBA frame from raw pixels. Panics if `pixels.len()` doesn't
    /// match `width * height * 4`.
    pub fn new(width: u32, height: u32, pixels: Vec<u8>) -> Self {
        let expected = (width as usize) * (height as usize) * 4;
        assert_eq!(
            pixels.len(),
            expected,
            "RgbaFrame pixel buffer size mismatch: got {} bytes, expected {} for {}x{}",
            pixels.len(),
            expected,
            width,
            height
        );
        Self {
            width,
            height,
            pixels,
        }
    }

    /// Allocate a black opaque RGBA frame.
    pub fn black(width: u32, height: u32) -> Self {
        let mut pixels = vec![0u8; (width as usize) * (height as usize) * 4];
        // Set alpha to 255.
        for i in (3..pixels.len()).step_by(4) {
            pixels[i] = 255;
        }
        Self {
            width,
            height,
            pixels,
        }
    }

    /// Quick hash for determinism tests / regression checks. FNV-1a 64.
    pub fn pixel_hash(&self) -> u64 {
        let mut h: u64 = 0xcbf29ce484222325;
        for &b in &self.pixels {
            h ^= b as u64;
            h = h.wrapping_mul(0x100000001b3);
        }
        h
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn black_frame_is_opaque() {
        let f = RgbaFrame::black(2, 2);
        assert_eq!(f.pixels.len(), 16);
        for i in (3..16).step_by(4) {
            assert_eq!(f.pixels[i], 255);
        }
        for i in (0..16).step_by(4) {
            assert_eq!(f.pixels[i], 0);
            assert_eq!(f.pixels[i + 1], 0);
            assert_eq!(f.pixels[i + 2], 0);
        }
    }

    #[test]
    fn hash_distinguishes_frames() {
        let a = RgbaFrame::black(2, 2);
        let mut b = RgbaFrame::black(2, 2);
        b.pixels[0] = 1;
        assert_ne!(a.pixel_hash(), b.pixel_hash());
    }

    #[test]
    #[should_panic]
    fn new_rejects_wrong_size() {
        RgbaFrame::new(2, 2, vec![0; 10]); // expected 16 bytes
    }
}
