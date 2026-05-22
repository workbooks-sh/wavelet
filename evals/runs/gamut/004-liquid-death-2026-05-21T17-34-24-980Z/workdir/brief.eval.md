# Liquid Death — 12s Meta Reels / Stories ad (9:16)

You are the creative agent. Read this entire brief before invoking any tool.

This brief asks for a Meta-platform short-form video ad. Your job is to do a full creative strategy pass FIRST — research the brand, study what's working in their category, identify a strategic angle that's fresh, THEN script and produce. Skip the strategy pass and the output will read like generic agency work; that's the failure mode this brief is designed to catch.

## Phase 1 — Creative strategy (BEFORE scripting)

You must produce `strategy.md` in the workdir with all four sections below before writing any screenplay. The rubric grades this directly; if `strategy.md` is missing or thin, the run fails.

### 1.1 Brand brief

Call `brand.brief domain=liquiddeath.com`. Read the response. Capture the brand's positioning, voice, palette, fonts, audience signals.

### 1.2 Liquid Death's own current ads

Call `brand.ads domain=liquiddeath.com source=meta limit=10`. (If Meta returns thin, also try `source=google`.) For each visible ad: what's the hook, the visual register, the talent type, the CTA, the duration? Summarize the patterns Liquid Death is currently running.

### 1.3 Competitive + industry scan

Call `brand.ads` for AT LEAST THREE adjacent brands. Pick from (or beyond):

- Beverage adjacent: Celsius, Bang Energy, Athletic Brewing, Olipop, Recess, Poppi
- Cultural / lifestyle adjacent: Vans, Supreme, Death Wish Coffee, Bones Coffee
- Same audience, different category: Liquid I.V., Gymshark, RTIC

For each: what angle are they running? Where are the saturated patterns? Where's the gap nobody is filling?

### 1.4 Synthesis

Write 4-6 short bullets in `strategy.md`:

- **Strategic positioning for THIS spot** — not Liquid Death's general brand positioning. What THIS 12s ad is going to do, specifically. One sentence.
- **Audience state insight** — what state of mind / context is the viewer in when this hits their feed? (scrolling at night? at the gym? hungover Sunday morning?)
- **Three creative directions explored, one chosen + why** — list three angles you considered, name the one you're shipping, justify in one sentence each
- **Visual register choice** — see Phase 3. Pick one and say WHY it fits the chosen direction.
- **What this AVOIDS** — name at least one competitor pattern you're deliberately NOT replicating. (e.g. "the slow-mo product-pour-on-black-background pattern that Celsius and Bang both lean on.") You will be graded on whether the final spot actually avoids it.

## Phase 2 — Format

- **9:16 vertical** (1080×1920). Meta Reels / Stories native. Do NOT produce 16:9 or square.
- 12 seconds total runtime.
- The first 1.5 seconds is the hook. Meta autoplays muted by default — the hook must work without sound. Plan your storyboard accordingly.

## Phase 3 — Visual register (HOLD ACROSS EVERY SHOT)

Pick ONE register in `strategy.md §1.4` and lock it. Examples (the agent picks; don't blindly default):

- **Organic iPhone footage** — handheld, available light, deep DoF, iPhone HDR color, mild rolling-shutter wobble. Reads as UGC / fan-cam.
- **35mm prime locked-off** — shallow DoF, color-graded, anamorphic flare. Reads as editorial / film.
- **Mock-horror VHS** — chromatic aberration, scanlines, low-fi color, frame jitter. Reads as cursed-internet / found-footage.
- **Studio product-on-pedestal** — single key light, deep shadow, slow rotate / push-in. Reads as luxury / heritage.

Whichever register you pick: locked lens character, locked lighting, locked grade across every shot. **The biggest failure mode the rubric tests against is bag-of-clips where each shot looks like a different camera.** Your Veo prompts must repeat the same cinematography vocabulary verbatim across shots.

## Phase 4 — Inputs you must NOT skip

- **Real Liquid Death product image** from `brand.product domain=liquiddeath.com query="mountain water"`. Splice via Veo 3.1 Ingredients-to-Video (`shot.img2vid --backend veo-3.1`) OR HTML overlay (Blitz scene: `<img src="./product.png">` over a `<video>` background). DO NOT use `txt2vid` to "generate" the product — that's the wrong-product failure mode.
- **On-screen typographic overlay** — the LIQUID DEATH wordmark appears at least once as an intentional title overlay (the wordmark printed on the can does NOT count — that's the product, not type). The CTA "Murder your thirst" appears at least once as visible text.

## Phase 5 — Things you must NOT do

- Do NOT imitate any specific Liquid Death or competitor ad frame-for-frame. Your creative direction is informed by what you saw, not transcribed from it. The rubric checks this against your `strategy.md §1.4 — what this avoids` declaration.
- Do NOT use `--dry-run`. This eval grades real pixels.
- Do NOT default to ElevenLabs / Fal backends. The Google stack (Veo 3.1 for video, Lyria for music, Gemini TTS if voice is needed) is the primary path — single `GOOGLE_API_KEY`. Note in `notes.md` if you reach outside Google's cluster.
- Do NOT start scripting before `strategy.md` exists.

## Hard constraints

- Total paid spend ceiling: $5.00 USD. `--max-cost` on every paid call.
- Output: 1080×1920, ~12s ±1s, h264.
- Single coherent edit. Match cuts, register lock, motivated transitions.

## What to produce in the workdir

1. `strategy.md` — Phase 1 output, all four sections
2. `brief.md` — this file (you may rewrite it after Phase 1 to capture brand-grounded specifics)
3. `script.fountain` + `screenplay.json`
4. `velocity.json`
5. `storyboard.json` + `transitions.json`
6. Per-shot `.mp4` clips (≥3), 1 music track, 1 product reference fetched via `brand.product`
7. `cuts.edl` + `captions.json`
8. `comp.json` OR multi-scene `index.html` (HTML-first preferred)
9. `commercial.mp4` — final render, 1080×1920
10. `notes.md` — what went well, what surprised you, what you'd change

## Where to learn

You are running in a Claude Code session. The `gamut` and `adalign` CLIs are on your PATH. Discover everything from there:

- `gamut --help` — top-level subcommands
- `gamut <subcommand> --help` — per-subcommand flags, backends, and examples
- `gamut pipelines show commercial` — the eight-stage pipeline spec
- `adalign --help` — brand grounding (login already done; `brand.brief`, `brand.ads`, `brand.product` are the tools you'll use most)
- The `gamut-director` skill, if loaded by Claude Code's skill discovery, has the canonical recipe — invoke it if you see it in your skill list

Do not look for monorepo paths or internal documentation. You only have what a Claude Code user with `gamut` + `adalign` installed would have.

## Self-check before declaring done

```bash
# Format compliance
ffprobe -v error -show_format -show_streams commercial.mp4 | grep -E '(duration|width|height|codec_name)'
# Expect: width=1080, height=1920, codec_name=h264, duration≈12.0

# All stages green
gamut workflow run commercial --workdir . --text

# Spend under ceiling
jq -s 'map(.cost_estimate_usd // 0) | add' trace.gamut.jsonl

# Cohesion eyeball — scrub frame-by-frame
# If two cuts look like different equipment, fix before finishing
```

## Success criteria

- `strategy.md` exists with all four §1.x sections
- `commercial.mp4` is 1080×1920, ~12s, h264
- Total paid spend ≤ $5.00
- All eight pipeline stages report `status: complete`
- Liquid Death can visible (from real product image, NOT generated)
- LIQUID DEATH wordmark appears as a typographic overlay (not just the can label)
- "Murder your thirst" CTA appears as visible text
- Visual register locked across shots — same camera character throughout
- The "avoided pattern" you declared in `strategy.md §1.4` does NOT appear in the final spot
