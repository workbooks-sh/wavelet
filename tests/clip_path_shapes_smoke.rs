//! Smoke test for the extended `clip-path` shape coverage (wb-e4zf).
//!
//! Renders five 200×200 colored divs, each clipped to a different shape:
//!   row 0 — `inset(40px round 20px)` (rounded inset rectangle)
//!   row 1 — `ellipse(80px 40px at 100px 100px)` (off-aspect ellipse)
//!   row 2 — `path('M 0 0 L 200 0 L 100 200 Z')` (triangle via SVG path)
//!   row 3 — `xywh(40px 40px 120px 120px)` (inset shorthand)
//!   row 4 — `rect(40px 160px 160px 40px)` (rect shorthand)
//!
//! Each row is laid out at a known y offset; we probe pixels that should be
//! INSIDE the clipped region (must show the fill color) and pixels OUTSIDE
//! (must show the page background). Thresholds are deliberately generous —
//! the goal is to detect "clip not applied at all" or "clip applied to the
//! wrong shape", not pixel-perfect boundary accuracy.
//!
//! Compare against the existing `clip-path: circle()` Tree Runner eval
//! (closed 18/18); same coverage discipline.

use wavelet::render::render_html_to_rgba;

const WIDTH: u32 = 200;
const HEIGHT: u32 = 1000;
const ROW_H: u32 = 200;

fn pixel(buf: &[u8], x: u32, y: u32) -> [u8; 4] {
    let i = ((y * WIDTH + x) * 4) as usize;
    [buf[i], buf[i + 1], buf[i + 2], buf[i + 3]]
}

fn is_fill(p: [u8; 4]) -> bool {
    // #ff5050 with anti-aliasing slack
    p[0] > 180 && p[1] < 160 && p[2] < 160 && p[1].abs_diff(p[2]) < 60
}

fn is_bg(p: [u8; 4]) -> bool {
    // anything close to #101010
    p[0] < 60 && p[1] < 60 && p[2] < 60
}

fn probe_inside(buf: &[u8], pts: &[(u32, u32)], label: &str) {
    let mut hits = 0;
    for &(x, y) in pts {
        if is_fill(pixel(buf, x, y)) {
            hits += 1;
        }
    }
    assert!(
        hits as f32 / pts.len() as f32 > 0.6,
        "{label}: expected most inside-probes to be fill, got {hits}/{} hits",
        pts.len()
    );
}

fn probe_outside(buf: &[u8], pts: &[(u32, u32)], label: &str) {
    let mut hits = 0;
    for &(x, y) in pts {
        if is_bg(pixel(buf, x, y)) {
            hits += 1;
        }
    }
    assert!(
        hits as f32 / pts.len() as f32 > 0.6,
        "{label}: expected most outside-probes to be background, got {hits}/{} hits",
        pts.len()
    );
}

#[test]
fn clip_path_shapes_smoke() {
    // Five stacked rows. Each div is 200×200, filled #ff5050, with a different
    // clip-path. The page bg is #101010 so unclipped pixels are obvious.
    let html = format!(
        r#"<!doctype html><html><head><style>
            html, body {{ margin: 0; padding: 0; background: #101010; }}
            .box {{ width: 200px; height: 200px; background: #ff5050; }}
            .a {{ clip-path: inset(40px round 20px); }}
            .b {{ clip-path: ellipse(80px 40px at 100px 100px); }}
            .c {{ clip-path: path('M 0 0 L 200 0 L 100 200 Z'); }}
            .d {{ clip-path: xywh(40px 40px 120px 120px); }}
            .e {{ clip-path: rect(40px 160px 160px 40px); }}
        </style></head><body>
            <div class="box a"></div>
            <div class="box b"></div>
            <div class="box c"></div>
            <div class="box d"></div>
            <div class="box e"></div>
        </body></html>"#
    );

    let buf = render_html_to_rgba(&html, WIDTH, HEIGHT);
    assert_eq!(buf.len(), (WIDTH * HEIGHT * 4) as usize);

    // Helper: row Y offsets (row 0 starts at y=0, row 1 at y=200, etc.)
    let row = |i: u32| -> u32 { i * ROW_H };

    // --- row 0: inset(40px round 20px) ----------------------------------
    // Inset rect is x:40-160, y:40-160 within the row's local frame.
    // Inside probes: well within the inset rect.
    // Outside probes: corners of the row (outside the inset).
    let r0 = row(0);
    probe_inside(
        &buf,
        &[
            (100, r0 + 100),
            (60, r0 + 60),
            (140, r0 + 140),
            (100, r0 + 50),
            (50, r0 + 100),
        ],
        "inset center",
    );
    probe_outside(
        &buf,
        &[
            (10, r0 + 10),
            (190, r0 + 10),
            (10, r0 + 190),
            (190, r0 + 190),
        ],
        "inset corners",
    );

    // --- row 1: ellipse(80px 40px at 100px 100px) -----------------------
    // Center is (100, 100) local, rx=80, ry=40.
    let r1 = row(1);
    probe_inside(
        &buf,
        &[
            (100, r1 + 100),
            (70, r1 + 100),
            (130, r1 + 100),
            (100, r1 + 80),
            (100, r1 + 120),
        ],
        "ellipse center band",
    );
    probe_outside(
        &buf,
        &[
            // Corners — far outside the ellipse.
            (10, r1 + 10),
            (190, r1 + 10),
            (10, r1 + 190),
            (190, r1 + 190),
            // Above/below the ellipse — y is past ±40 from center.
            (100, r1 + 20),
            (100, r1 + 180),
        ],
        "ellipse outside band",
    );

    // --- row 2: path('M 0 0 L 200 0 L 100 200 Z') (triangle) -------------
    // Apex at top-left and top-right; tip at bottom-center.
    let r2 = row(2);
    probe_inside(
        &buf,
        &[
            (100, r2 + 30),  // near top center, well inside
            (60, r2 + 30),
            (140, r2 + 30),
            (100, r2 + 100),
            (100, r2 + 160),
        ],
        "triangle interior",
    );
    probe_outside(
        &buf,
        &[
            // Bottom-left and bottom-right corners — outside the triangle.
            (10, r2 + 190),
            (190, r2 + 190),
            (20, r2 + 150),
            (180, r2 + 150),
        ],
        "triangle outside",
    );

    // --- row 3: xywh(40px 40px 120px 120px) → inset(40,40,40,40) --------
    let r3 = row(3);
    probe_inside(
        &buf,
        &[
            (100, r3 + 100),
            (60, r3 + 60),
            (140, r3 + 140),
            (50, r3 + 100),
            (150, r3 + 100),
        ],
        "xywh center",
    );
    probe_outside(
        &buf,
        &[
            (10, r3 + 10),
            (190, r3 + 10),
            (10, r3 + 190),
            (190, r3 + 190),
        ],
        "xywh outside corners",
    );

    // --- row 4: rect(40px 160px 160px 40px) → inset(40,40,40,40) --------
    let r4 = row(4);
    probe_inside(
        &buf,
        &[
            (100, r4 + 100),
            (60, r4 + 60),
            (140, r4 + 140),
            (50, r4 + 100),
            (150, r4 + 100),
        ],
        "rect center",
    );
    probe_outside(
        &buf,
        &[
            (10, r4 + 10),
            (190, r4 + 10),
            (10, r4 + 190),
            (190, r4 + 190),
        ],
        "rect outside corners",
    );

    eprintln!("clip_path_shapes_smoke: inset, ellipse, path, xywh, rect all clipped correctly");
}
