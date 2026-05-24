# Wavelet — render-lint system + quality guardrails (planning doc)

Status: planning. Captures the feedback from the 005-whirlpool live run on
2026-05-22. Implement one section at a time; do not begin until the order
is agreed.

## What the 005 run proved

The full Veo-driven pipeline worked end-to-end:

- All 8 commercial pipeline stages produced their required artifacts
- 3 Veo 3.1 clips rendered (44 s wall-clock each, ~$2.50 / 5 s)
- Agent authored Fountain (`script.fountain`) + HTML scenes + emitted
  `velocity.json` / `storyboard.json` / `transitions.json` via the
  wavelet CLI subcommands (DSL discipline held)
- Final `commercial.mp4` rendered: h264, 11.96 s, 720 × 1280, 5.6 MB

The four quality gaps the rendered MP4 exposed, in order of impact:

1. **Color grade drifts across shots.** Shot 1 was a moody dawn; shot 2
   was bright clinical daylight; shot 3 was rustic warm tones. No shared
   visual register — the spot reads as three different commercials cut
   together. This is the canonical single-edit-coherence failure mode
   the eval was designed to catch.

2. **Glyphs clipped by overflow containers.** The "10" headline in one
   scene had the top of the `1` and right edge of the `0` shaved off by
   a parent's `overflow: hidden` plus tight padding. Cosmetic but
   readable as an authoring mistake from across the room.

3. **Text too small to read in the feed.** Most overlay copy was sized
   for desktop reading distance, not for a 9 × 16 thumb-stop. ADA-style
   minimums weren't applied.

4. **Resolution off-spec.** Veo's default for `--aspect 9:16` returned
   720 × 1280; rubric expected 1080 × 1920. The single blocking
   `video_renders` check failure. Pure knob.

Plus two systemic gaps the rendered spot exposed:

- **No music.** Music backend needs ElevenLabs / Fal-via-ElevenLabs;
  the run had FAL_KEY but the wavelet music backend still calls
  ElevenLabs direct. Voiceover is brief-dependent; music is default.

- **No CTA.** Last-shot CTA card was missing. Brand-research stage
  has the data (Whirlpool / KitchenAid is direct-to-consumer; a CTA
  is appropriate). Skill should describe the pattern without
  hard-coding "always have one."

This document organizes the fixes by surface.

## Section 1 — `wavelet lint` (new subcommand)

The substrate already exists. The wavelet binary has:

- `wavelet query <comp> --bbox '#headline'` — scene-graph queries on
  resolved layout
- `wavelet query-shader --shader <name> --frame <png>` — five WGSL
  starter assertions (`contrast_in_region`, `motion_magnitude`,
  `golden_rmse`, `sobel_edge_density`, `color_band_mean`)
- `wavelet verify <comp> [--deep]` — structural lint; `--deep` renders
  mid-frame of each scene and probes audio decode

The gap is a unified lint entry point that runs the right checks at the
right authoring stage. Proposed:

```
wavelet lint <comp.html | scenes-dir | shots-dir>
            [--platform tiktok | instagram_reels | youtube_shorts | facebook_reels | ...]
            [--aspect 9:16 | 16:9 | 1:1 | 4:5]
            [--rules glyph-clip,color-grade-coherence,text-readability,safe-zone]
            [--format json | text]
```

Returns a structured report with severities (`error`, `warn`, `info`)
per finding. Exits non-zero on any `error`. The agent runs this between
the compose stage and the render stage; if errors land, the agent
revises the HTML before paying for the final render.

### Rule 1.1 — glyph-clip

**Symptom**: a rendered glyph is partially occluded by a parent's
`overflow: hidden`, `clip-path`, or sibling z-order.

**Detection**: combine layout + shader.

1. Layout side: walk the resolved layout tree (already accessible via
   `query`). For every text element, compute its baseline + ascent +
   descent + horizontal extent. Walk up the ancestor chain; for each
   ancestor with `overflow: hidden` / `clip-path` / `mask`, intersect
   the text bbox with the ancestor's clip rect. If the intersection
   shrinks the text bbox in any direction, flag with the per-character
   pixel delta.

2. Pixel side (confirmation): render the scene at the time of the
   flagged element. Run a shader assertion that compares the rendered
   glyph's actual ink extent against its expected layout bbox. If the
   actual ink extent stops short of the expected horizontal/vertical
   range, the glyph is clipped.

Output:

```
ERROR  glyph-clip  scenes/01-dawn.html @ t=1.5s
       element: .headline > .num
       expected bbox: x=120 y=480 w=420 h=160
       clipped by: .headline.box-mask (overflow:hidden, padding:8)
       missing: top 4px, right 6px
       fix: increase parent padding or remove overflow:hidden
```

### Rule 1.2 — color-grade-coherence

**Symptom**: adjacent shots come from "different cameras" — different
luminance, different color cast, different contrast.

**Detection**: shader-driven on rendered frames from each shot.

For each `shots/shot-N.mp4`, sample a frame at the midpoint. Compute:

- mean luminance (Y channel)
- mean a*, b* (CIELAB chroma)
- contrast ratio (P95 luma − P5 luma)
- dominant color cast (mode of hue histogram, weighted by saturation)

Build a per-shot vector. Compute pairwise distances. If any pair
exceeds a coherence threshold, flag.

The five existing query-shader primitives cover ~30 % of this; we add:

- `color_grade_signature` — emits the 4-tuple above as JSON
- `color_grade_coherence` — takes a list of frame paths, runs
  `color_grade_signature` on each, and emits pairwise distance matrix +
  pass/fail

Output:

```
ERROR  color-grade-coherence  shots/
       shot-1.mp4 → luma=72  cast=warm-orange  contrast=180
       shot-2.mp4 → luma=210 cast=cool-white   contrast=120
       shot-3.mp4 → luma=140 cast=warm-amber   contrast=170
       deltaE shot-1 vs shot-2 = 38.7 (threshold 12)
       fix: the shot-N Veo prompts must repeat the same cinematography
            vocabulary verbatim — same camera (e.g. "35mm anamorphic"),
            same lens character, same lighting key, same grade language
            ("A24-style", "amber tungsten key", etc.). Today's prompts
            differ on lighting time-of-day; that is the leak.
```

The fix-text is critical: this surfaces a prompt-engineering issue at
the storyboard stage, not just a rendering bug. The lint should point
the agent back to its storyboard.json to fix the prompts before any
re-rolls.

### Rule 1.3 — text-readability (ADA-ish)

**Symptom**: overlay text is too small to read in-feed on mobile.

**Detection**: layout-only. For each text element in each scene:

- Compute the rendered pixel height of the glyph baseline-to-cap
  (the actual painted size, not the CSS `font-size`)
- Compare against a per-aspect minimum:

  | aspect      | min cap-height                          |
  |-------------|------------------------------------------|
  | 9:16 (vert) | 56 px @ 1080 × 1920 (≈ 36 pt)            |
  | 16:9        | 32 px @ 1920 × 1080 (≈ 22 pt)            |
  | 1:1 / 4:5   | 44 px @ 1080-square                      |

  These are derived from WCAG AA + the published Meta/TikTok creative
  guidelines for autoplay-muted feed contexts.

- Plus a contrast-ratio check via `contrast_in_region` (existing
  shader) — text needs ≥ 4.5:1 against whatever's painted beneath it.

Output:

```
WARN   text-readability  scenes/02-whisk.html @ t=3.0s
       element: .ingredient-label
       cap-height: 28 px (min 56 px for 9:16)
       fix: raise font-size to ≥ 110 px at 1080×1920 to clear the
            36 pt floor; check that the parent container can hold it.

ERROR  text-readability  scenes/03-loaf.html @ t=8.0s
       element: .cta-line
       contrast-ratio: 2.1 (min 4.5)
       fix: text fg = #e0d6c4, bg under = #c7b89f → push text to white
            or add a scrim layer
```

### Rule 1.4 — safe-zone collision (per-platform)

**Symptom**: critical text or product hero lands behind a platform's
chrome (TikTok captions bar, Instagram CTA strip, YouTube Shorts
seek bar).

**Substrate**: the pixel-precise table is already authored at
`vendor/colorwave/app/src/lib/video-reframe/safe-zones.ts` covering:

- `tiktok`: top 108, bottom 320, left 60, right 120
- `instagram_reels`: top 200, bottom 250, left 50, right 50
- `instagram_reels_boosted`: top 220, bottom 420, left 50, right 50
- `youtube_shorts`: top 380, bottom 380, left 60, right 120
- `facebook_reels`: top 150, bottom 350, left 60, right 60
- `instagram_feed`, `linkedin`: light margins
- `youtube`, `vimeo`: no chrome

All values normalized to a 1080 × 1920 vertical reference frame.

**Plumbing**: port the TS table to a small JSON file at
`packages/wavelet/data/safe_zones.json` (single source of truth, shared
by wavelet + future colorwave consumers via a small bindings crate if
needed). The TS file becomes a generated artifact in a follow-up
ticket; for now the duplication is fine.

**Detection**: layout-only. Walk every text element + every flagged
`role="hero"` element. Intersect each bbox with the platform's
danger-zone rectangles. Any overlap → finding.

Inputs:

- `--platform <name>` selects the table; if `tiktok` is selected we
  use the TikTok numbers, etc.
- `--aspect` defaults to inferred from canvas dimensions
- For non-vertical platforms (16:9, 1:1), use a generic widescreen /
  in-feed safe-zone table (text within central 80% of canvas)

Output:

```
ERROR  safe-zone  scenes/04-cta.html @ t=11.0s  --platform tiktok
       element: .cta-button
       bbox: x=440 y=1680 w=200 h=80
       overlaps: bottom-chrome (TikTok captions+actions, y > 1600)
       fix: lift the element ≥ 80 px (one canvas-relative-unit-line),
            or reflow the scene to keep CTAs above the bottom 320 px
            of the canvas
```

### Implementation order

```
1. shader: color_grade_signature + color_grade_coherence assertions
2. JSON port of safe_zones table + a tiny loader
3. wavelet lint <comp>      — single entry point, dispatches to rules
4. rule: safe-zone           — pure layout; quickest win
5. rule: glyph-clip           — layout + optional pixel confirmation
6. rule: text-readability    — layout + contrast_in_region shader
7. rule: color-grade-coherence — shader on shot midpoints
```

Each rule lives in `packages/wavelet/src/lint/<rule>.rs`. The lint
binary subcommand is the orchestrator.

## Section 2 — quality guardrails NOT in lint (system / skill / prompt-level)

These aren't caught by linting; they're authored discipline that lives
in the skill or in the binary's default behavior.

### 2.1 — color-grade consistency upstream of lint

The lint catches the symptom; the fix is at the storyboard / director
stage. The director skill (`packages/workbooks/skills/wavelet-director/`)
should specify a **shot-prompt-prefix** convention:

```
<scene-prompt> + " " + <fixed cinematography preamble>

Where <fixed cinematography preamble> for THIS commercial is repeated
verbatim across every shot:

  "shot on 35 mm anamorphic, shallow DoF, amber tungsten key from
   right, A24 color grade, soft film grain, no LUT shift"
```

The agent locks the preamble once in `strategy.md` and pastes it into
every `shot.txt2vid` prompt. The lint then confirms the rendered shots
honored it.

This is a skill-content edit, not a code change. Land it in the
director skill before re-running 005.

### 2.2 — music as default

Currently `wavelet music gen` requires `ELEVENLABS_API_KEY`. The 005
run had `FAL_KEY` but no ElevenLabs key, so music was skipped.

Two paths:

A. **Add fal-elevenlabs music backend.** Fal proxies ElevenLabs music
   endpoints (e.g. `fal-ai/elevenlabs/music`). Add an adapter that
   uses `FAL_KEY` and hits the Fal proxy URL. Becomes the new default
   for `wavelet music gen` when no `ELEVENLABS_API_KEY` is present but
   `FAL_KEY` is.

B. **Add Lyria via Google.** Google's Lyria is on `GOOGLE_API_KEY`.
   Wavelet already has a `backends/google/lyria.rs` from a prior
   wiring effort — check if it's connected to `wavelet music gen`.

Path A is the smaller change. Path B is simpler operationally for the
current key inventory. Recommend doing both — Lyria becomes default,
Fal-ElevenLabs is a `--backend` flag option.

### 2.3 — CTA + real logos (brand-research-conditioned)

Last-shot CTA card is a common pattern but **not always appropriate**.
Shane's rule: lifestyle / brand-vibe spots → no hard CTA; direct-to-
consumer / conversion spots → clear CTA.

The decision needs to live in the **brand-research stage**, not in the
director recipe. Proposed addition to that stage:

The `brandwork brand.brief <domain>` response already includes brand
descriptors (positioning, slogan, palette, social handles). Add one
derived field the brief stage emits:

```json
{
  "...standard brandwork fields...",
  "wavelet_recommended_cta_mode": "lifestyle" | "direct_response"
}
```

Resolution rule: if the brand's published ads (`brandwork ads`) lean
heavily on "buy now / shop the link / use code XYZ" copy → direct
response. If they lean on aspirational / mood / "available where good
X is sold" → lifestyle. Director uses this hint when scripting the
last scene.

For direct-response mode, the director skill describes the canonical
CTA-card pattern:

- Last scene 1.5–2 s of total runtime
- Brand wordmark animates in (use the brand's actual logo URL from
  `brandwork brand.brief`)
- One-line CTA copy (e.g. "shop the iconic stand mixer")
- One button (real CSS / SVG, not an image of a button)
- Optional QR code

For lifestyle mode, the last shot stays atmospheric; the wordmark
may appear as a small bug in the corner, no button.

This is a skill-content edit + one small enrichment of the
brief-stage output. No new wavelet subcommand.

## Section 3 — what to do next, in order

The eval will not change between iterations; the system around it
will. Order:

1. **Build `wavelet lint` v1 with safe-zone + glyph-clip rules.**
   These are pure layout walks against the resolved DOM; no new
   shaders required. Fastest path to surfacing the worst pain.

2. **Land color-grade-coherence shader + lint rule.**
   The hardest of the four rules; benefits most from being separate
   work.

3. **Land text-readability lint rule.**
   Layout + the existing `contrast_in_region` shader.

4. **Update the director skill** with the shot-prompt-prefix
   convention (section 2.1) + the CTA-mode brand-research enrichment
   (section 2.3).

5. **Add music defaulting.** Lyria via Google first
   (`backends/google/lyria.rs` likely needs final wiring); Fal-
   ElevenLabs second.

6. **Re-run 005 / 006 / 007.** Expect the rubric scores to climb on
   the dimensions these address — single_edit_coherence, on_screen_
   text, format_compliance, and the new programmatic_artifacts dim.

7. **Fix the cost-tracking gap.** The shim doesn't capture
   `cost_estimate_usd` from wavelet's stdout JSON, so `cost_below`
   passes vacuously at $0. Add JSON-stdout parsing to the shim and
   record cost per call. Independent of the rest of this work; do
   whenever convenient.

## Section 4 — files this work touches

```
packages/wavelet/src/lint/                        — NEW dir
  mod.rs                                            — lint subcommand orchestrator
  safe_zone.rs                                      — rule 1.4
  glyph_clip.rs                                     — rule 1.1
  text_readability.rs                               — rule 1.3
  color_grade.rs                                    — rule 1.2 (calls into shader)
packages/wavelet/src/bin/wavelet.rs               — wire `lint` subcommand
packages/wavelet/src/cli_args/cmd.rs              — add `Lint(LintOp)` variant
packages/wavelet/src/cli_args/lint_op.rs          — NEW
packages/wavelet/src/agent/plan/validators/shader.rs  — add color_grade_signature + color_grade_coherence shaders
packages/wavelet/data/safe_zones.json             — NEW; ported from colorwave TS
packages/wavelet/src/backends/google/lyria.rs     — finish wiring to `wavelet music gen` default
packages/wavelet/evals/bin/wavelet-traced         — parse stdout JSON, capture cost_estimate_usd into trace

packages/workbooks/skills/wavelet-director/SKILL.md — shot-prompt-prefix convention; CTA-mode pattern
vendor/colorwave/app/src/lib/video-reframe/safe-zones.ts → becomes a generated artifact (follow-up)
```

## Section 5 — what's out of scope here

- Workbench-side eval orchestrator changes (covered in earlier
  commits — env-source, shim, cost-tracking is the only one left)
- Changes to the prompts themselves (the 005 / 006 / 007 prompts stay
  as one paragraph; skill carries the discipline)
- Refactoring the deleted-Fal grab-bag any further (it's gone)
- Adalign feature work (the brief enrichment in 2.3 might live on the
  brandwork side or in a wavelet post-processor; that's a design call
  for whoever implements step 4)

End of doc. Confirm order + scope, then implement section by section.
