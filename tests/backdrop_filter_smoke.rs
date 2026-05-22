//! Smoke test for `backdrop-filter` paint coverage (wb-3v87).
//!
//! Two scenes, one assertion per scene:
//!
//!   1. `backdrop-filter: grayscale(1)` over a vivid magenta-to-cyan
//!      gradient. Inside the rect: every sampled pixel must satisfy
//!      R == G == B (within a tolerance — anti-aliasing and color
//!      math drift can leave a 1-2 LSB gap). Outside the rect on the
//!      gradient: R != B (the magenta side is high-R/low-B, the cyan
//!      side is low-R/high-B). Cleanest possible probe — doesn't
//!      depend on getting the blur kernel exactly right.
//!
//!   2. `backdrop-filter: blur(20px)` over the same gradient. Inside
//!      the rect we expect the magenta + cyan to smear together, so
//!      the RGB stddev across samples inside the rect should be much
//!      LOWER than across an equivalent span of the unblurred
//!      gradient outside the rect.
//!
//! Both scenes use a fully-transparent rect (no own background) so the
//! probe reads the backdrop, not the element's own paint.
//!
//! If either assertion fails, backdrop-filter paint coverage is broken
//! and wb-3v87 must reopen.

use wavelet::render::render_html_to_rgba;

const WIDTH: u32 = 600;
const HEIGHT: u32 = 400;

fn pixel(buf: &[u8], x: u32, y: u32) -> [u8; 4] {
    let i = ((y * WIDTH + x) * 4) as usize;
    [buf[i], buf[i + 1], buf[i + 2], buf[i + 3]]
}

fn stddev(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    let mean = samples.iter().sum::<f32>() / samples.len() as f32;
    let var = samples.iter().map(|s| (s - mean).powi(2)).sum::<f32>() / samples.len() as f32;
    var.sqrt()
}

#[test]
fn backdrop_filter_grayscale_smoke() {
    // The gradient strip spans the full width: pure magenta on the left,
    // pure cyan on the right. The rect sits centered, 200x150, with a
    // fully transparent background so what we sample inside is the
    // filtered backdrop.
    let html = r#"<!doctype html><html><head><style>
        html, body { margin: 0; padding: 0; width: 100%; height: 100%; }
        body {
            background: linear-gradient(to right, magenta, cyan);
        }
        .glass {
            position: absolute;
            left: 200px;
            top: 125px;
            width: 200px;
            height: 150px;
            background: transparent;
            backdrop-filter: grayscale(1);
        }
    </style></head><body><div class="glass"></div></body></html>"#;

    let buf = render_html_to_rgba(html, WIDTH, HEIGHT);
    assert_eq!(buf.len(), (WIDTH * HEIGHT * 4) as usize);

    // Probe inside the rect: x in [210, 390], y in [135, 265].
    // Every pixel must read close to R==G==B.
    let mut inside_max_spread: i32 = 0;
    let mut inside_samples = 0;
    for y in (140..260).step_by(15) {
        for x in (215..385).step_by(15) {
            let p = pixel(&buf, x, y);
            let r = p[0] as i32;
            let g = p[1] as i32;
            let b = p[2] as i32;
            let spread = (r - g).abs().max((g - b).abs()).max((r - b).abs());
            inside_max_spread = inside_max_spread.max(spread);
            inside_samples += 1;
        }
    }
    assert!(inside_samples > 30, "not enough inside samples: {inside_samples}");
    // Grayscale should drive R==G==B; allow a few LSB of color math drift.
    assert!(
        inside_max_spread <= 6,
        "backdrop-filter: grayscale(1) did not desaturate the backdrop. \
         Max RGB spread inside rect = {inside_max_spread} (expected <= 6 over {inside_samples} samples)"
    );

    // Probe outside the rect on the gradient: y around 50 (well above
    // the rect). The gradient runs magenta(R=255,G=0,B=255) → cyan
    // (R=0,G=255,B=255), so the discriminator is R-G: strongly positive
    // on the magenta side, strongly negative on the cyan side.
    let left_pixel = pixel(&buf, 30, 50);
    let right_pixel = pixel(&buf, 570, 50);
    let left_rg = left_pixel[0] as i32 - left_pixel[1] as i32;
    let right_rg = right_pixel[0] as i32 - right_pixel[1] as i32;
    assert!(
        left_rg > 100,
        "left of rect should be magenta-dominant: pixel={left_pixel:?} R-G={left_rg}"
    );
    assert!(
        right_rg < -100,
        "right of rect should be cyan-dominant: pixel={right_pixel:?} R-G={right_rg}"
    );

    eprintln!(
        "backdrop-filter grayscale smoke: inside max RGB spread = {inside_max_spread} \
         over {inside_samples} samples; outside L(R-G)={left_rg} R(R-G)={right_rg}"
    );
}

#[test]
fn backdrop_filter_blur_smoke() {
    // Vertical stripes (10px wide alternating magenta / yellow) give the
    // blur kernel actual high-frequency content to smear. A linear
    // gradient is a bad probe — blur is roughly a no-op on smooth
    // continuous gradients (local average ≈ local value).
    //
    // Inside the rect: a 20px blur should completely smear the 10px
    // stripes into a roughly uniform orange. Outside: stripes stay
    // crisp.
    let html = r#"<!doctype html><html><head><style>
        html, body { margin: 0; padding: 0; width: 100%; height: 100%; }
        body {
            background:
                repeating-linear-gradient(
                    to right,
                    magenta 0px,
                    magenta 10px,
                    yellow 10px,
                    yellow 20px
                );
        }
        .glass {
            position: absolute;
            left: 200px;
            top: 125px;
            width: 200px;
            height: 150px;
            background: transparent;
            backdrop-filter: blur(20px);
        }
    </style></head><body><div class="glass"></div></body></html>"#;

    let buf = render_html_to_rgba(html, WIDTH, HEIGHT);
    assert_eq!(buf.len(), (WIDTH * HEIGHT * 4) as usize);

    // Sample on a horizontal scan across the rect width. The stripe
    // pattern is magenta(R+B, G=0) and yellow(R+G, B=0), so the G
    // channel alone is the cleanest discriminator: 0 on magenta
    // columns, ~255 on yellow columns. A 20px blur smears the G
    // channel toward ~128 everywhere inside the rect, collapsing the
    // stddev. Outside the rect (above), the stripes stay sharp.
    let mut inside_g: Vec<f32> = Vec::new();
    // Use step 1 for fine-grained sampling — we want to catch every
    // stripe transition.
    for x in 215..385 {
        let p = pixel(&buf, x, 200);
        inside_g.push(p[1] as f32);
    }
    let mut outside_g: Vec<f32> = Vec::new();
    for x in 215..385 {
        let p = pixel(&buf, x, 50);
        outside_g.push(p[1] as f32);
    }

    let inside_std = stddev(&inside_g);
    let outside_std = stddev(&outside_g);
    // The blur should compress the (R-G) range substantially. Demand
    // at least 2× compression — a generous margin given numeric drift.
    assert!(
        inside_std * 2.0 < outside_std,
        "backdrop-filter: blur(20px) did not smear the backdrop. \
         inside_std(G)={inside_std:.1} outside_std(G)={outside_std:.1} \
         (expected inside*2 < outside)"
    );

    // Sanity: the blurred backdrop should still have alpha (it's an
    // image fill). A solid black or fully-transparent rect would mean
    // we painted something wrong.
    let center = pixel(&buf, 300, 200);
    assert!(center[3] > 200, "blurred backdrop center pixel is transparent: {center:?}");
    let center_brightness = center[0] as u32 + center[1] as u32 + center[2] as u32;
    assert!(
        center_brightness > 60,
        "blurred backdrop center pixel is too dark: {center:?} (sum={center_brightness})"
    );

    eprintln!(
        "backdrop-filter blur smoke: inside_std(G)={inside_std:.1} \
         outside_std(G)={outside_std:.1} ratio={:.2}",
        outside_std / inside_std.max(0.001)
    );
}
