use super::host::{assert_contrast, Region};
use crate::shader::assert::FrameSource;

fn split_black_white(width: u32, height: u32) -> FrameSource {
    let mut pixels = Vec::with_capacity((width * height * 4) as usize);
    for y in 0..height {
        for x in 0..width {
            if x < width / 2 {
                pixels.extend_from_slice(&[0, 0, 0, 255]);
            } else {
                pixels.extend_from_slice(&[255, 255, 255, 255]);
            }
            let _ = y;
        }
    }
    FrameSource::Rgba8 {
        width,
        height,
        pixels,
    }
}

fn near_gray(width: u32, height: u32) -> FrameSource {
    let mut pixels = Vec::with_capacity((width * height * 4) as usize);
    for y in 0..height {
        for x in 0..width {
            let v = if x < width / 2 { 120u8 } else { 132u8 };
            pixels.extend_from_slice(&[v, v, v, 255]);
            let _ = y;
        }
    }
    FrameSource::Rgba8 {
        width,
        height,
        pixels,
    }
}

#[test]
fn black_white_region_meets_aa() {
    let frame = split_black_white(16, 16);
    let region = Region { x: 0.0, y: 0.0, w: 1.0, h: 1.0 };
    let outcome = assert_contrast(frame, region, 4.5).expect("dispatch");
    assert!(outcome.passed, "expected pass, got {outcome:?}");
    assert!(outcome.evidence[0] > 20.0, "CR should be ~21, got {}", outcome.evidence[0]);
}

#[test]
fn near_gray_region_fails_aa() {
    let frame = near_gray(16, 16);
    let region = Region { x: 0.0, y: 0.0, w: 1.0, h: 1.0 };
    let outcome = assert_contrast(frame, region, 4.5).expect("dispatch");
    assert!(!outcome.passed, "expected fail, got {outcome:?}");
    assert_eq!(outcome.reason_code, 1);
    assert!(outcome.evidence[0] < 2.0, "CR should be small, got {}", outcome.evidence[0]);
}
