use super::host::assert_sobel_edge_density;
use crate::shader::assert::contrast_in_region::Region;
use crate::shader::assert::FrameSource;

fn solid(width: u32, height: u32, r: u8, g: u8, b: u8) -> FrameSource {
    let mut pixels = Vec::with_capacity((width * height * 4) as usize);
    for _ in 0..(width * height) {
        pixels.extend_from_slice(&[r, g, b, 255]);
    }
    FrameSource::Rgba8 { width, height, pixels }
}

fn vertical_stripes(width: u32, height: u32) -> FrameSource {
    let mut pixels = Vec::with_capacity((width * height * 4) as usize);
    for y in 0..height {
        for x in 0..width {
            let v: u8 = if (x / 2) % 2 == 0 { 255 } else { 0 };
            pixels.extend_from_slice(&[v, v, v, 255]);
            let _ = y;
        }
    }
    FrameSource::Rgba8 { width, height, pixels }
}

#[test]
fn solid_color_has_no_edges() {
    let frame = solid(16, 16, 200, 200, 200);
    let region = Region { x: 0.0, y: 0.0, w: 1.0, h: 1.0 };
    let outcome = assert_sobel_edge_density(frame, region, 0.1, 0.1).expect("dispatch");
    assert!(!outcome.passed, "solid color should fail: {outcome:?}");
    assert_eq!(outcome.evidence[1] as u32, 0, "no edges");
}

#[test]
fn stripes_have_dense_edges() {
    let frame = vertical_stripes(16, 16);
    let region = Region { x: 0.0, y: 0.0, w: 1.0, h: 1.0 };
    let outcome = assert_sobel_edge_density(frame, region, 0.5, 0.4).expect("dispatch");
    assert!(outcome.passed, "stripes should pass: {outcome:?}");
    assert!(outcome.evidence[0] > 0.4, "density should be high, got {}", outcome.evidence[0]);
}
