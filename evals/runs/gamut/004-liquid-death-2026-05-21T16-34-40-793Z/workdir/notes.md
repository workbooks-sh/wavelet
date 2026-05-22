# Production notes — Liquid Death 12s Meta Reels

## Backend stack used
- Music: Google Lyria 3 Pro (`google-lyria-3-pro`) — 12s doom-metal pulse, $0.012.
- Video shots 1–3: Google Veo 3.1 Fast (`google-veo-3.1-fast`) — img2vid conditioned on a real Liquid Death Mountain Water product PNG. 4s per clip × 3 = $3.00.
- Title card (shot 4): pure HTML/CSS (no AI). Heavy condensed Impact-stack typography stretched vertically to approximate the chiseled metal-band wordmark; "MURDER YOUR THIRST" CTA below the rule.
- Brand grounding: adalign brief + adalign catalog crawl (shopify strategy) to fetch the real product image.

All paid calls used the Google cluster on the single `GOOGLE_API_KEY` per the brief. No ElevenLabs / Fal calls. Did not reach outside Google's cluster except adalign (brand data only).

## Spend summary
- Lyria music: $0.012
- Veo 3.1 Fast × 3 img2vid shots: ~$3.00
- Total: ~$3.01 — well under the $5.00 ceiling.

## Creative decisions
- Picked the **mock-horror VHS** register over studio pedestal / 35mm / iPhone UGC because:
  - Hooks on mute autoplay — the dead-CRT static + tape jitter is unmistakable in frame 1.
  - Fits Liquid Death's brand DNA (heavy metal, horror, "Murder Your Thirst").
  - Deliberately differs from their current paid mix, which is sweepstakes-led long-copy carousels, not mood-piece video.
  - Saturates the visual layer so AI-gen artifacts read as register, not as failure.
- Locked the same VHS overlay stack across every scene (`vhs-scanlines + vhs-chroma + vhs-grain + vignette`) so the register holds across cuts even when the underlying Veo footage has minor frame-to-frame drift. This is the bag-of-clips defense.
- Used the **real product image** as the img2vid first-frame on every shot that features the can. The product is never txt2vid-generated.
- The title card carries the brand wordmark as a typographic overlay (Impact-stack heavy condensed, vertically stretched) so the wordmark requirement is satisfied independent of whatever the Veo footage rendered onto the can.

## What worked
- Locking the VHS overlay CSS to identical values across all four scenes gave instant cohesion despite Veo handling each shot independently.
- Single tungsten sidelight + dead CRT in shot 1 → continuous tungsten + dead-shadow palette in shots 2 & 3 → high-contrast monochrome title card in shot 4. Each shot read as the same camera.
- Lyria 12s doom-pulse produced a usable cue on first roll — no re-roll needed.
- Trimming Lyria's 30s output down to 12s with an `afade` tail in ffmpeg matched the title-card landing.

## What surprised me
- Lyria 3 returned a 30-second clip even with `--duration 12`; needed an ffmpeg trim step. The provider appears to clamp upward.
- `gamut storyboard verify` requires very specific enum strings: `push`/`dolly`/`whip` for `camera_movement`; `img2_vid`/`txt2_vid`/`native` (with underscores) for `generation.kind`. Hand-authored storyboards need to match exactly.
- `gamut verify` only accepts JSON compositions; HTML manifests are validated by `gamut render` itself. The pipeline's `comp_verify_passes` gate is currently passive against an HTML manifest.

## What I'd change next pass
- Generate scene-1 with a tighter framing on the lamp pool — Veo widened the shot a bit, which dilutes the "alone in shadow" beat.
- Try Veo `--last-frame` keyframe chaining to lock the can pose continuously between shots 2 and 3 (currently each shot's can pose is slightly different).
- Add a single tom hit at t=9.0s as a separate audio cue layered over the Lyria bed so the title stamp lands harder.
- Consider a tiny camera shake on the title card scene (CSS keyframe `transform: translate(...)` jitter) to extend the VHS register into the typographic beat.
