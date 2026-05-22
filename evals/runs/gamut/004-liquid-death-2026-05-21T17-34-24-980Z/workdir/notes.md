# Notes — Liquid Death 12s Meta Reels build

## Backend stack used (Google-cluster only, single GOOGLE_API_KEY)

- **Music**: Google Lyria 3 Clip (`google-lyria-3-clip`) — drone+tom bed seeded from velocity.json. $0.005. Lyria returned 30s; trimmed to 12s during render.
- **Video shots 1–3**: Google Veo 3.1 Fast (`google-veo-3.1-fast`), txt2vid, 9:16, 4s each. $1.00/clip × 3 = $3.00.
- **Scenes 4 + 5**: pure HTML/CSS (Blitz), no AI render. Real Mountain Water product PNG composited via `<img>` overlay.
- **Brand grounding**: `adalign brief liquiddeath.com` (+ Meta ads pull); `adalign catalog crawl liquiddeath.com` to find the real Mountain Water (Still) tallboy image.

Did **not** reach outside the Google cluster for paid generative calls.

## Spend
- Lyria 3 Clip: $0.005
- Veo 3.1 Fast × 3: $3.00
- **Total: $3.005** — well under the $5.00 ceiling.

## Creative decisions

**Visual register chosen: handheld iPhone fan-cam.** Locked vocabulary across every Veo prompt: *4500K candlelight, single tea-candle off-frame left, deep DoF, mild rolling shutter, iPhone HDR color, no color grade, matte black painted kitchen counter, deep shadow eating the frame edges*. Every shot inherits the same scrim / candle-warmth / grain CSS layer so the register holds across cuts.

**Why this register, not others** — Liquid Death's organic mythology is gothic and ritualistic but their paid Meta creative is loud comedy and Win-A-House sweepstakes; nobody in the cluster is running the brutal-quiet register the can art has been promising for years. Documented in `strategy.md §1.3-1.4`. I deliberately avoided:
  - The slow-mo can-floating-over-vibrant-gradient pattern (Celsius, Bang).
  - LD's own sweepstakes graphic-card layout.
  - The bright daylight creator-UGC tornado stunt.

**Real product image as the hero** — Veo's txt2vid output produced recognizable Liquid Death cans (the classic black skull SKU), but the brief asked for *Mountain Water* specifically, which is the white-with-gold-drip SKU. I treated the Veo plates as ambient candle-counter mood + hand motion, then layered the real `product.png` (sourced from `adalign catalog crawl`, file `Liquid_Death_19.2oz_Mountain_Still_Drinking_Water_Test.webp`) on top of every can-bearing scene via HTML `<img>` overlay with handheld-jitter, push-in, and lift-out CSS animations. The brief explicitly warns *"DO NOT use txt2vid to generate the product — that's the wrong-product failure mode"*, so the real PNG is the proof-of-product element, not the Veo render.

**Scene 5 — typographic overlay + CTA** — pure HTML black void with the "Liquid Death" wordmark (gothic font stack `UnifrakturCook, Pirata One, Times New Roman, serif`) and "MURDER YOUR THIRST" CTA in Acumin-Pro-fallback caps. Satisfies the rubric's *"wordmark appears as typographic overlay, not just the can label"* and *"Murder your thirst CTA appears as visible text"* requirements independent of whatever the Veo footage rendered onto the can label.

## What went well
- Single Lyria pass produced a usable drone+tom bed — no re-roll.
- Veo 3.1 Fast nailed the candle-on-counter mood + the LD brand visual cues on first roll for all three shots. Register-lock holds across the cuts.
- HTML overlay path means the *real* Mountain Water can is always the dominant on-screen product, regardless of what Veo drew.
- `gamut workflow run commercial` walked the eight-stage pipeline cleanly — research → script → velocity → storyboard → asset → edit → compose → publish, no stages skipped.

## What surprised me
- Veo's txt2vid was good enough at reproducing the LD wordmark + skull that it would have *looked* on-brand if I had ignored the brief's "wrong product" warning. The HTML overlay path adds friction but guarantees the *real* Mountain Water (Still) tallboy is on top.
- Veo 3.1 Fast `--duration` is clamped to [4, 8] seconds — defaults to 5 but errored at lower bound.
- Lyria 3 Clip ignored `--duration 12` and returned a 30-second track; ffmpeg trim handles the truncation at render.
- `gamut brief check` is strict about parsing — colons in the H1 line trip the slot parser, and the RUNTIME slot expects a positive integer (not "12s ±1s").
- `gamut verify commercial.html` errors because it only accepts JSON compositions; HTML manifests are validated by `gamut render` directly. Prior eval (16-34-40-793Z) noted the same.

## What I'd change next pass
- Render scenes 1–3 via Veo 3.1 (hero tier, not Fast) for a tighter grade — Fast can introduce minor wobble that fights the brutal-quiet register.
- Use Veo `--last-frame` keyframe chaining between shots 2 and 3 to preserve continuous hand-and-can pose across the cut.
- Replace the gothic font-family stack on scene 5 with a base64-embedded woff2 of UnifrakturCook to eliminate any fallback risk under Blitz.
- Add a single sub-bass tom-hit at t=9.6s as a separate audio cue ducked over the Lyria bed so the wordmark slam lands harder.
- Run `gamut image identity-check` against the Veo plates to flag any frames where Veo's drawn can disagrees too strongly with the overlaid real Mountain Water PNG (currently visually compatible but worth automating).
