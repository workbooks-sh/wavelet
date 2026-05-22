---
name: gamut-director
description: Use when the user wants to produce a short generative video — a 10-30 second commercial, brand spot, montage, or trailer — entirely from a written brief, using only the `gamut` CLI + Fal / ElevenLabs AI backends + the web for reference. Triggers on "make a commercial", "generate a video ad", "produce a spot", "direct a video from a brief", "end-to-end generative video".
---

# gamut-director — end-to-end generative video

You are the director. Take a brief (or invent one), produce a finished MP4.
Every visible frame and every audible sample is AI-generated. No stock
footage, no hand-edited timeline, no manual asset wrangling.

## What gamut actually is

Gamut is a motion-graphics renderer. **Blitz** handles HTML layout,
**Stylo** (Servo's parallel CSS engine) computes styles and drives the
animation clock, **Parley** shapes text, **Animato** powers timeline
math, **Vello** rasterizes through wgpu, **rsmpeg** encodes h264/h265.
The render loop walks frame-by-frame, ticks the CSS animation engine via
`BaseDocument::resolve(now)` so every `@keyframes` rule and every
`transition` advances to the current scene time, paints to RGBA,
composites over a per-scene background video (and audio cues mixed
through rsmpeg), and writes an MP4 + sidecar WAV.

The author surface is HTML. One file per scene (`scenes/01-title.html`,
`scenes/02-product.html`, …). One top-level `index.html` lists the
scenes and binds the audio. That is the entire authoring layer. No JSON
sidecars, no DSLs, no JS, no proprietary timeline format.

**The hard rule:** this isn't a browser, but you write as if it is. If
something is standard CSS, assume it works — the exceptions are listed
explicitly below. Stylo and Blitz cover the bulk of the modern web
platform. Reach for the same idioms you'd use building a hand-crafted
landing page: `@keyframes`, `transition`, `clip-path`, `mix-blend-mode`,
flexbox, grid, gradients, `transform`, `cubic-bezier()`, web fonts via
`@font-face`. They render.

**The anti-pattern this doc is written to prevent.** Every freshly-spun
agent writes the same thing on its first try: four scenes, all
`position: absolute; left: 80px; bottom: 80px; font: 900 88px Inter;`,
no `clip-path`, no `mix-blend-mode`, no animation beyond a single
`@keyframes fade-in`. That's the **AI-default lockup**. It looks like
every other AI commercial, and every senior creative director can spot
it in one frame. If your spot looks like every other AI ad, you skipped
the palette — go back and use it.

## Tools you need

- **`gamut` CLI** — the entire pipeline. Single binary at
  `packages/gamut/target/debug/gamut` from the repo root, or just `gamut`
  if it's on PATH.
- **The web** — research the subject of the commercial (palette, mood,
  reference shots) to inform your prompts. Use WebSearch/WebFetch.
- **Bash** — for parallel shot generation and one final ffmpeg mux step.
- **`FAL_KEY` + `ELEVENLABS_API_KEY`** — pre-exported in env. Don't
  print them. The CLI reads them.

## The two production paths

Two workflows, picked by whether the commercial is for a **real product**
(brand identity, recognizable silhouette — cars, watches, sneakers,
named landmarks) or a **brand-vibe** concept (fragrance, abstract
luxury, travel, energy drink — anything where AI hallucination is
acceptable).

**Path B — reference-conditioned scene-still gen (the default for any
real product).** Collect 1-3 reference photos of the exact product.
For each scene, generate a scene-aware still that places the product
INTO that scene's lighting, angle, and perspective via
`gamut image scene-still` (Fal Seedream, ref-conditioned txt2img).
Then `img2vid` each scene-still. The product is generated fresh into
each shot's world — no cutout edges, no flat composites, no floating
subjects on black voids.

```
web research → 1-3 high-quality reference photos (URLs)
            → for each scene:
                gamut image scene-still --refs <urls> --prompt "<scene>"
                gamut shot img2vid still-N.png "<motion>"
```

**Path A — txt2vid only (brand-vibe only).**
Wan-T2V generates each shot from scratch. Cheap (~$0.10/shot), fast
(~30s/shot). Use for fragrance, travel, abstract concepts. Do NOT use
for real branded products — Wan hallucinates logos, badges, and text.

**Legacy: bg-remove + composite (deprecated, kept for one narrow case).**
The old pipeline (`gamut image isolate` → `gamut shot still` →
`gamut image composite` → `img2vid`) still works, but it's only
recommended when you have a photographically-perfect reference and
just need to place it into a clean studio backdrop. For everything
else, scene-still gen avoids the cutout-seam failure mode.

Cost per shot (Path B): $0.04 scene-still + $0.10 img2vid = $0.14.

**The one hard rule for Path B.** Pick 1-3 high-quality reference photos
of the *same* product. All shots in the spot derive from those refs via
scene-still gen. Different scenes come from different *scene prompts*,
not from different reference photos of different cars. Refs lock the
product identity; scene prompts vary the world around it.

## The pipeline (run in order)

```
brief.md (9-line) → gamut brief check
            │
            ├─ write screenplay
            ├─ gamut screenplay parse → screenplay.json (sanity check)
            ├─ gamut velocity propose → velocity.json
            ├─ gamut storyboard plan  → storyboard.json
            ├─ gamut storyboard verify → must be 0 errors
            ├─ gamut continuity check  → must be 0 errors
            ├─ gamut transitions classify → transitions.json
            │
            ├─ gamut music gen → music.wav   (paid: ~$0.06)
            ├─ gamut velocity validate --against music.wav  (sanity)
            │
            ├─ for each scene:
            │   gamut shot txt2vid → shots/shot-N.mp4 (paid: ~$0.10 each)
            │
            ├─ write scene HTML overlays (the freeform palette section)
            ├─ assemble index.html manifest (or comp.json for advanced cases)
            └─ gamut render index.html -o commercial.mp4
```

Canonical pipeline spec lives at `gamut pipelines show commercial`. The
workflow runner walks it cooperatively — `gamut workflow run commercial
--workdir .` reports the next stage based on which artifacts are on disk.

## Gating spend with the reviewer

After each stage produces an artifact — storyboard JSON, hero-panels
set, per-shot scene-still, per-shot i2v MP4, the final muxed cut —
invoke the `gamut-reviewer` skill with the stage name + artifact path
+ the brief. It returns a structured pass/warn/fail verdict and a
`spend_decision` of `proceed`, `iterate`, or `abort`. Honor it: only
move to the next paid step on `proceed`. On `iterate`, apply the
named remediation (re-roll, `fix-from-verify`, tighten the prompt)
and re-review. On `abort` (the same fail has recurred ≥ 3 times),
stop and report back rather than burning more budget. The reviewer
only reads — it never spends — so calling it between every stage is
free insurance against compounding errors.

## Step 1 — pick a concept and write the 9-line brief

A 10-15 second commercial works best. Good fits:

- A consumer brand without specific logo demands (coffee, fragrance,
  EV concept, watch, travel)
- A travel destination
- A non-profit cause / public service spot

Avoid:

- Real people with dialogue (no lip-sync yet)
- Specific brand logos (Wan will hallucinate them poorly)
- Products that need close-ups of small written details
- Anything requiring text legibility in the generated footage

### The 9-line ad creative brief

Don't write prose. Write `brief.md` in the **9-line slot-filled
format**. One slot per line, in any order:

| Slot       | What it captures                                       |
|------------|--------------------------------------------------------|
| `PRODUCT`  | What we're selling — one noun phrase                   |
| `AUDIENCE` | Who the spot is for — specific demographic, not "everyone" |
| `INSIGHT`  | What they currently believe/feel that the brand wants to shift |
| `PROMISE`  | What the brand says it will deliver                    |
| `PROOF`    | One concrete reason to believe the promise             |
| `TONE`     | Single-word aesthetic register (e.g. `cinematic`, `irreverent`, `brutalist`) |
| `MUSIC`    | Genre + energy curve (e.g. `ambient build → driving electronic peak`) |
| `CALL`     | What the viewer should do — CTA in 1-5 words           |
| `RUNTIME`  | Target duration in seconds (integer)                   |

Worked example (`brief.md`):

```markdown
PRODUCT: Allbirds Tree Runner sneakers
AUDIENCE: 28-40 urban professionals who walk more than they run
INSIGHT: "Sustainable" usually means uncomfortable or ugly
PROMISE: All-day comfort that happens to be made from trees
PROOF: Eucalyptus-fiber upper + sugarcane sole, machine washable
TONE: understated
MUSIC: acoustic minimal → warm indie-folk swell
CALL: Try them barefoot
RUNTIME: 15
```

Validate before continuing:

```bash
gamut brief check brief.md
```

Long-form briefs are still acceptable as input. When a human hands you a
prose brief, distill it into the 9-line shape *before* moving to step 2.

## Step 2 — write the screenplay

Fountain format (`.fountain`). 4-6 scenes, mostly action paragraphs.
Match the screenplay's pacing to the commercial: short punchy action =
fast cuts; long flowing description = slower scenes.

```fountain
Title: <product>
Author: gamut-director

EXT. SAGUARO FIELD - DAY

A giant cactus stands sentinel against the morning sky.

CUT TO:

EXT. SLOT CANYON - DAY

Light cuts through the narrow walls of red stone.

CUT TO:

EXT. SEDONA VISTA - SUNSET

Cliffs glow as the sun drops behind the ridge.

CUT TO:

EXT. DESERT ROAD - NIGHT

Headlights cut a path through the silence.

FADE OUT.
```

Save to `script.fountain`.

## Step 3 — run the agent-side pipeline

All free, deterministic, and reversible:

```bash
gamut screenplay parse script.fountain --pretty -o screenplay.json
gamut velocity propose script.fountain --pretty -o velocity.json
gamut storyboard plan script.fountain --velocity velocity.json --pretty -o storyboard.json
gamut storyboard verify storyboard.json
gamut continuity check storyboard.json
gamut transitions classify script.fountain --velocity velocity.json --pretty -o transitions.json
```

Read each output. `velocity.json`'s `mean_bpm` tells you the music's
target tempo. `storyboard.json`'s `shots[].subject` tells you what each
shot is about. The continuity report flags 180° / motion / scale-jump
issues — if there are errors, reorder the screenplay or add a transition.

## Step 3.25 — fill structured shot attributes (L-Storyboard)

`Shot` carries an optional `attributes` block — seven typed slots that
replace freeform prose in the eventual model prompt:

| Slot     | What it captures                                   |
|----------|----------------------------------------------------|
| subject  | what the shot is OF                                |
| action   | what's happening                                   |
| scene    | where it is (location + time of day + environment) |
| camera   | shot type + focal length + angle                   |
| lens     | optical character — DoF, anamorphic, fringe        |
| lighting | direction + quality of light                       |
| style    | aesthetic register, film stock, color grade        |

```json
"attributes": {
  "subject": "a 1968 Porsche 911 GT3 in racing yellow",
  "action": "idles, engine off, parked at pit lane",
  "scene": "on wet asphalt as the sun crests the ridge",
  "camera": "WS 50mm, low angle, 3/4 front",
  "lens": "anamorphic, shallow DoF, slight chromatic fringe",
  "lighting": "backlit by rising sun, mist-diffused",
  "style": "cinematic, A24-flavored, restrained color"
}
```

All seven required. If you don't know one, write the literal
`"unspecified"`. Reference fixture: `packages/gamut/tests/fixtures/l-storyboard-example.json`.

## Step 3.26 — let an LLM fill the slots

```bash
gamut director synthesize brief.md storyboard.json -o storyboard.dir.json --pretty
```

Default Gemini 2.5 Pro via fal-ai/any-llm (~$0.02–$0.05/spot). Pass
`--model claude` for Opus 4.7 fallback. Read the output and patch the
two or three slots that drift.

## Step 3.5 — generate the voiceover (optional)

```bash
gamut dialogue tts "<your VO copy>" \
  --backend fal-kokoro \
  --voice af_nicole \
  --max-cost 0.05 \
  --out vo.wav \
  --pretty
```

VO copy: fits the total duration (~2.5 words/sec is a comfortable read
pace), lands the brand or model name clearly with a 1-second pause for
emphasis, ends on a tagline or CTA.

### Word-level captions (CapCut / Hormozi / minimal)

```bash
gamut dialogue captions \
  --audio vo.wav --text "Fast cheap reliable big wins" \
  --backend fal-whisper-words --style hormozi \
  -o captions.json --pretty

gamut captions overlay --in captions.json --style hormozi \
  --width 1080 --height 1920 -o caption.html
```

The emitted `caption.html` is a normal scene HTML file — drop it into
your scene list as a sibling. CSS is `@keyframes`-only (no JS) so it
renders correctly through Blitz.

## Step 4 — generate the music

```bash
gamut music gen \
  --velocity velocity.json \
  --style "<2-3 line style description>" \
  --duration <total_secs> \
  --max-cost 0.10 \
  --out music.mp3 \
  --pretty
```

Default `--backend elevenlabs` is the only commercially-safe option
(Merlin + Kobalt-licensed). `--backend fal-musicgen` is DEPRECATED —
pending litigation, not commercial-safe.

Validate:

```bash
gamut velocity validate velocity.json --against music.wav --tolerance 20 --pretty
```

The validator now also writes a sibling `music.cuts.edl` when onsets are
detected. Use those onsets as snap targets for shot boundaries.

## Step 5 — generate the shots

### Path A: txt2vid (brand-vibe)

One `gamut shot txt2vid` call per scene. Run them in parallel.

```bash
gamut shot txt2vid \
  "<rich, specific prompt>" \
  --duration 5 --max-cost 0.20 \
  --out shots/shot-N-<name>.mp4 --pretty
```

Prompt construction (in this order, comma-separated): subject, action,
setting, composition, atmosphere, tech. The standard negative prompt
(`"no text overlay, no watermark, no distortion, no extra limbs, low
quality, blurry"`) is appended automatically.

### Path B: reference-conditioned scene-still + img2vid

```bash
# B.1: hero panels lock the spot's look
gamut storyboard hero-panels storyboard.json -o shots/hero/ \
  --refs "https://.../ref-1.jpg" --layout 2x2 \
  --max-cost 0.10 --pretty

# B.2: per-shot scene-still conditioned on the hero panel
gamut image scene-still \
  --refs "https://.../ref-1.jpg" \
  --hero-panel-ref "https://.../panel-2.png" \
  --prompt "<rich scene prompt>" \
  --image-size landscape_16_9 \
  --max-cost 0.05 --out shots/still-N.png --pretty

# B.3: img2vid the scene-still
gamut shot img2vid shots/still-N.png "<short motion prompt>" \
  --duration 5 --max-cost 0.15 \
  --out shots/shot-N.mp4 --pretty
```

Hero shots (1-2 per spot) run on `--backend nano-banana-pro` (~$0.24/img,
14 refs, 4K). Bulk shots run on default `seedream`.

### Variant generation — roll N, pick the winner

Every still / clip gen verb accepts `--variants N` (1-8, default 1).

```bash
gamut image scene-still --refs https://… --prompt "…hero shot…" \
  --variants 3 --select max-vlm \
  --max-cost 0.05 --max-variants-cost 0.15 --pretty
```

`--select` policies: `max-vlm` (default), `pairwise-tournament` (VISTA
bracket for identity-critical shots), `first`, `user`, `cheapest`.

Use `pairwise-tournament` for hero shots and identity-critical SKUs;
`max-vlm` for everything else. Skip variants entirely for filler shots.

## Step 6 — write the scene HTML overlays (FREEFORM)

**This is the section that decides whether your spot looks AI-default or
art-directed.** Read it carefully.

One `scenes/<id>.html` per scene composites *over* the generated video.
Blitz's CSS engine covers the bulk of the modern web platform: standard
CSS animations (`@keyframes`, `transition`, `cubic-bezier()`, `steps()`),
the full transform stack, clip-path, mix-blend-mode, all 16 blend modes,
gradients, web fonts, flexbox, grid, the works.

Your training data is dense in standard HTML/CSS. Use it. Anything you'd
write on a hand-crafted brand site renders here.

### Two structural rules

1. **`html` and `body` must have `background: transparent`** so the
   generated video shows through where your HTML doesn't paint.
2. **The per-scene background video comes via the scene's `video_bg`
   field** (in `comp.json`) or via the top-level `<section
   data-scene-href="…">` reference (in `index.html`). Inline `<video>`
   inside a scene HTML file is not rendered today (see "What does not
   work" below). Top-level `<audio>` elements in `index.html` are bound
   to the comp's audio cue list — that's the canonical place for music
   + VO.

### The supported palette — every one of these works today

Pull from this whenever you're tempted to default to the bottom-left
lockup. One-line examples each; treat them as the toolkit.

**Layout + position**

```css
.frame { position: absolute; inset: 0; display: grid; grid-template-rows: 1fr auto; }
.lower-third { display: flex; gap: 1.2rem; align-items: flex-end; padding: 4rem; }
```

`position: absolute / fixed / relative`, flexbox, grid, `inset`, `gap`,
`aspect-ratio`, `z-index`. `position: sticky` does not work (no scroll
in video).

**Typography**

```css
@import url("https://fonts.googleapis.com/css2?family=Bodoni+Moda:wght@400;900&display=swap");
@font-face { font-family: "Custom"; src: url("./fonts/custom.woff2") format("woff2"); }
.title { font: 900 240px/0.88 "Bodoni Moda", serif; letter-spacing: -0.04em; }
.subtitle { font: 200 18px "Inter", sans-serif; letter-spacing: 0.4em; text-transform: uppercase; }
```

System fonts work. WOFF / WOFF2 via `@font-face`. Google Fonts via
`@import`. Variable font axes work where Stylo understands the
named-instance form. `line-height`, `letter-spacing`, `word-spacing`,
`text-transform`, `text-align`, `font-feature-settings`, `font-variant`
all work.

**Color + gradients**

```css
.scrim { background: linear-gradient(180deg, transparent 0%, #000a 60%, #000 100%); }
.aurora { background: conic-gradient(from 90deg at 30% 70%, #f06, #f60, #06f, #f06); }
.spot   { background: radial-gradient(ellipse at 30% 30%, #fffa 0%, transparent 60%); }
```

Linear, radial, and conic gradients. Hex / rgb / rgba / hsl /
`color-mix()`. `currentColor` and CSS variables.

**Transforms**

```css
.tilt { transform: rotate(-3deg) scale(1.08) translateX(40px); transform-origin: top left; }
.skew { transform: skewX(-8deg); }
```

`translate`, `scale`, `rotate`, `skew`, `matrix()`, `transform-origin`,
3D transforms (`rotateX/Y/Z`, `perspective`).

**Borders, shadows, radius**

```css
.card { border: 2px solid #fff; border-radius: 24px; box-shadow: 0 24px 80px #000c, inset 0 0 0 1px #fff2; }
```

Multi-stop borders, `border-radius` (each corner independently),
`box-shadow` (multiple, including `inset`).

**clip-path** — *use this*

```css
.window { clip-path: circle(40% at 50% 50%); }
.notch  { clip-path: polygon(0 0, 100% 0, 100% 100%, 50% 70%, 0 100%); }
.tag    { clip-path: polygon(0 0, calc(100% - 24px) 0, 100% 50%, calc(100% - 24px) 100%, 0 100%); }
```

`circle()`, `polygon()`, and the box-keyword forms (`margin-box`,
`border-box`, `padding-box`, `content-box`) render today.
`inset()`, `ellipse()`, `path()`, and `url(#mask)` are silently dropped
— pick a polygon or circle approximation if you need a rect-with-radius
window.

**mix-blend-mode** — *use this*

```css
.title-diff   { color: #fff; mix-blend-mode: difference; }
.title-screen { mix-blend-mode: screen; }
.title-mult   { mix-blend-mode: multiply; }
```

All 16 CSS blend modes render through Vello's `peniko::Mix` —
`multiply, screen, overlay, darken, lighten, color-dodge, color-burn,
hard-light, soft-light, difference, exclusion, hue, saturation, color,
luminosity, normal`. `difference` over a generated shot is the canonical
"type carves through video" idiom and you should use it at least once
per spot.

**filter** — *use this*

```css
.soft     { filter: blur(8px); }
.poster   { filter: contrast(1.6) saturate(1.2); }
.mono     { filter: grayscale(1); }
.shifted  { filter: hue-rotate(180deg); }
.lifted   { filter: drop-shadow(0 12px 24px rgba(0,0,0,0.6)); }
.chained  { filter: blur(2px) saturate(1.3) brightness(1.1); }
```

`blur(Npx)`, `saturate(N)`, `brightness(N)`, `contrast(N)`,
`grayscale(N)`, `hue-rotate(Ndeg)`, `invert(N)`, `sepia(N)`, `opacity(N)`,
and `drop-shadow(X Y B color)` all render. Chains apply left-to-right
per CSS spec. Implemented as a render-to-image fallback in
`vendor/blitz-paint/src/render/filter.rs` — the element is painted into
a sidecar Vello scene, the resulting RGBA buffer is filtered via
`image::imageops::blur` for spatial blur + standard `feColorMatrix`
matrices for color filters, then composited back into the parent scene
as an image brush. The fallback expands the painted region by 3·sigma
for blur and by the offset + 3·sigma for drop-shadow, so soft halos
don't get clipped.

`backdrop-filter` does *not* render today (Stylo doesn't expose the
property under the Servo feature flag blitz-dom uses). Use a translucent
solid scrim or gradient scrim as a workaround.

**Standard CSS animations** — *use this*

```css
@keyframes title-in {
  0%   { transform: translateY(40px); opacity: 0; }
  60%  { transform: translateY(-4px); opacity: 1; }
  100% { transform: translateY(0);    opacity: 1; }
}
.title { animation: title-in 0.9s var(--ease-out-back) both; }

.subtitle { transition: opacity 0.4s ease-out 0.3s; opacity: 0; }
.scene.live .subtitle { opacity: 1; }
```

Stylo drives `@keyframes` and `transition` advance per-frame from the
scene clock — `render_offline.rs` resolves the document at every frame's
local time, so animations actually play in offline render. `linear`,
`ease`, `ease-in`, `ease-out`, `ease-in-out`, `cubic-bezier(…)`,
`steps(N [, jump-…])` all render. `animation-fill-mode`,
`animation-delay`, `animation-iteration-count`, `animation-direction`
work. Stagger by `:nth-child` + `animation-delay` math.

Springs / bounces / elastic / wiggle aren't representable as
cubic-bezier. For those, lay out the animation as a multi-stop
`@keyframes` timeline by hand — Stylo interpolates per-stop.

**Extended easing — paste from `eases.css`**

A sibling file ships the standard easings.net curves as CSS custom
properties — `var(--ease-out-back)`, `var(--ease-out-expo)`,
`var(--ease-out-quint)`, etc., 24 named eases total. Copy the `:root`
block from `vendor/workbooks/skills/gamut-director/eases.css` into each
scene's `<style>` (or `@import` it). Two scenes in every spot should
reach into the extended table — `ease-out` is fine but identical use of
plain `ease` across every cut is the AI-default tell.

```css
:root {
  --ease-out-back:  cubic-bezier(0.34, 1.56, 0.64, 1);
  --ease-out-quint: cubic-bezier(0.22, 1,    0.36, 1);
  --ease-out-expo:  cubic-bezier(0.16, 1,    0.3,  1);
  /* …see eases.css for the full set */
}

.title  { animation: enter 0.9s var(--ease-out-back) both; }
.kicker { animation: enter 1.4s var(--ease-out-quint) 0.2s both; }
```

**`<img>` element**

```html
<img src="./logo.svg" style="width: 120px; opacity: 0.9;">
<img src="./moodboard-1.png" style="position: absolute; inset: 0; object-fit: cover;">
```

Raster images (PNG / JPG / WebP) and SVG paint natively. Decoded via the
image crate's codec features in Blitz. `object-fit`, `object-position`
work.

### Variation across cuts — a hard rule

Within a single spot, **no two adjacent scenes may share the same
typographic treatment**. Same typeface across the spot is fine — that's
the through-line. Same size *and* position *and* motion is the
"AI-default" tell that flattens the work.

The good failure mode: "scene 1 uses Bodoni 240px center, scene 2 uses
Inter 22px upper-right, scene 3 is type-free, scene 4 uses JetBrains
Mono 18px corner tags." The bad failure mode: "scene 1 is Inter 88px
bottom-left, scene 2 is Inter 88px bottom-left, scene 3 is Inter 88px
bottom-left, scene 4 is Inter 88px bottom-left."

### Self-check before declaring a scene done

Ask yourself:

1. **Did I reuse the previous scene's lockup?** Same typeface + same
   size + same position + same motion. If yes — redo this one.
2. **Did I reach for `clip-path` or `mix-blend-mode` anywhere in the
   spot?** Neither one is mandatory in every scene, but a four-scene
   spot that doesn't touch either is using maybe 30% of the palette.
3. **Did I use anything from the extended ease table?** At least two
   scenes should be on `var(--ease-*)` curves, not plain `ease`.
4. **Did I let any scene have zero type?** Editorial silence is a real
   move. Captioning every cut isn't a requirement.
5. **Are my animations distinct per scene?** Four scenes all running the
   same `@keyframes slide-in` is the AI-default in motion form.

### Anti-pattern gallery — do NOT ship these

```css
/* ANTI-PATTERN 1: the AI-default lockup. Four scenes of this is failure. */
.title {
  position: absolute; left: 80px; bottom: 80px;
  font: 900 88px Inter, sans-serif;
  color: white;
  animation: fade-in 0.6s ease both;
}
```

```css
/* ANTI-PATTERN 2: same @keyframes recycled across every scene */
@keyframes enter { from { opacity: 0; transform: translateY(20px); } to { opacity: 1; transform: none; } }
.scene-1-title { animation: enter 0.6s ease both; }
.scene-2-title { animation: enter 0.6s ease both; }
.scene-3-title { animation: enter 0.6s ease both; }
.scene-4-title { animation: enter 0.6s ease both; }
```

```css
/* ANTI-PATTERN 3: everything centered, no negative-space awareness */
.scene { display: grid; place-items: center; }
.title { text-align: center; font-size: 120px; }
/* Repeated unmodified across all four scenes. */
```

```css
/* ANTI-PATTERN 4: plain `ease` everywhere, no extended curves used */
.a { animation: in 0.5s ease both; }
.b { animation: in 0.5s ease-in-out both; }
.c { animation: in 0.5s ease both; }
.d { animation: in 0.5s ease both; }
```

### Edge insets, contrast, reading rate — verify with the tools

- **Title-safe** is the center 80% (10% inset off each edge). Type
  across title-safe is a deliberate choice (brutalist, display-driven).
  Type across action-safe (center 90%, 5% inset) is a mistake on most
  platforms. `gamut::aspect::safe_areas(w, h)` returns the right
  rectangles per aspect.
- **Read the shot first.** `gamut image negative-space <png>` returns
  the eye-friendly grid cells with suggested text color + scrim opacity.
  Position type in the top-ranked zone unless you have a reason to
  fight the shot.
- **Contrast.** `gamut image contrast <png> --region X,Y,W,H
  --text-color #...` reports WCAG ratio + suggests a scrim if below
  threshold. WCAG AA is 4.5:1 for normal text, 3:1 for 18pt+ or 14pt
  bold. Below AA is a deliberate choice (luxury whisper, brutalist
  clash), not an accident.
- **Reading rate** is ~2.5 chars/sec. Kinetic word-by-word reveals live
  in the 0.25–0.4s range; faster than 0.15s reads as a flicker.
- **Existing text in the shot.** `gamut image ocr <png>` (stubbed
  pending Fal got-ocr) detects baked-in text so you don't stack overlays
  on plate numbers or storefront signage.

## Step 6.5 — ten worked scene examples (study these)

Each example is a complete `<style>` + `<body>` block. They span the
palette — copy patterns from these directly into your scenes. They
deliberately don't share a lockup.

### Example 1 — Brutalist: Helvetica Black, all-caps, covers the frame

```html
<!doctype html>
<html><head><style>
  html, body { margin: 0; padding: 0; background: transparent; width: 100%; height: 100%; overflow: hidden; }
  body { font-family: "Helvetica Neue", Helvetica, Arial, sans-serif; color: #fff; }
  .smash {
    position: absolute; inset: 0;
    display: grid; grid-template-rows: 1fr auto;
    padding: 24px 32px;
  }
  .word {
    font-weight: 900;
    font-size: 26vw;
    line-height: 0.78;
    letter-spacing: -0.05em;
    text-transform: uppercase;
    margin: 0;
  }
  .word.two { text-align: right; transform: translateY(-0.05em); }
  .meta {
    font-size: 18px; letter-spacing: 0.3em; text-transform: uppercase;
    display: flex; justify-content: space-between;
  }
  @keyframes punch-in {
    0%   { transform: scale(1.04); opacity: 0; }
    60%  { transform: scale(1);    opacity: 1; }
    100% { transform: scale(1);    opacity: 1; }
  }
  .word { animation: punch-in 0.5s cubic-bezier(0.16, 1, 0.3, 1) both; }
  .word.two { animation-delay: 0.18s; }
</style></head>
<body>
  <div class="smash">
    <div>
      <h1 class="word">NOW</h1>
      <h1 class="word two">EVERYTHING</h1>
    </div>
    <div class="meta"><span>04 / SS26</span><span>FIELD NOTES</span></div>
  </div>
</body></html>
```

Design intent: type IS the image. Reads at any size, leans anti-pretty,
defies the safe-area convention deliberately.

### Example 2 — Editorial: Didone serif, asymmetric two-line moment

```html
<!doctype html>
<html><head>
<style>
  @import url("https://fonts.googleapis.com/css2?family=Bodoni+Moda:ital,wght@0,400;0,700;1,400&display=swap");
  html, body { margin: 0; padding: 0; background: transparent; height: 100%; }
  .frame { position: absolute; inset: 0; padding: 8vh 9vw; display: grid; grid-template-rows: 1fr auto 1fr; }
  .moment {
    font-family: "Bodoni Moda", "Didot", serif;
    color: #f5efe6;
    font-size: 7vw;
    line-height: 1.02;
    font-weight: 400;
  }
  .moment em { font-style: italic; font-weight: 400; }
  .line-1 { grid-row: 2; max-width: 60%; }
  .kicker {
    align-self: end; justify-self: end;
    grid-row: 3;
    font: 400 14px/1.4 "Bodoni Moda", serif;
    letter-spacing: 0.34em; text-transform: uppercase;
    color: #f5efe6c0;
    max-width: 22ch; text-align: right;
  }
  @keyframes drift-up {
    from { transform: translateY(20px); opacity: 0; }
    to   { transform: translateY(0);    opacity: 1; }
  }
  .moment { animation: drift-up 1.4s cubic-bezier(0.22, 1, 0.36, 1) both; }
  .kicker { animation: drift-up 1.8s cubic-bezier(0.22, 1, 0.36, 1) 0.4s both; }
</style></head>
<body>
  <div class="frame">
    <div class="moment line-1">A place, <em>not</em><br>a product.</div>
    <div class="kicker">Marrakech Intense — Eau de Toilette</div>
  </div>
</body></html>
```

Design intent: print-magazine pacing. Negative space carries the weight;
the type is small relative to the canvas and offset off-center.

### Example 3 — Kinetic: single-word per-beat reveal

```html
<!doctype html>
<html><head><style>
  :root {
    --ease-out-back: cubic-bezier(0.34, 1.56, 0.64, 1);
    --ease-out-expo: cubic-bezier(0.16, 1, 0.3, 1);
  }
  html, body { margin: 0; background: transparent; height: 100%; font-family: "Inter", sans-serif; }
  .stage {
    position: absolute; inset: 0;
    display: grid; place-items: center;
  }
  .beat {
    position: absolute;
    font-weight: 900;
    font-size: 18vw;
    line-height: 1;
    color: #fff;
    opacity: 0;
    letter-spacing: -0.02em;
  }
  @keyframes pop {
    0%   { transform: scale(0.82); opacity: 0; }
    18%  { transform: scale(1.02); opacity: 1; }
    32%  { transform: scale(1);    opacity: 1; }
    78%  { transform: scale(1);    opacity: 1; }
    100% { transform: scale(0.96); opacity: 0; }
  }
  .beat:nth-child(1) { animation: pop 0.9s var(--ease-out-back) 0.0s both; }
  .beat:nth-child(2) { animation: pop 0.9s var(--ease-out-back) 0.9s both; }
  .beat:nth-child(3) { animation: pop 0.9s var(--ease-out-back) 1.8s both; }
  .beat:nth-child(4) { animation: pop 1.2s var(--ease-out-expo) 2.7s both; color: #ffd400; }
</style></head>
<body>
  <div class="stage">
    <div class="beat">RUN.</div>
    <div class="beat">FALL.</div>
    <div class="beat">RUN.</div>
    <div class="beat">AGAIN.</div>
  </div>
</body></html>
```

Design intent: one word at a time. Last word breaks the color rule for
emphasis. The pop / dwell / fade timing is the whole effect.

### Example 4 — Luxury whisper: thin weight, near-invisible

```html
<!doctype html>
<html><head><style>
  @import url("https://fonts.googleapis.com/css2?family=Inter:wght@200&display=swap");
  html, body { margin: 0; background: transparent; height: 100%; }
  .center {
    position: absolute; inset: 0;
    display: grid; place-items: center;
  }
  .whisper {
    font-family: "Inter", sans-serif;
    font-weight: 200;
    font-size: 22px;
    color: #ffffffb0;
    letter-spacing: 0.55em;
    text-transform: uppercase;
    padding-left: 0.55em;
    transition: opacity 1.6s ease-out;
  }
  @keyframes whisper-in {
    from { opacity: 0; letter-spacing: 0.2em; }
    to   { opacity: 1; letter-spacing: 0.55em; }
  }
  .whisper { animation: whisper-in 2.2s cubic-bezier(0.22, 1, 0.36, 1) both; }
</style></head>
<body>
  <div class="center"><div class="whisper">Eau de Marrakech</div></div>
</body></html>
```

Design intent: the type apologizes for being there. Wide tracking,
near-transparent fill, only barely legible. Used for luxury houses
where the product image carries the spot.

### Example 5 — Display-driven: massive number, video bleeds through negative space

```html
<!doctype html>
<html><head><style>
  html, body { margin: 0; background: transparent; height: 100%; font-family: "Helvetica Neue", sans-serif; }
  .spec {
    position: absolute; inset: 0;
    display: grid;
    grid-template-columns: 1fr auto 1fr;
    grid-template-rows: 1fr auto 1fr;
  }
  .number {
    grid-column: 2; grid-row: 2;
    font-weight: 900;
    font-size: 52vw;
    line-height: 0.8;
    color: #fff;
    mix-blend-mode: difference;
    margin: 0;
    letter-spacing: -0.05em;
  }
  .units {
    position: absolute;
    right: 4vw; bottom: 6vh;
    color: #fff;
    font-size: 14px;
    letter-spacing: 0.35em;
    text-transform: uppercase;
    text-align: right;
    line-height: 1.6;
  }
  @keyframes ramp {
    from { transform: translateY(8vh); opacity: 0; }
    to   { transform: translateY(0);   opacity: 1; }
  }
  .number { animation: ramp 1.1s cubic-bezier(0.22, 1, 0.36, 1) both; }
</style></head>
<body>
  <div class="spec">
    <h1 class="number">911</h1>
  </div>
  <div class="units">HP : 502<br>0 → 60 : 3.2s<br>SS / 26</div>
</body></html>
```

Design intent: one number fills the frame, video shows through where
the number isn't. `mix-blend-mode: difference` inverts the underlying
pixels through the type so the number is legible against any shot.

### Example 6 — Typographic mask: clip-path carves a word-shape into the video

```html
<!doctype html>
<html><head><style>
  html, body { margin: 0; background: transparent; height: 100%; }
  .stage { position: absolute; inset: 0; display: grid; place-items: center; }
  /* Hex polygon that reads as a viewing port through the scene. */
  .port {
    width: 70vw; aspect-ratio: 16 / 7;
    background: #000;
    clip-path: polygon(8% 0, 92% 0, 100% 50%, 92% 100%, 8% 100%, 0% 50%);
    mix-blend-mode: lighten;
  }
  .legend {
    position: absolute; left: 8vw; bottom: 8vh;
    font: 800 22px "Helvetica Neue", sans-serif;
    color: #fff;
    letter-spacing: 0.18em; text-transform: uppercase;
  }
  @keyframes iris {
    from { clip-path: polygon(50% 50%, 50% 50%, 50% 50%, 50% 50%, 50% 50%, 50% 50%); }
    to   { clip-path: polygon(8% 0,    92% 0,    100% 50%, 92% 100%, 8% 100%, 0% 50%); }
  }
  .port { animation: iris 0.9s cubic-bezier(0.65, 0, 0.35, 1) both; }
</style></head>
<body>
  <div class="stage"><div class="port"></div></div>
  <div class="legend">Sector — 04 / observed</div>
</body></html>
```

Design intent: a polygon clip-path masks a black plate against the
moving footage; `mix-blend-mode: lighten` keeps the brightest pixels
of the video visible through the mask. The clip-path animates from a
collapsed point to its full hexagonal aperture — an iris reveal.

### Example 7 — Mono / tech: small fixed-width corner tags

```html
<!doctype html>
<html><head>
<style>
  @import url("https://fonts.googleapis.com/css2?family=JetBrains+Mono:wght@500&display=swap");
  html, body { margin: 0; background: transparent; height: 100%; }
  .hud { position: absolute; inset: 0; padding: 20px 28px; color: #fff; font-family: "JetBrains Mono", monospace; font-size: 14px; }
  .tl, .tr, .bl, .br { position: absolute; opacity: 0.92; }
  .tl { top: 20px; left: 28px; }
  .tr { top: 20px; right: 28px; text-align: right; }
  .bl { bottom: 20px; left: 28px; }
  .br { bottom: 20px; right: 28px; text-align: right; }
  .row { display: flex; gap: 1.2em; }
  .key { color: #fff8; }
  @keyframes flicker {
    0%, 100% { opacity: 1; }
    50%      { opacity: 0.6; }
  }
  .live { animation: flicker 0.8s steps(2, jump-end) infinite; }
</style></head>
<body>
  <div class="hud">
    <div class="tl"><div class="row"><span class="key">LOC</span> 34.0522°N · 118.2437°W</div></div>
    <div class="tr"><div>T+ 00:00:04.20</div></div>
    <div class="bl"><div class="row"><span class="key">CH</span> 02 · <span class="key">F-STOP</span> 2.8</div></div>
    <div class="br"><span class="live">● REC</span></div>
  </div>
</body></html>
```

Design intent: zero hero typography. The "design" is fixed-width metadata
in four corners with a flickering REC indicator. Reads as observational,
not commercial.

### Example 8 — Editorial silence: zero text

```html
<!doctype html>
<html><head><style>
  html, body { margin: 0; background: transparent; height: 100%; }
  .vignette {
    position: absolute; inset: 0;
    background:
      radial-gradient(ellipse at center, transparent 40%, #00000050 80%, #000000a0 100%);
    transition: opacity 1s ease-out;
    animation: vignette-in 2.4s ease-out both;
  }
  @keyframes vignette-in {
    from { opacity: 0; }
    to   { opacity: 1; }
  }
</style></head>
<body>
  <div class="vignette"></div>
</body></html>
```

Design intent: editorial pacing demands a beat with no caption. A subtle
radial vignette frames the shot and that is the entire overlay. Don't
caption every scene.

### Example 9 — Vertical pour: lower-third slide-up with extended ease

```html
<!doctype html>
<html><head><style>
  :root { --ease-out-quint: cubic-bezier(0.22, 1, 0.36, 1); }
  html, body { margin: 0; background: transparent; height: 100%; font-family: "Inter", sans-serif; color: #fff; }
  .pour {
    position: absolute; left: 0; right: 0; bottom: 0;
    padding: 5vh 6vw;
    background: linear-gradient(180deg, transparent 0%, #000a 70%, #000c 100%);
    display: grid; grid-template-columns: auto 1fr; gap: 2.4rem; align-items: end;
  }
  .lockup .name {
    font-weight: 700; font-size: 36px;
    letter-spacing: -0.01em;
    margin: 0 0 0.2em;
  }
  .lockup .strap {
    font-weight: 300; font-size: 15px;
    letter-spacing: 0.16em; text-transform: uppercase;
    color: #ffffffb0;
  }
  .cta {
    align-self: end; justify-self: end;
    border: 1.5px solid #fff;
    padding: 0.7em 1.4em;
    font-size: 14px; letter-spacing: 0.22em; text-transform: uppercase;
    border-radius: 999px;
  }
  @keyframes rise {
    from { transform: translateY(8vh); opacity: 0; }
    to   { transform: translateY(0);   opacity: 1; }
  }
  .pour { animation: rise 1.2s var(--ease-out-quint) 0.2s both; }
</style></head>
<body>
  <div class="pour">
    <div class="lockup">
      <h2 class="name">Allbirds Tree Runner</h2>
      <div class="strap">Made from trees — Walk on it</div>
    </div>
    <div class="cta">Try them barefoot</div>
  </div>
</body></html>
```

Design intent: classic broadcast lower-third, but with the extended
`out-quint` curve so the rise feels weighted rather than rubbery. The
CTA pill on the right balances the type on the left.

### Example 10 — Multi-element kinetic stack with staggered eases

```html
<!doctype html>
<html><head><style>
  :root {
    --ease-out-back:   cubic-bezier(0.34, 1.56, 0.64, 1);
    --ease-out-quint:  cubic-bezier(0.22, 1,    0.36, 1);
    --ease-out-circ:   cubic-bezier(0,    0.55, 0.45, 1);
  }
  html, body { margin: 0; background: transparent; height: 100%; font-family: "Inter", sans-serif; color: #fff; }
  .stack {
    position: absolute; left: 6vw; top: 50%;
    transform: translateY(-50%);
    display: flex; flex-direction: column; gap: 0.6em;
    max-width: 56%;
  }
  .stack > * { opacity: 0; transform: translateX(-40px); }
  .stack .eyebrow { font-size: 14px; letter-spacing: 0.3em; text-transform: uppercase; color: #ffd400; font-weight: 600; }
  .stack .head    { font-size: 7vw; line-height: 1; font-weight: 900; letter-spacing: -0.03em; }
  .stack .sub     { font-size: 18px; max-width: 36ch; color: #ffffffd0; line-height: 1.4; }
  @keyframes slide-in {
    to { opacity: 1; transform: translateX(0); }
  }
  .stack > :nth-child(1) { animation: slide-in 0.6s var(--ease-out-back)  0.15s forwards; }
  .stack > :nth-child(2) { animation: slide-in 0.9s var(--ease-out-quint) 0.35s forwards; }
  .stack > :nth-child(3) { animation: slide-in 1.1s var(--ease-out-circ)  0.65s forwards; }
</style></head>
<body>
  <div class="stack">
    <div class="eyebrow">FIELD TEST · DAY 12</div>
    <h1 class="head">Built for the long walk home.</h1>
    <p class="sub">Eucalyptus-fiber upper. Sugarcane sole. Machine washable. The kind of comfort you forget about — until you take them off.</p>
  </div>
</body></html>
```

Design intent: three vertically-stacked elements, each animating in on
a different ease curve from the extended table. Staggered by
`animation-delay`. The contrast between the back / quint / circ feel is
the design — three siblings, three personalities.

### Example 11 — Difference-mode title over a generated shot

```html
<!doctype html>
<html><head><style>
  html, body { margin: 0; background: transparent; height: 100%; font-family: "Helvetica Neue", sans-serif; }
  .stage { position: absolute; inset: 0; display: grid; place-items: end center; padding: 6vh 0; }
  .knockout {
    font-weight: 900;
    font-size: 16vw;
    line-height: 0.92;
    color: #fff;
    mix-blend-mode: difference;
    letter-spacing: -0.035em;
    text-align: center;
    margin: 0;
  }
  @keyframes settle {
    0%   { letter-spacing: 0.2em;   opacity: 0; }
    100% { letter-spacing: -0.035em; opacity: 1; }
  }
  .knockout { animation: settle 1.3s cubic-bezier(0.16, 1, 0.3, 1) both; }
</style></head>
<body>
  <div class="stage">
    <h1 class="knockout">EVERYWHERE.<br>NOWHERE.</h1>
  </div>
</body></html>
```

Design intent: the canonical "white type that carves through video".
`mix-blend-mode: difference` makes the type readable over any underlying
footage by inverting whatever's behind it. The kerning animation from
wide to tight is the entry move.

## What does NOT work

The agent should never reach for these — they're silent no-ops at time
of writing. If you write them, expect nothing to render.

- **`background-clip: text`** — wb-wudl tracks this. Workaround: render
  the gradient as a background on the element (the type itself stays
  solid color) or use SVG `<text>` with a gradient fill.
- **`text-shadow` (any variant)** — wb-o7s0 tracks this. Workaround:
  stack two translated copies of the text with reduced opacity and a
  bit of `transform: translate(1px,1px)` per layer to fake a glow.
- **`backdrop-filter`** — Stylo doesn't expose the property under the
  Servo feature flag blitz-dom uses (wb-3v87 tracks). The paint
  pipeline can do it; the value isn't reachable. Use a translucent
  solid scrim or a gradient scrim as a workaround for now.
- **`clip-path: inset() / ellipse() / path() / xywh() / rect() /
  url(#mask)`** — only `circle()`, `polygon()`, and box keywords ship.
  Use a polygon for inset-with-radius shapes; use `border-radius` plus
  `overflow: hidden` for plain rounded rectangles.
- **`mask-image` / `mask`** — not wired through Stylo's style
  computation. Use `clip-path: polygon(…)` as the closest substitute.
- **JavaScript** — `<script>` tags are silently ignored. All animation
  goes through CSS. There is no canvas, no JS-driven render hook, no
  user event loop. Don't write JS.
- **`<video>` element inside scene HTML** — not painted today. The
  per-scene background video comes from the scene's `video_bg` field
  (in `comp.json`) or via `<section data-scene-href="…">` in
  `index.html`. Inline `<video>` is on the roadmap (wb-9h9u); once it
  lands you'll place `<video src="shot.mp4" autoplay muted>` inline in
  the scene HTML. **Today, use `video_bg` / `data-scene-href`.**
- **`<audio>` element inside scene HTML** — same. Audio goes in
  `comp.json`'s `audio_cues: [...]` array or as `<audio>` at the
  top-level of `index.html`. Inline `<audio>` inside a scene is on the
  roadmap (wb-ga6s).
- **`position: sticky`, `:hover`, `:focus`, scroll-driven anything** —
  there is no scroll, no hover, no focus in offline video render.
  Selectors that depend on user interaction don't fire.
- **`<iframe>`** — not supported.

## Step 7 — assemble the multi-scene manifest

The canonical authoring path is a top-level `index.html` that lists the
scenes and their audio cues. The renderer parses it via
`packages/gamut/src/compose/mod.rs` and resolves relative paths against
the manifest's parent directory.

```html
<!doctype html>
<html><head>
  <title>Tree Runner Spot</title>
  <meta name="resolution" content="1280x720">
  <meta name="fps" content="30">
  <meta name="duration" content="15s">
</head><body>
  <section data-scene-href="scenes/01-title.html"   data-duration="3s"></section>
  <section data-scene-href="scenes/02-product.html" data-duration="6s"
           data-transition-in="crossfade" data-transition-duration="0.5s"></section>
  <section data-scene-href="scenes/03-detail.html"  data-duration="3s"></section>
  <section data-scene-href="scenes/04-cta.html"     data-duration="3s"
           data-transition-in="crossfade" data-transition-duration="0.4s"></section>

  <audio src="music/track.wav" data-spans="all" data-volume="0.8" data-fade-in="0.4s" data-fade-out="1s"></audio>
  <audio src="vo/line.wav"     data-start="6s" data-duration="3s" data-fade-in="0.2s"></audio>
</body></html>
```

Required `<meta>`: `resolution` (`WxH`) and `fps`. `duration` is
optional — if omitted, the composition duration is the sum of scene
durations.

**`<section>` attributes:**

| Attribute                  | Required | What it means                              |
|----------------------------|----------|--------------------------------------------|
| `data-scene-href`          | yes      | Relative path to the scene HTML file       |
| `data-duration`            | yes      | `3s`, `1500ms`, or plain integer seconds   |
| `data-transition-in`       | no       | `cut` (default), `crossfade`, `fade`, `shader:<name>` |
| `data-transition-duration` | no       | Duration of the transition; default `0.5s` |

**`<audio>` attributes:**

| Attribute       | What it means                                                  |
|-----------------|----------------------------------------------------------------|
| `src`           | Asset path (relative). REQUIRED.                                |
| `data-spans`    | `all` — bind to the full composition duration                  |
| `data-start`    | Start offset, default `0s`                                     |
| `data-duration` | Explicit duration; default 0 (use until end)                   |
| `data-fade-in`  | Fade-in duration                                                |
| `data-fade-out` | Fade-out duration                                              |
| `data-volume`   | Float 0..1, default `1.0`                                      |

Per-scene background video is currently still wired via `comp.json`'s
`video_bg` field — the `<section>` manifest reads that path from the
scene's parent `comp.json` when present, otherwise the scene renders
with transparent background only. Once wb-9h9u lands the scene HTML
itself will carry the `<video>` tag inline.

Render with:

```bash
gamut render index.html -o commercial.mp4
```

The CLI auto-detects `.html` vs `.json` and routes to `load_index_html`
or `Composition::from_json_path` accordingly.

## Step 7.5 — the comp.json escape hatch

For tests, advanced motion control, and per-scene `video_bg` wiring
you'd rather not stuff into the HTML manifest, `comp.json` is the
lower-level format. The same `gamut render comp.json -o out.mp4`
command works.

```json
{
  "width": 1280,
  "height": 720,
  "fps": 30,
  "duration_frames": 360,
  "scenes": [
    {
      "html_path": "scenes/01-title.html",
      "video_bg": "shots/shot-1-saguaro.mp4",
      "start_frame": 0,
      "duration_frames": 90
    }
  ],
  "audio_cues": [
    {
      "id": "music",
      "asset_path": "music.wav",
      "start_frame": 0,
      "duration_frames": 360,
      "volume": 0.7,
      "pan": 0.0,
      "fade_in_frames": 6,
      "fade_out_frames": 24,
      "duck_targets": [],
      "duck_db": 0.0
    }
  ]
}
```

Required fields per scene: `html_path`, `start_frame`, `duration_frames`.
Optional: `video_bg`, `transition_in`. Frame math: at 30fps,
`duration_frames = 30 * secs`. Scene durations should sum to the total —
don't leave gaps.

Reference: `packages/gamut/examples/arizona/comp.json` is a working
multi-scene composition.

> Historical note: a `motion: [...]` array used to live on each scene
> as a 7-property enum (`x, y, scale, rotate, opacity, width, height`).
> That layer is **retired** — animation lives in standard CSS now via
> Stylo's `@keyframes` + `transition` engine. Don't reach for it.

## Step 8 — render and (optionally) re-mux

```bash
gamut render index.html -o commercial.mp4
```

When the composition has audio cues, render emits a sidecar `.wav`
alongside the video. If you need a finer-grained audio path (different
codec, additional ducking, ffmpeg-side normalization), re-mux:

```bash
ffmpeg -y -i commercial.mp4 -i commercial.wav \
  -c:v copy -c:a aac -b:a 192k -shortest \
  commercial.muxed.mp4
```

## Step 8.5 — provenance signing (C2PA)

EU AI Act Article 50 enforcement begins **August 2026**. Sign at render time:

```bash
gamut render index.html -o commercial.mp4 \
  --sign-c2pa --title "Brand spot v3" --author "Studio name"
```

Or retroactively:

```bash
gamut c2pa sign commercial.mp4 -o commercial.signed.mp4 \
  --comp index.html --title "Brand spot v3" --author "Studio name"
gamut c2pa verify commercial.signed.mp4
```

The bundled test cert chains to a non-trusted root — fine for
development, not for delivery. For production, BYO cert chain that
traces to a C2PA-trusted root via `--signing-cert` + `--signing-key`.

## Step 8.6 — optional premium finish (Topaz Astra 2)

Manual post-step. Drag `commercial.mp4` into Topaz Astra 2, pick the
`Proteus` preset, export to `commercial.astra.mp4`, then re-sign:

```bash
gamut c2pa sign commercial.astra.mp4 --comp index.html
```

Astra's re-encode invalidates the original C2PA hash, so re-signing
is mandatory.

## Verifying the result

Extract a frame from the rendered video and spot-check:

```bash
ffmpeg -ss 1.5 -i commercial.mp4 -vframes 1 frame.jpg
```

Confirm: the expected scene's subject is visible, the HTML overlay is
readable, the frame matches the AI-generated content (not a fallback).
Pull a frame at each scene boundary (0.5s, mid-scene, last 0.5s) to
verify the CSS animations actually played.

## Budget guidelines

Default ceiling: **$1.00 total** for a 12-second commercial.

- Music (12s × $0.005/s): ~$0.06
- Shots (4-6 × $0.10): $0.40-0.60
- Re-render: free (cache hits)
- Re-generate one shot: $0.10
- Final mux: free

Always pass `--max-cost <N>` on every billed call. The CLI refuses
requests where estimate exceeds the budget.

## File layout convention

```
/tmp/gamut-commercial/
  brief.md
  script.fountain
  screenplay.json
  velocity.json
  storyboard.json
  transitions.json
  music/
    track.wav
  vo/
    line.wav            (optional)
  eases.css             (the :root block from skills/gamut-director/eases.css)
  scenes/
    01-title.html
    02-canyon.html
    03-vista.html
    04-road.html
  shots/
    still-1.png         (Path B: scene-still per scene)
    still-2.png
    shot-1-saguaro.mp4
    shot-2-canyon.mp4
    shot-3-sedona.mp4
    shot-4-road.mp4
  index.html            (the multi-scene manifest — feed this to gamut render)
  commercial.mp4        (the deliverable)
  commercial.wav        (sidecar audio if you need to re-mux)
```

## Common pitfalls

- **Stock-looking generated shots:** Prompts too generic. Add subject
  specifics, composition, camera type. Not "a desert" — "a single
  saguaro silhouetted against a flame-orange sunset, mountains on the
  horizon, wide low-angle, cinematic".
- **Black frames in render:** A `video_bg` path didn't resolve. Check
  paths in `comp.json` (or your wiring) are relative to the manifest's
  directory.
- **Audio out of sync:** Music duration must equal total render
  duration. Generate music to match (`--duration <total_secs>`) or use
  `<audio data-spans="all">` to bind to the comp's duration explicitly.
- **Continuity check fails:** Two adjacent shots cross the 180° line.
  Reorder shots in the screenplay or add a `WHIP PAN TO:` /
  `SMASH CUT TO:` between them.
- **Wan output doesn't match prompt:** Re-roll with a different seed
  (`--seed <N>`) or rewrite the prompt to be more specific.
- **Animations don't play in the rendered MP4:** Verify the scene HTML
  uses standard CSS `@keyframes` (not the retired `motion: [...]` JSON
  format). Stylo runs the clock per-frame; bad `cubic-bezier()`
  arguments outside `[0,1]` for x-axis silently fall back.
- **Spot looks like every other AI ad:** You used the AI-default
  lockup. Same Inter-88px-bottom-left across all four scenes, no
  `clip-path`, no `mix-blend-mode`, every animation on plain `ease`.
  Go back, vary the typography per scene, reach into the extended
  ease table on at least two cuts, use `mix-blend-mode: difference`
  on at least one title.
- **`filter: blur(8px)` did nothing:** Filter support hasn't landed —
  see "What does NOT work". Pre-blur the underlying shot in the image
  gen prompt, or accept the overlay can't blur the backdrop.
- **`background-clip: text` didn't apply a gradient to type:** Same —
  not shipping yet. Render the gradient as the element's background
  (solid type, gradient field) or move to an SVG `<text>` with
  `<linearGradient>` fill.

## When you're done

Report:

- Path to the final muxed MP4
- Total billed spend
- Which generation steps succeeded / required retry
- Anything in the brief you couldn't honor (and why)

Don't open the file for the user — they'll do that themselves.

---

*Last updated 2026-05-19 — closes bd issue **wb-ndw7** (child of epic
wb-e8jh). Verify the "shipping today / coming soon" lists in this doc
against the current state of `vendor/blitz-paint/src/render/` and
`packages/gamut/src/compose/` on every release; this file describes the
palette as of that date.*
