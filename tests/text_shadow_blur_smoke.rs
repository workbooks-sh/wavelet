//! Smoke test for blurred `text-shadow` paint patch (wb-e8jh.1).
//!
//! Renders three text rows side-by-side, each with a different shadow:
//!   1. `text-shadow: 8px 8px 0 magenta` — hard offset, blur 0. Must keep
//!      working (regression guard for wb-o7s0).
//!   2. `text-shadow: 0 0 12px #ffd700` — symmetric glow. Without blur
//!      support, no shadow would render at all; with the new path, we
//!      expect a soft yellow halo extending outside the glyph silhouettes.
//!   3. `text-shadow: 0 4px 16px rgba(0,0,0,0.6)` — the canonical
//!      drop-shadow idiom. We expect dark pixels beneath the glyphs.
//!
//! The probes are deliberately loose — anti-aliasing and font availability
//! make bit-exact baselines brittle. We sample windows and require a
//! threshold count of hits.

use wavelet::render::render_html_to_rgba;

const WIDTH: u32 = 900;
const HEIGHT: u32 = 600;

fn pixel(buf: &[u8], x: u32, y: u32) -> [u8; 4] {
    let i = ((y * WIDTH + x) * 4) as usize;
    [buf[i], buf[i + 1], buf[i + 2], buf[i + 3]]
}

fn is_magenta(p: [u8; 4]) -> bool {
    p[0] > 180 && p[2] > 180 && p[1] < 80 && p[3] > 64
}

fn is_yellow_tint(p: [u8; 4]) -> bool {
    // Gold halo (#ffd700) composited OVER white reads as warm-white:
    // R and G high, B noticeably lower than R. Pure white has R==G==B,
    // so any reasonable gap between R and B indicates yellow tinting.
    let r = p[0] as i32;
    let g = p[1] as i32;
    let b = p[2] as i32;
    r > 200 && g > 180 && (r - b) > 30 && (g - b) > 20
}

fn is_grey_halo(p: [u8; 4]) -> bool {
    // Drop-shadow over white reads as desaturated grey: R≈G≈B, all
    // dimmer than the pure-white background but brighter than full
    // black glyphs. Excludes glyph cores (which are #111) and pure
    // background (255,255,255).
    let r = p[0] as i32;
    let g = p[1] as i32;
    let b = p[2] as i32;
    r > 30 && r < 245
        && g > 30 && g < 245
        && b > 30 && b < 245
        && (r - g).abs() < 12
        && (g - b).abs() < 12
        && (r - b).abs() < 12
}

fn is_white(p: [u8; 4]) -> bool {
    p[0] > 240 && p[1] > 240 && p[2] > 240
}

#[test]
fn text_shadow_blur_smoke() {
    // Three stacked rows, each with a tall enough line-box that we can
    // probe known regions. White background so every shadow color stands
    // out on its own.
    //
    // Row 1 (HARD):  y ~ 40..160,   glyphs offset 8px right/down → shadow
    //                magenta appears around 8px below/right of glyph
    //                outlines. We probe x in 100..240 / y in 110..150
    //                (i.e. below the bottom of the H/A baseline in a 120px
    //                font).
    // Row 2 (GLOW):  y ~ 200..320, symmetric halo around the glyphs;
    //                yellow pixels exist OUTSIDE the glyph silhouette
    //                proper.
    // Row 3 (DROP):  y ~ 360..480, soft dark halo below glyphs.
    let html = r#"<!doctype html><html><head><style>
        html, body { margin: 0; background: #ffffff; width: 100%; height: 100%; }
        .row { font: 900 100px sans-serif; line-height: 1.2; padding: 10px 30px; color: #111111; }
        .hard  { text-shadow: 8px 8px 0 magenta; }
        .glow  { text-shadow: 0 0 12px #ffd700; }
        .drop  { text-shadow: 0 4px 16px rgba(0,0,0,0.6); }
    </style></head><body>
        <div class="row hard">HARD</div>
        <div class="row glow">GLOW</div>
        <div class="row drop">DROP</div>
    </body></html>"#;

    let buf = render_html_to_rgba(html, WIDTH, HEIGHT);
    assert_eq!(buf.len(), (WIDTH * HEIGHT * 4) as usize);

    // ---------- Row 1: hard magenta shadow ----------
    // The shadow is 8px below/right of glyphs in a 100px-tall row.
    // First row's content baseline lives near y ~ 100. Probe a horizontal
    // strip just below the glyphs in screen y ~ 115..145.
    let mut hard_hits = 0;
    for y in (20..150).step_by(2) {
        for x in (40..500).step_by(2) {
            if is_magenta(pixel(&buf, x, y)) {
                hard_hits += 1;
            }
        }
    }
    assert!(
        hard_hits > 50,
        "row 1 (hard shadow): no magenta pixels — \
         the unblurred text-shadow path regressed. hits={hard_hits}"
    );

    // ---------- Row 2: glow ----------
    // Row 2 spans roughly y ~ 150..250 (10px padding + ~120px row 1 +
    // line-height). Probe a fat band; the gold shadow composited over
    // white reads as warm-white (R>>B). Without the new branch the
    // shadow is silently dropped and no yellow tint appears anywhere.
    let mut glow_hits = 0;
    for y in (140..260).step_by(2) {
        for x in (40..500).step_by(2) {
            if is_yellow_tint(pixel(&buf, x, y)) {
                glow_hits += 1;
            }
        }
    }
    assert!(
        glow_hits > 30,
        "row 2 (glow): no yellow-tint pixels — blurred text-shadow did not paint. \
         hits={glow_hits}"
    );

    // ---------- Row 3: drop shadow ----------
    // Row 3 spans roughly y ~ 270..400. A soft dark halo sits below the
    // glyphs (visible at y ~ 370..400). We sample a band that is BELOW
    // the glyph baselines so glyph interiors don't pollute the count;
    // pure white background is rejected by `is_grey_halo`, glyph cores
    // (#111) are too, leaving only the blurred-shadow grey ring.
    let mut drop_hits = 0;
    for y in (370..420).step_by(2) {
        for x in (40..500).step_by(2) {
            if is_grey_halo(pixel(&buf, x, y)) {
                drop_hits += 1;
            }
        }
    }
    assert!(
        drop_hits > 30,
        "row 3 (drop): no grey halo below glyphs — blurred text-shadow did not paint. \
         hits={drop_hits}"
    );

    // ---------- Inverse: between rows must stay white ----------
    // The strip between row 1 and row 2 (well above any drop shadow that
    // could leak in) should mostly be the white background. This
    // catches the failure mode "we painted the shadow image too big or in
    // the wrong place".
    //
    // Pick a 4px-tall strip at y ~ 160-164 in the right margin (x 750+).
    let mut white_total = 0;
    let mut white_hits = 0;
    for y in 160..164 {
        for x in (750..880).step_by(4) {
            white_total += 1;
            if is_white(pixel(&buf, x, y)) {
                white_hits += 1;
            }
        }
    }
    let white_ratio = if white_total > 0 {
        white_hits as f32 / white_total as f32
    } else {
        0.0
    };
    assert!(
        white_ratio > 0.9,
        "right-margin between rows is not predominantly white — \
         a shadow leaked into a region where no text or shadow should be. \
         ratio={white_ratio:.2} hits={white_hits}/{white_total}"
    );

    eprintln!(
        "text-shadow blur smoke: hard={hard_hits} glow={glow_hits} drop={drop_hits} \
         right-margin-white={white_ratio:.2}"
    );
}
