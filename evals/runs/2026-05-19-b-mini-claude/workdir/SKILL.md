---
name: gamut-director
description: Use when the user wants to produce a short generative video — a 10-30 second commercial, brand spot, montage, or trailer — entirely from a written brief, using only the `gamut` CLI + Fal AI backends + the web for reference. Triggers on "make a commercial", "generate a video ad", "produce a spot", "direct a video from a brief", "end-to-end generative video".
---

# gamut-director — end-to-end generative video

You are the director. Take a brief (or invent one), produce a finished MP4.
Every visible frame and every audible sample is AI-generated. No stock
footage, no hand-edited timeline, no manual asset wrangling.

## Tools you need

- **`gamut` CLI** — the entire pipeline. Single binary at
  `packages/gamut/target/debug/gamut` from the repo root, or just `gamut`
  if it's on PATH.
- **The web** — research the subject of the commercial (palette, mood,
  reference shots) to inform your prompts. Use WebSearch/WebFetch.
- **Bash** — for parallel shot generation and one final ffmpeg mux step.
- **`FAL_KEY`** — pre-exported in env. Don't print it. The CLI reads it.

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
About 22% more than the legacy compose pipeline ($0.115); buys away
cutout seams and the floating-subject look.

**The one hard rule for Path B.**

**Pick 1-3 high-quality reference photos of the *same* product. All
shots in the spot derive from those refs via scene-still gen.**
Different scenes come from different *scene prompts*, not from
different reference photos of different cars. Refs lock the product
identity; scene prompts vary the world around it. Use refs of the
same car / same watch / same sneaker (different angles of the same
unit are fine and helpful). Do NOT mix a 911 ref with a Cayman ref —
the model will blend silhouettes between shots and the identity will
drift.

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
            ├─ write scene HTML overlays
            ├─ build comp.json
            └─ gamut render + ffmpeg mux → commercial.mp4
```

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
format** popularized by veo3gen.app and now standard across the AI
commercial community. One slot per line, in any order:

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

Each slot maps cleanly into downstream L-Storyboard slot generation
(`attributes` in step 3.25) and the eventual model prompts. Forcing
the agent to commit to specifics on each axis — rather than writing
prose that hand-waves over the hard choices — is the whole point.

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
# OK
# (or: missing slots / parse errors / "fewer than 3 words" warnings)
```

`gamut brief check` is pure parsing — no LLM, no network. It catches
missing slots, non-numeric runtimes, duplicate slots, and flags
suspiciously short slots (< 3 words on AUDIENCE / INSIGHT / PROMISE /
PROOF) or prose-shaped slots (> 20 words on TONE / CALL).

**Long-form briefs are still acceptable as input.** When a human hands
you a prose brief, your job is to **distill** it into the 9-line shape
*before* moving to step 2. Don't pipe prose into the screenplay
stage — the downstream stages assume slot-filled input.

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

These are all free, deterministic, and reversible:

```bash
gamut screenplay parse script.fountain --pretty -o screenplay.json
gamut velocity propose script.fountain --pretty -o velocity.json
gamut storyboard plan script.fountain --velocity velocity.json --pretty -o storyboard.json
gamut storyboard verify storyboard.json
gamut continuity check storyboard.json
gamut transitions classify script.fountain --velocity velocity.json --pretty -o transitions.json
```

**Read each output.** The velocity profile's `mean_bpm` field (top
level of `velocity.json`) tells you the music's target tempo —
duration-weighted average across all anchors. `duration_secs` tells
you how long gamut *estimates* the screenplay would take on screen (a
heuristic — 2.5 wps for action, 1.2s min per dialogue beat). You
don't have to honor that — pass `--duration <secs>` to `music gen` to
hit a specific length. The storyboard's `shots[].subject` tells you
what each shot is about. The continuity report flags any 180° /
motion / scale-jump issues — if there are errors, reorder the
screenplay or add a transition before continuing.

## Step 3.25 — fill structured shot attributes (L-Storyboard, encouraged)

`Shot` now carries an optional `attributes` block — the L-Storyboard
schema from arXiv 2505.12237. Seven typed slots replace freeform prose
in the eventual model prompt:

| Slot     | What it captures                                   |
|----------|----------------------------------------------------|
| subject  | what the shot is OF                                |
| action   | what's happening                                   |
| scene    | where it is (location + time of day + environment) |
| camera   | shot type + focal length + angle                   |
| lens     | optical character — DoF, anamorphic, fringe        |
| lighting | direction + quality of light                       |
| style    | aesthetic register, film stock, color grade        |

When you have enough information after research, fill every slot for
every shot. The assembler joins them in fixed order
(`{subject}. {action}. {scene}. Shot: {camera}, {lens}. Lighting:
{lighting}. Style: {style}.`) so identical attribute sets produce
identical prompts across runs — important for backend caching and
regression tests.

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

When `attributes` is present, the per-shot prompt fragment is
`attributes.to_prompt()` — the legacy `Generation`-payload + shot-type
suffix is skipped. When `attributes` is absent (or under-specified
shots where you genuinely don't know the lens / lighting / style yet)
the freeform path keeps working unchanged.

All seven slots are required — if you don't know one, write the literal
string `"unspecified"` rather than leaving it blank. Empty strings fail
validation; explicit "unspecified" surfaces the omission for later
review. Reference fixture: `packages/gamut/tests/fixtures/l-storyboard-example.json`.

## Step 3.26 — let an LLM fill the slots (`gamut director synthesize`)

Hand-writing seven slots per shot is mechanical work the agent
shouldn't have to do for every spot. `gamut director synthesize` reads
a brief + a storyboard skeleton (each shot already has `id` + `subject`
+ a `Generation` payload) and asks an LLM (Gemini 2.5 Pro by default;
Claude Opus 4.7 via `--model claude`) to fill every shot's seven slots
in one call. The orchestrator:

- Sends the canonical creative-director system prompt + your brief +
  the shot list as JSON.
- Parses the returned `{shots:[…]}` JSON (markdown fences and leading
  prose are tolerated).
- Validates every returned shot via `ShotAttributes::validate()`.
- Retries ONCE with a follow-up listing any empty slots; errors after
  the second attempt.
- Merges the populated attributes into each `Shot.attributes` and
  writes the new storyboard.

```bash
# default: Gemini 2.5 Pro via fal-ai/any-llm, ~$0.02–$0.05 / spot
gamut director synthesize brief.md storyboard.json -o storyboard.dir.json --pretty

# style override applied to every shot
gamut director synthesize brief.md storyboard.json -o storyboard.dir.json \
  --style-anchor "A24-flavored, 35mm grain, dusk palette"

# Claude Opus 4.7 fallback when Gemini's drifting on consistency
gamut director synthesize brief.md storyboard.json -o storyboard.dir.json --model claude
```

### When to use this vs. hand-writing attributes

| Use `director synthesize` when…             | Hand-write attributes when…                  |
|---------------------------------------------|----------------------------------------------|
| You have a brief and 4–12 shots to fill.    | You have a specific creative vision per shot.|
| Lens / lighting / style should match across the spot but no shot needs a unique look. | A particular shot needs an off-pattern lens or lighting (e.g. the closer is harsh fluorescent while the rest is golden hour). |
| You want a reasonable starting point and will lightly tune two or three slots. | The spot is two shots — faster to write than to prompt-engineer. |
| The brief is rich (palette, lens family, time of day, tone references). | The brief is one sentence — the LLM will hallucinate scene specifics. |

The LLM-written output is usually 70–80% of director-grade — same lens
family across shots, consistent lighting motif, reasonable camera
phrasing. Read the output and patch the two or three slots that drift
(common pattern: scene descriptions wander away from the literal
brief — e.g. "kitchen" becomes "design studio" in shot 2 of 4). The
agent's role here is *direction*, not transcription — push back when a
slot is wrong.

## Step 3.5 — generate the voiceover (optional)

If the commercial calls for narration, generate it BEFORE music so you
know how long the VO is (and can time the music to match).

```bash
gamut dialogue tts "<your VO copy>" \
  --backend fal-kokoro \
  --voice af_nicole \
  --max-cost 0.05 \
  --out vo.wav \
  --pretty
```

Backends:
- `fal-kokoro` — fast, open-weight, Fal-hosted. Default for our stack.
  Voice ids include `af_nicole` (female), `af_bella`, `am_adam` (male),
  `am_michael`. Pick one that suits the brand.
- `elevenlabs` — requires `ELEVENLABS_API_KEY` env var with
  `text_to_speech` permission on the key.

Write VO copy that:
- Fits the total duration (~2.5 words/sec is a comfortable read pace,
  so 30s commercial ≈ 60-75 spoken words max — including pauses)
- Lands the brand or model name CLEARLY (preferably with a 1-second
  pause before for emphasis)
- Has a clear call-to-action or tagline at the end

Listen to the result (`open vo.wav`) or check `audio_bytes` in the
JSON output to confirm it's reasonable (typically 40-60 KB/sec for
Kokoro WAV).

### Word-level captions (CapCut / Hormozi / minimal)

Kinetic captions are table-stakes for AI commercial spots in 2026 —
~0.25-0.4s dwell per word, single emphasis word per beat, bottom-third
placement. Generate them straight from the VO you just produced.

```bash
# 1. Align: VO audio → per-word timestamps (JSON)
gamut dialogue captions \
  --audio vo.wav \
  --text "Fast cheap reliable big wins" \
  --backend fal-whisper-words \
  --max-cost 0.10 \
  --style hormozi \
  -o captions.json \
  --pretty

# 2. Render the HTML overlay
gamut captions overlay \
  --in captions.json \
  --style hormozi \
  --width 1080 --height 1920 \
  -o caption.html
```

Two backends:
- `fal-whisper-words` — Fal-hosted Whisper with `chunk_level: "word"`.
  Native per-word `[start_ms, end_ms]` timestamps. ~$0.001 per 10s of
  audio. Local files are uploaded to fal-storage automatically.
- `synthetic` — distributes the VO duration evenly across the words of
  `--text`. Requires `--duration-ms`. No API call, no cost, no
  intonation awareness. Use as a fallback when `FAL_KEY` is unset.

Three style presets:
- **`hormozi`** — single word at a time, large bottom-center, yellow
  highlight on the emphasis word per beat (longest word in each
  4-word window; ALL-CAPS words outrank longer mixed-case neighbours).
  Dwell ~0.25-0.4s. Best for high-energy direct-response spots.
- **`capcut`** — sliding groups of 2-3 words, soft fade from below,
  dwell ~0.5-0.8s per group. Best for "explainer" / lifestyle pacing.
- **`minimal`** — full sentence at once, classic broadcast lower-third
  with a single fade in/out. Best for editorial spots and luxury brands
  where word-by-word kinetics feel cheap.

Drop the emitted `caption.html` into your scene overlay flow as a sibling
HTML scene with the same duration as the VO. The CSS uses `@keyframes`
only — no JS — so it renders correctly under Blitz / RVST / headless
browsers without a runtime.

Emphasis detection is a v0 heuristic (longest word, ALL-CAPS preferred).
Future work: ML-driven prosody emphasis (TBD follow-up issue).

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

### Backend — commercial safety matters

The default `--backend elevenlabs` (ElevenLabs Music) is the **only
commercially-safe** option. ElevenLabs licenses its training catalog
through Merlin + Kobalt — the two largest music-industry rights
aggregators — so output is licensed for commercial use by construction.

- `--backend elevenlabs` *(default)* — Merlin + Kobalt-licensed.
  Requires `ELEVENLABS_API_KEY` with the `music_generation` permission
  (TTS-only keys 401 on this endpoint). Output is MP3. Min duration 3s,
  max 600s.
- `--backend udio` — Udio partnership tier, alternative for orchestral
  hero cues. Live calls are not yet wired (dry-run works; planned in
  wb-23j2).
- `--backend fal-musicgen` — **DEPRECATED**. MusicGen has pending
  training-data litigation; output is **not** commercial-safe. Kept
  for experimentation only; the CLI prints a deprecation warning on
  every invocation.

Style prompt guidance:
- Match the commercial's mood (luxury → "cinematic strings, slow build,
  warm reverb"; energy → "driving percussion, kick drum, electronic")
- Include "no vocals" if the music shouldn't compete with VO
- The CLI auto-appends the velocity arc to your prompt, so don't repeat
  BPM markers — just describe the *style*

Validate:
```bash
gamut velocity validate velocity.json --against music.wav --tolerance 20 --pretty
```

The validator looks for percussive onsets. Soft ambient music produces
0 detected onsets; that's not a failure, just means the validator
can't grade it. If you want strong validation, prompt for a percussive
style.

### Snap shot boundaries to onsets

`velocity validate` now also writes a sibling `music.cuts.edl` (FCP7 /
Resolve compatible) when any onsets were detected, and the JSON report
carries a `detected_onsets_ms` field with the full onset list. Use
those onsets as snap targets for shot boundaries — per Curious Refuge,
"snap title cards to the loudest onset within ±1 frame". To regenerate
just the EDL without re-validating:

```bash
gamut velocity onsets-to-edl --music music.wav --fps 30 -o cuts.edl
```

This composes with frame-chaining (wb-fr2g/G1, already shipped):
onset-snapped shot boundaries + first-last-frame chaining together
give beat-aligned, visually-continuous cuts. Suppress the sibling EDL
with `--no-emit-edl` if you only want the JSON report.

## Step 5 — generate the shots

### Path A: txt2vid (brand-vibe)

One `gamut shot txt2vid` call per scene. **Run them in parallel**
(separate Bash calls with `run_in_background: true`) since each takes
30-90 seconds.

```bash
gamut shot txt2vid \
  "<rich, specific prompt>" \
  --duration 5 \
  --max-cost 0.20 \
  --out shots/shot-N-<name>.mp4 \
  --pretty
```

Prompt construction (in this order, comma-separated):
1. Subject (`a giant saguaro cactus`)
2. Action (`stands sentinel`)
3. Setting (`Arizona desert at golden hour`)
4. Composition (`wide low-angle shot`)
5. Atmosphere (`dramatic clouds, lens flare`)
6. Tech (`cinematic, photorealistic, shallow depth of field`)

Each shot is 5 seconds at 16fps native (Wan-T2V default). The renderer
handles fps mismatch automatically and holds the last frame if a scene
overruns the clip (so request scene durations ≤ 5s, or generate longer
shots upstream).

#### Negative prompts — the standard set is automatic

Every `gamut shot txt2vid` / `img2vid` call automatically appends a
canonical negative prompt — `"no text overlay, no watermark, no
distortion, no extra limbs, no extra fingers, low quality, blurry"` —
documented to cut unusable outputs by ~30% per the Artlist 2026 guide.
You never have to type the standard negatives. Only use `--negative`
when the shot needs *additional* don'ts on top of the default (e.g.
`--negative "morphing, jittery motion"` for hand-animation shots). The
adapter merges your addition with the default. Pass
`--no-default-negatives` only for adversarial experiments.

### Path B: reference-conditioned scene-still gen (2-step pipeline per shot)

**Step B.1 — Collect 1-3 reference photos of the exact product.**
Use `WebSearch` to find direct image URLs (not page URLs) — Wikipedia
and manufacturer press kits are good sources. Prefer photos with:

- Solo subject (no bystanders, no other vehicles in frame)
- Neutral, even-light backgrounds (parking lot, studio, plain road)
- No baked-in watermarks (check the corners)

Why 1-3 and not always 1: a side profile + a front 3/4 + a rear angle
of the *same unit* lets the scene-still model understand the
silhouette from multiple sides. All refs must be the same product —
same model, same color, same trim, same year.

> HTTPS URLs only for now. `wb-m9qe` added local-path support to the
> isolate path, but the scene-still CLI hasn't been updated to use it
> yet. If you find that's been done, the `--refs` flag should accept
> local paths transparently; otherwise file a follow-up.

**Step B.2 — For each scene, generate a scene-aware still.**

```bash
gamut image scene-still \
  --refs "https://.../ref-1.jpg,https://.../ref-2.jpg" \
  --prompt "the car at golden hour on a coastal cliff highway, wide low-angle, dramatic side-light, no other vehicles" \
  --image-size landscape_16_9 \
  --max-cost 0.05 \
  --out shots/still-N.png \
  --pretty
```

Wraps Fal Seedream (reference-conditioned txt2img). The refs lock the
product identity; the prompt describes the scene's lighting, angle,
camera, and atmosphere. Be specific:

- ✓ `"the car at golden hour on a coastal cliff highway, wide
   low-angle, dramatic side-light, no other vehicles"`
- ✓ `"the watch resting on a polished black stone slab, top-down
   macro, single soft key light from upper-left, deep shadows"`
- ✗ `"a car driving fast"` (no scene detail; the model has nothing
   to hang the product on)

Scene prompts vary; the `--refs` list stays the same across every
shot in the spot. That's what gives the product identity continuity.

**Hero-frame pre-gen (Curious Refuge "lock the look").** Before running
per-shot scene-still calls, generate a single 4-panel composite covering
the spot's 4 key moments — one Seedream call returns a 1×4 strip (or
2×2 grid) where the model is forced to hold consistent palette,
lighting direction, and color temperature across all four panels
simultaneously. Crop the composite into per-shot reference stills,
then condition every per-shot scene-still on the matching panel.

```bash
gamut storyboard hero-panels storyboard.json \
  -o shots/hero/ \
  --refs "https://.../ref-1.jpg" \
  --layout 2x2 \
  --max-cost 0.10 --pretty
# writes shots/hero/composite.png + panel-{1..4}.png + panels.json

# Then for each downstream scene-still, pass the matching panel:
gamut image scene-still \
  --hero-panel-ref "https://.../panel-2.png" \
  --refs "https://.../ref-1.jpg" \
  --prompt "..." --max-cost 0.05 --out shots/still-2.png
```

Cost: $0.04 hero-panels + 4 × $0.04 scene-still = $0.20 total vs $0.16
without — one extra Seedream call for substantially better
shot-to-shot palette + lighting coherence across the spot. Seedream
sometimes returns a 2×2 layout even when you ask for 1×4; pass
`--layout 2x2` when the storyboard is portrait/square or when you
want denser tiles. The `panels.json` manifest maps shot ids → panel
paths so the per-shot calls always pick the right one.

**Two-tier still gen.** The first 1-2 shots (the ones that define the
spot's "look") should run through the premium tier — they lock the
palette, lighting, and product placement that every later shot
conditions on. Bulk shots run on the cheap tier conditioned on the
hero stills.

- **Hero shots:** `--backend nano-banana-pro` (~$0.24/img, up to **14**
  refs, **4K** output, 94-96% in-image text accuracy). Pass your raw
  product refs.
- **Bulk shots:** default `--backend seedream` (~$0.04/img, up to 10
  refs). Pass the hero stills as `--refs` so palette + lighting carry
  across the spot.

```bash
# Hero shot — locks the spot's look
gamut image scene-still --backend nano-banana-pro \
  --refs "https://.../ref-1.jpg,https://.../ref-2.jpg" \
  --prompt "..." --max-cost 0.30 --out shots/hero-1.png

# Bulk shot — conditioned on the hero still (upload hero to a public URL
# first; local-path refs ship with wb-m9qe)
gamut image scene-still \
  --refs "https://.../hero-1.png,https://.../ref-1.jpg" \
  --prompt "..." --max-cost 0.05 --out shots/still-3.png
```

**Step B.3 — img2vid each scene-still.**

```bash
gamut shot img2vid shots/still-N.png \
  "<motion prompt>" \
  --duration 5 --max-cost 0.15 \
  --out shots/shot-N.mp4 \
  --pretty
```

Because the scene-still already contains a full world (lighting,
horizon, ground plane, subject in correct perspective), i2v produces
meaningful camera motion rather than tiny nudges around a floating
cutout.

Motion prompts (keep SHORT — 1 sentence max):
- `slow push in, cinematic parallax`
- `dolly left to right`
- `slight reveal, the subject emerges from shadow`
- `subtle handheld float, atmospheric`

### Validating each shot

Extract a frame at t=2.5s of each shot and confirm:
1. The subject is recognizable as the named product (and matches the
   refs — same color, same trim, same silhouette)
2. The subject sits naturally in the scene (correct ground contact,
   plausible scale, lighting matches the scene prompt)
3. No bystanders / watermarks / other vehicles hallucinated in

If a shot fails: re-roll `scene-still` with a sharper scene prompt or
swap in a better ref. The expensive step (i2v at $0.10) only re-runs
on actually-different inputs, so iterate on the still first.

Budget for Path B per shot: ~$0.14
(`scene-still $0.04` + `img2vid $0.10`).

### Variant generation — roll N, pick the winner

Every still / clip gen verb accepts `--variants N` (1-8, default 1).
With `N > 1` the verb runs N gens in parallel with `seed = base + i`
and emits a JSON manifest pointing at all N cached results plus the
winner. Each variant lands in its own cache slot keyed on
`request_hash + seed`, so re-runs are free.

```bash
gamut image scene-still \
  --refs https://… --prompt "…hero shot…" \
  --variants 3 --select max-vlm \
  --max-cost 0.05 --max-variants-cost 0.15 \
  --pretty
```

When to roll variants:

- `--variants 3` for **hero shots** and **identity-critical SKUs**
  (the product on a transparent background, the closing logo plate,
  any frame the eye lingers on). One-shot picks too often hallucinate
  badge text or warp silhouette.
- `--variants 1` (default) for **filler shots**, **draft tier**, and
  **anything cost-bound**. Re-rolling 3× a $0.10 i2v clip is $0.30 per
  shot — not free.

`--select` policies:

| Policy | When to use |
|---|---|
| `max-vlm` (default) | Production — VLM-grades each variant against the brief's negative criteria; highest pass-rate wins. Tie-break by first-by-seed. Stills only — clips fall back to `first`. |
| `pairwise-tournament` | Hero shots, identity-critical SKUs — VISTA-style bracket (multi-dim VLM critique). See below. |
| `first` | Debug / mock. Variant 0 wins. |
| `user` | Emit all N + manifest; you (or the agent) pick interactively. `winner` is `null`. |
| `cheapest` | Debug only — whichever cached fastest. |

Cost gating:

- `--max-cost` continues to apply per call.
- `--max-variants-cost <USD>` is the **aggregate ceiling** across all N
  variants. The pre-call line prints `variants=3 estimated cost =
  $0.12 (3 × $0.04)` so the agent can read it before spending. For
  `pairwise-tournament` the pre-call line splits gen + judging:
  `variants=4 pairwise gen=$0.1600 + judging=$0.0300 = $0.1900`.
- If one variant errors out, the rest still produce a winner — the
  errored variant appears in the manifest with an `error` field.

#### VISTA tournament (`--select pairwise-tournament`)

`max-vlm` reduces every variant to a single integer (`pass_count`).
That works for catching obvious failures (extra limbs, baked text)
but loses signal whenever two variants both pass every criterion —
the tie-break falls to seed order, which is arbitrary.

`pairwise-tournament` (per [arXiv 2510.15831](https://arxiv.org/abs/2510.15831))
replaces the single-dim score with a **single-elimination bracket**
of pair-wise VLM critiques. Each pair, the VLM grades A vs B across
four dimensions and returns one of `{A, B, tie}` per dimension:

1. **Subject fidelity** — identity preserved, right product, right brand.
2. **Composition** — rule-of-thirds, negative space, no centered-and-flat.
3. **Lighting + color** — coherent with the prompt's atmosphere.
4. **Production polish** — sharpness, no artifacts, no anatomy errors.

The pair winner is A if A wins ≥ 3 dimensions OR `(2 wins + 2 ties)`,
symmetrically for B, otherwise lower-seed wins (`seed_tiebreak: true`
in the manifest). Bracket cost grows as `N - 1` pairs:

| N variants | Pair calls | Judging cost |
|---|---|---|
| 2 | 1 | ~$0.01 |
| 4 | 3 | ~$0.03 |
| 8 | 7 | ~$0.07 |

When to use which:

- `pairwise-tournament` — **hero shots**, **identity-critical SKUs**,
  the closing logo plate, any frame the eye lingers on. The per-pair
  rationale ends up in the manifest, so the selection is auditable
  (every round records the dim-level verdicts + a one-sentence
  explanation).
- `max-vlm` — everything else. Faster (1 VLM call vs N - 1 sequential
  pair calls), cheaper, and good enough for filler shots and draft
  tier.

The manifest grows a `bracket` array with one entry per match:

```json
{
  "round": "semi-1",
  "a_seed": 0, "b_seed": 1,
  "winner": 0,
  "judgments": {
    "subject_fidelity": "tie",
    "composition": "A",
    "lighting_color": "A",
    "production": "tie",
    "rationale": "A's framing exploits negative space; lighting on A is more cinematic."
  }
}
```

The same `--criteria` flag feeds into pairwise — the criteria get
joined as the `brief_excerpt` the VLM uses to anchor its judging.

## Step 6 — write the scene HTML overlays (be creative)

One `scenes/<id>.html` per scene composites *over* the generated video.
The renderer's HTML engine is **Blitz** — a real CSS engine with Stylo
(Servo's parallel CSS engine) and Parley for text shaping. It supports
the full web platform: CSS animations (`@keyframes`, transitions),
`transform`, `clip-path`, blend modes, gradients (linear/radial/conic),
SVG, Web fonts via `@font-face`, CSS variables, masking, filters,
flexbox/grid. Use any of it.

### Only two structural rules

1. **`html` and `body` must have `background: transparent`** so the
   generated video shows through where your HTML doesn't paint.
2. **No `<img>` or `<video>` tags** — those layers come from
   `comp.json` (`video_bg` for the moving image, `audio_cues` for
   sound).

Everything else is yours. Write the HTML the way you would for a
hand-crafted website.

### Creative ambition — innovate on every spot

Every commercial should look like it was art-directed by a different
designer. **Do not** ship the same Inter-88px-bottom-left lockup on
every spot — that's the recognizable "AI-default" pattern that flattens
the work. Make text part of the *concept*, not chrome on top of the
video.

### Principles, not templates

Read these once, then design freely. They aren't a checklist you walk
through scene by scene — they're the constraints that decide whether a
move you made works. Most of them have a tool you can call to verify
the decision after the fact.

**Edge insets.** Title-safe is the center 80% of the frame, roughly a
10% inset off each edge. Action-safe is the center 90%, a 5% inset.
Type that crosses title-safe is a deliberate choice — the kind a
brutalist or display-driven treatment makes on purpose. Type that
crosses action-safe is a mistake on most platforms; phones, broadcast
crops, and inset UI will eat it. The math lives in
`gamut::aspect::safe_areas(w, h)` and returns the right rectangles per
aspect ratio, so you don't hard-code pixel values that go wrong the
moment the spot reframes from 16:9 to 9:16.

**Lower-thirds is a convention, not the convention.** It's the right
move when the spot wants editorial calm — luxury, documentary, news,
anything where the image is the subject and type is the caption. It's
the wrong move when the spot wants energy or kinetic motion — sport,
energy drink, performance tech. In those, lower-thirds reads as
broadcast-news pastiche and kills the pulse. Don't reach for it by
default just because every YouTube tutorial does.

**Read the shot first.** A scene-still is a real artifact, not a
sketch. Generate it, then run `gamut image negative-space <png>` to see
where the eye-friendly zones actually are. The tool returns a ranked
list of grid cells with a suggested text color and scrim opacity for
each. Position type in the top-ranked zone unless you have a reason to
fight the shot. Designing typography without looking at the underlying
frame is how you get text laid over a face.

**Contrast minimums.** WCAG AA is 4.5:1 for normal text and 3:1 for
18pt+ or 14pt bold. After you pick a region, call `gamut image contrast
<png> --region X,Y,W,H --text-color #...` — if the ratio comes back
below threshold, the tool suggests a scrim color and opacity that would
lift it above. Below AA is a deliberate choice (a near-invisible
luxury whisper, a brutalist clash), not an accident. If you're shipping
below AA, you should know why.

**Reading rate.** A comfortable on-screen read is about 2.5 characters
per second, which means a one-second beat fits roughly 25 characters
before viewers fall behind. Kinetic typography that flashes per word
lives in the 0.25–0.4-second range; faster than 0.15s reads as a
flicker, not a word. Set dwell times against the string length, not
against the cut.

**Motion-vs-text legibility tradeoff.** Fast camera motion plus tight
type equals unreadable. You have two ways out: hold the camera and let
the type breathe, or use heavier weights, shorter strings, and longer
dwells. As a rough rule, if the underlying shot has more than ~50% of
frame motion per second, cap the visible text at three or four words at
a time. Don't try to letter-space-track over a whip pan.

**Check what's already in the shot.** Generated stills sometimes have
baked-in text — license plates, storefront signage, watermarks,
synthetic logos. Run `gamut image ocr <png>` to detect existing text
and avoid stacking overlays on top of it. (The OCR command is currently
stubbed pending the Fal `got-ocr` queue adapter; when that lands, this
becomes a hard check rather than a hint.)

**Vary across scenes.** Within a single spot, no two adjacent scenes
should share the same typographic treatment. Same typeface is fine —
that's the through-line. Same size *and* position *and* motion is the
"AI-default" tell. Variation across cuts isn't decoration; it's the
design.

The directions in the next section are starting points, not templates —
freely combine, invert, ignore.

Some directions to consider (mix and match across scenes within a single
spot — different treatments for different moments):

- **Brutalist** — Helvetica Black, all-caps, no kerning, text covers
  60-80% of the frame. Tight to the edges. Anti-pretty.
- **Editorial** — Didone serif (Bodoni, Playfair), generous
  letterspacing, two-line typography moments centered low.
- **Kinetic** — single word per beat, animated in via `@keyframes`,
  enter+exit fast (200ms cubic-bezier). Text IS the shot.
- **Mono / tech** — JetBrains Mono or IBM Plex Mono, small (16-22px),
  corner-tagged with running timestamps or "loc/spec/version" labels.
- **Luxury** — thin sans (Inter 200), letter-spacing wide (`0.3em`),
  dead-center, sub-22px, almost-invisible whisper.
- **Display-driven** — a single massive number (year, model, spec)
  fills the frame, the video bleeds through the negative space via
  blend modes.
- **Typographic mask** — `clip-path: url(#mask)` or
  `mix-blend-mode: difference` so the type carves into the video.
- **No type at all** — some scenes are pure visual. Don't feel obligated
  to caption every cut.

These are **inspiration**, not templates. Compose freely. Use CSS
animations for motion (`@keyframes`, `transition`). Vary the typeface,
weight, size, position, motion, and blend mode across scenes within
the same spot.

### Web fonts

Load via `@font-face` against a CDN or a local file. Google Fonts CSS
works (`@import url(...)`). Don't trust a single font to carry a whole
spot — load 2-3 if you want type-system contrast between scenes.

### When you're stuck

Look at how real ad agencies treat type. Apple's product pages, Nike's
launch films, Aesop's homepage, Bottega Veneta's seasonal campaigns,
Porsche's own marketing. Don't imitate — *steal the structural moves*
(asymmetric layout, mid-shot title cards, kinetic reveals, typographic
silence).

## Step 7 — build `comp.json`

Schema:

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
      "duration_frames": 90,
      "motion": [
        { "selector": "#title", "props": [["y", 50.0], ["opacity", 0.0]], "duration_secs": 0.8, "easing": "EaseOutQuart", "start_at_secs": 0.2, "kind": "from" }
      ]
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
Optional: `video_bg`, `motion`, `transition_in`.

Frame math: at 30fps, `duration_frames = 30 * secs`. For 12 seconds,
`duration_frames` is 360 total. Each scene's `duration_frames` should
sum to the total. Don't leave gaps.

Motion easings: `Linear`, `EaseInQuart`, `EaseOutQuart`, `EaseInOutQuart`,
`EaseOutBack`, etc. (Animato's catalog.)

Motion property names: `x`, `y`, `opacity`, `scale`, `rotation`.

Motion `kind`: `from` (animate from these values *to* the element's
current style) or `to` (animate from current *to* these values).

Reference: `packages/gamut/examples/arizona/comp.json` is a working
4-scene composition with motion + transitions.

## Step 8 — render and mux

```bash
gamut render comp.json -o final.mp4
ffmpeg -y -i final.mp4 -i final.wav \
  -c:v copy -c:a aac -b:a 192k -shortest \
  commercial.mp4
```

The render emits video-only MP4 plus a sidecar WAV. ffmpeg muxes them
into one file with embedded audio. **The muxed file is what you ship.**

## Step 8.5 — provenance signing (C2PA)

EU AI Act Article 50 enforcement begins **August 2026**. Any commercial
AI-generated deliverable shipped to EU markets after that date must
carry a C2PA content-credentials manifest declaring AI generation +
the ingredient model list, or downstream platforms (Adobe Premiere,
Sony hardware, every major CMS) will reject it. Adobe + Sony already
ship signing; agency work without it is dead-on-arrival.

Sign at render time:

```bash
gamut render comp.json -o final.mp4 \
  --sign-c2pa --title "Brand spot v3" --author "Studio name"
```

Or sign retroactively (e.g. after ffmpeg muxes audio in):

```bash
gamut c2pa sign commercial.mp4 -o commercial.signed.mp4 \
  --comp comp.json --title "Brand spot v3" --author "Studio name"
mv commercial.signed.mp4 commercial.mp4
```

Verify before delivery:

```bash
gamut c2pa verify commercial.mp4
```

The signed manifest carries:

- `c2pa.actions` — declares AI composition + per-scene parameters
- `stds.schema-org.CreativeWork` — title + author
- `c2pa.training-mining` — opt-out from AI training reuse
- Ingredient list — one entry per cached backend call (Seedream / Kling
  / EL Music / etc.), each with provider + request hash + cost
- BMFF hash chain — any post-export byte change invalidates the manifest

### Test cert vs production cert

Default `--sign-c2pa` uses a bundled ES256 **test certificate**. The
hash chain is real, but the signer chains to a non-trusted root, so
Content Credentials viewers will display "untrusted signer." This is
fine for development + internal review, **not for client delivery**.

For production, pass a real cert + key:

```bash
gamut render comp.json -o final.mp4 --sign-c2pa \
  --signing-cert /path/to/cert.pem \
  --signing-key /path/to/key.pem
```

Quick self-signed cert generation (ES256, dev-grade — replace with a
CA-issued cert for production):

```bash
openssl ecparam -name prime256v1 -genkey -noout -out signing.key
openssl req -new -x509 -key signing.key -days 365 -out signing.cert \
  -subj "/CN=Your Studio/O=Your Studio Ltd"
```

A Polar.sh-issued org cert flow is on the roadmap. Until then, BYO cert
chain that traces to a C2PA-trusted root (see https://opensource.contentauthenticity.org/docs/trust-list).

Default in v0 is **opt-in** (`--sign-c2pa`). Closer to the Aug 2026
deadline this flips to opt-out — every commercial render signs unless
you pass `--no-sign-c2pa` for an internal cut.

## Step 8.6 — optional premium finish (Topaz Astra 2)

Every shipping AI commercial above ~10M views runs a **Topaz Astra 2**
final pass. It cleans up the 720p/1080p generator output to feel like
4K studio-grade footage — sharper edges, recovered detail, less
diffusion-model hash in flat areas. The difference is visible
side-by-side; on its own you'd never notice it's missing.

This is a manual post-step, not a gamut subcommand. Topaz Astra 2 is
a desktop app (Mac + Windows), not a hosted API, so there's nothing
to wrap as a backend adapter. Run it yourself once the gamut output
is ready:

1. Open Topaz Astra 2 → drag in `commercial.mp4`.
2. Pick the "AI Enhance" preset closest to your footage (typically
   `Proteus` for AI-generated material).
3. Export to `commercial.astra.mp4`.
4. Re-run `gamut c2pa sign commercial.astra.mp4 --comp comp.json` to
   re-sign the enhanced file — the original signature is invalidated
   by Astra's re-encode.

Cost: $39/month subscription (or one-off via Topaz). Worth it for any
deliverable that will run on a billboard, broadcast, or in any
side-by-side comparison against live-action footage. Skip it for
internal review cuts.

> If/when Topaz exposes a hosted API, this becomes a `gamut finish`
> verb. Until then it's a checklist item, not a code path.

## Verifying the result

Extract a frame from the rendered video and spot-check:

```bash
ffmpeg -ss 1.5 -i commercial.mp4 -vframes 1 frame.jpg
```

Read it back. Confirm:
- The expected scene's subject is visible
- The HTML overlay text is readable
- The frame looks like the AI-generated content (not a fallback)

If a shot looks wrong, regenerate that single shot with a better prompt
and re-render — the rest of the cache stays warm.

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
  music.wav
  scenes/
    01-title.html
    02-canyon.html
    03-vista.html
    04-road.html
  shots/
    still-1.png        (Path B: scene-still per scene)
    still-2.png
    still-3.png
    still-4.png
    shot-1-saguaro.mp4
    shot-2-canyon.mp4
    shot-3-sedona.mp4
    shot-4-road.mp4
  comp.json
  final.mp4    (video only, from gamut render)
  final.wav    (audio mix, sidecar)
  commercial.mp4   (muxed — the deliverable)
```

## Common pitfalls

- **Stock-looking generated shots:** Prompts too generic. Add subject
  specifics, composition, camera type. Don't write prompts like "a
  desert"; write "a single saguaro silhouetted against a flame-orange
  sunset, mountains on the horizon, wide low-angle, cinematic".
- **Black frames in render:** A video_bg path didn't resolve. Check
  comp.json paths are relative to the comp.json's directory.
- **Audio out of sync:** Music duration must equal total render
  duration. Generate music to match (`--duration <total_secs>`).
- **Continuity check fails:** Two adjacent shots cross the 180° line
  without a sanctioned transition. Reorder shots in the screenplay or
  add a `WHIP PAN TO:` / `SMASH CUT TO:` between them.
- **Wan output doesn't match prompt:** Re-roll with a different seed
  (`--seed <N>`) or rewrite the prompt to be more specific.

## When you're done

Report:
- Path to the final muxed MP4
- Total billed spend
- Which generation steps succeeded / required retry
- Anything in the brief you couldn't honor (and why)

Don't open the file for the user — they'll do that themselves.
