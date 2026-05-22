use super::host::assert_motion;
use crate::shader::assert::FrameSource;

fn solid(width: u32, height: u32, r: u8, g: u8, b: u8) -> FrameSource {
    let mut pixels = Vec::with_capacity((width * height * 4) as usize);
    for _ in 0..(width * height) {
        pixels.extend_from_slice(&[r, g, b, 255]);
    }
    FrameSource::Rgba8 { width, height, pixels }
}

#[test]
fn identical_frames_zero_motion() {
    let a = solid(8, 8, 128, 128, 128);
    let b = solid(8, 8, 128, 128, 128);
    let outcome = assert_motion(a, b, 0.0).expect("dispatch");
    assert!(outcome.passed, "expected pass at zero floor, got {outcome:?}");
    let mean = outcome.evidence[8];
    assert!(mean.abs() < 1e-4, "mean motion should be ~0, got {}", mean);
}

#[test]
fn full_swap_high_motion_passes_low_floor() {
    let a = solid(8, 8, 0, 0, 0);
    let b = solid(8, 8, 255, 255, 255);
    let outcome = assert_motion(a, b, 0.5).expect("dispatch");
    assert!(outcome.passed, "expected pass, got {outcome:?}");
    let mean = outcome.evidence[8];
    assert!(mean > 0.9, "mean motion should be ~1.0, got {}", mean);
}

#[test]
fn small_diff_fails_high_floor() {
    let a = solid(8, 8, 100, 100, 100);
    let b = solid(8, 8, 105, 105, 105);
    let outcome = assert_motion(a, b, 0.5).expect("dispatch");
    assert!(!outcome.passed, "expected fail, got {outcome:?}");
    assert_eq!(outcome.reason_code, 1);
}
