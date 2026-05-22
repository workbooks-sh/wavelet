use super::host::assert_golden_rmse;
use crate::shader::assert::FrameSource;

fn solid(width: u32, height: u32, r: u8, g: u8, b: u8) -> FrameSource {
    let mut pixels = Vec::with_capacity((width * height * 4) as usize);
    for _ in 0..(width * height) {
        pixels.extend_from_slice(&[r, g, b, 255]);
    }
    FrameSource::Rgba8 { width, height, pixels }
}

fn one_pixel_off(width: u32, height: u32, r: u8, g: u8, b: u8, shift: u8) -> FrameSource {
    let mut pixels = Vec::with_capacity((width * height * 4) as usize);
    for i in 0..(width * height) {
        if i == 0 {
            pixels.extend_from_slice(&[
                r.saturating_add(shift),
                g.saturating_add(shift),
                b.saturating_add(shift),
                255,
            ]);
        } else {
            pixels.extend_from_slice(&[r, g, b, 255]);
        }
    }
    FrameSource::Rgba8 { width, height, pixels }
}

#[test]
fn identical_frames_zero_rmse_passes() {
    let frame = solid(8, 8, 100, 150, 200);
    let golden = solid(8, 8, 100, 150, 200);
    let outcome = assert_golden_rmse(frame, golden, 0, 0).expect("dispatch");
    assert!(outcome.passed, "identical → pass: {outcome:?}");
    assert!(outcome.evidence[0].abs() < 1e-5, "RMSE should be 0, got {}", outcome.evidence[0]);
    assert_eq!(outcome.evidence[1] as u32, 0, "no pixels should exceed");
}

#[test]
fn fuzzy_tolerance_allows_small_drift() {
    // one pixel off by 5, max_diff=10, max_pixels=2 → pass.
    let frame = one_pixel_off(8, 8, 100, 100, 100, 5);
    let golden = solid(8, 8, 100, 100, 100);
    let outcome = assert_golden_rmse(frame, golden, 10, 2).expect("dispatch");
    assert!(outcome.passed, "within tolerance: {outcome:?}");
    assert_eq!(outcome.evidence[1] as u32, 0);
}

#[test]
fn large_diff_fails() {
    let frame = solid(8, 8, 0, 0, 0);
    let golden = solid(8, 8, 255, 255, 255);
    let outcome = assert_golden_rmse(frame, golden, 16, 4).expect("dispatch");
    assert!(!outcome.passed, "expected fail, got {outcome:?}");
    assert_eq!(outcome.reason_code, 1);
    assert_eq!(outcome.evidence[1] as u32, 64);
}
