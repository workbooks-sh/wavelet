//! Smoke test for `background-clip: text` glyph-mask paint patch (wb-wudl).
//!
//! Renders a single-frame HTML scene with a gradient h1 that uses
//! `-webkit-background-clip: text; color: transparent`. The acceptance
//! criterion: the gradient must paint INSIDE the letterforms (so we see
//! both magenta-ish and cyan-ish pixels somewhere in the text rect) and
//! the rectangle around the text must NOT have been painted (so the
//! solid background shows through, not the gradient).
//!
//! Pixel sampling instead of exact-match — anti-aliased glyph edges and
//! font availability across platforms makes a bit-identical baseline
//! brittle. We probe small windows in the regions where each constraint
//! should hold.

use wavelet::render::render_html_to_rgba;

const WIDTH: u32 = 600;
const HEIGHT: u32 = 200;

fn pixel(buf: &[u8], x: u32, y: u32) -> [u8; 4] {
    let i = ((y * WIDTH + x) * 4) as usize;
    [buf[i], buf[i + 1], buf[i + 2], buf[i + 3]]
}

fn is_magentaish(p: [u8; 4]) -> bool {
    // gradient start: magenta = (255, 0, 255). Allow slack for anti-aliasing.
    p[0] > 140 && p[2] > 140 && p[1] < 120
}

fn is_cyanish(p: [u8; 4]) -> bool {
    // gradient end: cyan = (0, 255, 255).
    p[1] > 140 && p[2] > 140 && p[0] < 120
}

fn is_background_grey(p: [u8; 4]) -> bool {
    // page background = #202020 (32,32,32). Be generous: anything dim and
    // close-to-neutral counts.
    p[0] < 80 && p[1] < 80 && p[2] < 80 && (p[0] as i32 - p[1] as i32).abs() < 30
}

#[test]
fn background_clip_text_masks_gradient_to_glyphs() {
    // Big bold sans-serif, gradient L→R magenta→cyan, text-clip on.
    // Background of the page is a dark grey so the surrounding area is
    // clearly distinguishable from BOTH gradient stops.
    // Inline style — the sniff only fires on inline `style="..."` because
    // Stylo's servo build does not parse `background-clip: text` at the
    // sheet-rule layer. The canonical idiom from the acceptance criterion
    // is inline anyway.
    let html = format!(
        r#"<!doctype html><html><head><style>
            html, body {{ margin: 0; background: #202020; width: 100%; height: 100%; }}
        </style></head><body><h1 style="margin:0;padding:30px 40px;font:900 120px sans-serif;line-height:1;background:linear-gradient(90deg, #ff00ff 0%, #00ffff 100%);-webkit-background-clip:text;background-clip:text;color:transparent">GRADIENT</h1></body></html>"#
    );

    let buf = render_html_to_rgba(&html, WIDTH, HEIGHT);
    assert_eq!(buf.len(), (WIDTH * HEIGHT * 4) as usize);

    // Probe a vertical band through the text. Sample many y-rows across
    // the painted text rect and a few across the horizontal extent.
    let xs_left: Vec<u32> = (60..160).step_by(5).collect();
    let xs_right: Vec<u32> = (440..540).step_by(5).collect();
    let ys: Vec<u32> = (60..160).step_by(5).collect();

    let mut left_magenta_hits = 0;
    for &x in &xs_left {
        for &y in &ys {
            if is_magentaish(pixel(&buf, x, y)) {
                left_magenta_hits += 1;
            }
        }
    }
    assert!(
        left_magenta_hits > 0,
        "no magenta-ish pixels in left half of text area — \
         gradient did not paint inside glyphs (expected ≥1 hit)"
    );

    let mut right_cyan_hits = 0;
    for &x in &xs_right {
        for &y in &ys {
            if is_cyanish(pixel(&buf, x, y)) {
                right_cyan_hits += 1;
            }
        }
    }
    assert!(
        right_cyan_hits > 0,
        "no cyan-ish pixels in right half of text area — \
         gradient did not paint inside glyphs (expected ≥1 hit)"
    );

    // Crucial inverse check: the band above/below the text (still within
    // the h1's bounding box, but outside the glyph silhouettes) must show
    // the page background, not the gradient. Without the clip, a solid
    // gradient rectangle would dominate this region.
    //
    // y in 5..25 covers the top padding strip of the h1 (30px of padding
    // before any glyphs). Page background dominates here only if the clip
    // is active.
    let mut top_padding_grey_hits = 0;
    let mut top_padding_total = 0;
    for x in (20..580).step_by(20) {
        for y in (5..25).step_by(5) {
            top_padding_total += 1;
            if is_background_grey(pixel(&buf, x, y)) {
                top_padding_grey_hits += 1;
            }
        }
    }
    // Allow a small number of stray gradient samples (sub-pixel rendering
    // can produce dim edges near padding). But the vast majority must be
    // background.
    let grey_ratio = top_padding_grey_hits as f32 / top_padding_total as f32;
    assert!(
        grey_ratio > 0.8,
        "top padding strip is not predominantly page-background — \
         the gradient looks unclipped (grey_ratio = {grey_ratio:.2}; \
         hits={top_padding_grey_hits}/{top_padding_total})"
    );

    eprintln!(
        "background-clip:text smoke: left magenta hits={left_magenta_hits}, \
         right cyan hits={right_cyan_hits}, top-padding grey ratio={grey_ratio:.2}"
    );
}

#[test]
fn background_clip_text_works_from_stylesheet_rule() {
    // Same acceptance as the inline-style test but the `background-clip:
    // text` declaration lives in a <style> sheet rule, not inline. This
    // is the wb-e8jh.7 path — only works because vendor/stylo locally
    // adds `extra_servo_values = ["text"]` so Stylo's parser keeps the
    // Text variant in the computed-value enum.
    let html = format!(
        r#"<!doctype html><html><head><style>
            html, body {{ margin: 0; background: #202020; width: 100%; height: 100%; }}
            h1 {{
                margin: 0;
                padding: 30px 40px;
                font: 900 120px sans-serif;
                line-height: 1;
                background: linear-gradient(90deg, #ff00ff 0%, #00ffff 100%);
                -webkit-background-clip: text;
                background-clip: text;
                color: transparent;
            }}
        </style></head><body><h1>GRADIENT</h1></body></html>"#
    );

    let buf = render_html_to_rgba(&html, WIDTH, HEIGHT);
    assert_eq!(buf.len(), (WIDTH * HEIGHT * 4) as usize);

    let xs_left: Vec<u32> = (60..160).step_by(5).collect();
    let xs_right: Vec<u32> = (440..540).step_by(5).collect();
    let ys: Vec<u32> = (60..160).step_by(5).collect();

    let mut left_magenta_hits = 0;
    for &x in &xs_left {
        for &y in &ys {
            if is_magentaish(pixel(&buf, x, y)) {
                left_magenta_hits += 1;
            }
        }
    }
    let mut right_cyan_hits = 0;
    for &x in &xs_right {
        for &y in &ys {
            if is_cyanish(pixel(&buf, x, y)) {
                right_cyan_hits += 1;
            }
        }
    }
    let mut top_padding_grey_hits = 0;
    let mut top_padding_total = 0;
    for x in (20..580).step_by(20) {
        for y in (5..25).step_by(5) {
            top_padding_total += 1;
            if is_background_grey(pixel(&buf, x, y)) {
                top_padding_grey_hits += 1;
            }
        }
    }
    let grey_ratio = top_padding_grey_hits as f32 / top_padding_total as f32;

    assert!(left_magenta_hits > 0, "stylesheet form: no magenta in left text band");
    assert!(right_cyan_hits > 0, "stylesheet form: no cyan in right text band");
    assert!(
        grey_ratio > 0.8,
        "stylesheet form: top-padding strip not predominantly page-background \
         (grey_ratio = {grey_ratio:.2}; hits={top_padding_grey_hits}/{top_padding_total}) \
         — Stylo accessor path may not be firing"
    );

    eprintln!(
        "background-clip:text via <style>: left magenta hits={left_magenta_hits}, \
         right cyan hits={right_cyan_hits}, top-padding grey ratio={grey_ratio:.2}"
    );
}
