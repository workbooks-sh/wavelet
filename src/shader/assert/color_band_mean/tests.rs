use super::host::{assert_color_band_mean, HslTarget};
use crate::shader::assert::contrast_in_region::Region;
use crate::shader::assert::FrameSource;

fn solid(width: u32, height: u32, r: u8, g: u8, b: u8) -> FrameSource {
    let mut pixels = Vec::with_capacity((width * height * 4) as usize);
    for _ in 0..(width * height) {
        pixels.extend_from_slice(&[r, g, b, 255]);
    }
    FrameSource::Rgba8 { width, height, pixels }
}

#[test]
fn solid_red_meets_red_target() {
    let frame = solid(16, 16, 255, 0, 0);
    let region = Region { x: 0.0, y: 0.0, w: 1.0, h: 1.0 };
    let target = HslTarget { h: 0.0, s: 1.0, l: 0.5, tolerance: 0.05 };
    let outcome = assert_color_band_mean(frame, region, target).expect("dispatch");
    assert!(outcome.passed, "pure red should match red target: {outcome:?}");
    assert!(outcome.evidence[1] > 0.95, "saturation should be ~1.0");
    assert!((outcome.evidence[2] - 0.5).abs() < 0.05, "lightness should be ~0.5");
}

#[test]
fn solid_blue_misses_red_target() {
    let frame = solid(16, 16, 0, 0, 255);
    let region = Region { x: 0.0, y: 0.0, w: 1.0, h: 1.0 };
    let target = HslTarget { h: 0.0, s: 1.0, l: 0.5, tolerance: 0.05 };
    let outcome = assert_color_band_mean(frame, region, target).expect("dispatch");
    assert!(!outcome.passed, "blue should not match red target: {outcome:?}");
    assert_eq!(outcome.reason_code, 1);
}
