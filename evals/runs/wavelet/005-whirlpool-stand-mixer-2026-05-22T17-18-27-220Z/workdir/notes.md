# Run notes — 005 Whirlpool stand mixer

## Why this is a pure-HTML/CSS spot

The eval environment exposes only `OPENROUTER_API_KEY`. There is no Fal,
no Pexels, no ElevenLabs, no Replicate, no Google AI credential. Every
paid leg of the `commercial` pipeline (`shot search`, `shot img2vid`,
`image scene-still`, `music gen`, `dialogue tts`) refuses without its
backend key. So the $5 budget couldn't have bought anything in this run
regardless — we ended at $0.

The previous attempt stalled on this same fact ("I don't want to render
a fabricated product"). Rather than wait, this run takes the pragmatic
path: build the spot from Wavelet's free render kernel (Blitz + Vello)
and lean on type, SVG silhouette, and light. The mixer is drawn from
primitives, not photographed and not generated.

## Brand framing

"Whirlpool iconic stand mixer" is a slightly mixed brief in the real
world — the iconic stand mixer is KitchenAid's, which Whirlpool owns
since 1986. The spot threads this by leading on the Whirlpool wordmark
+ heritage line ("EST. 1911") and showing the mixer as a Whirlpool
product, with one finish nodding to the KitchenAid family ("Empire Red"
is the famous KA colorway).

## Verify caveat

`wavelet verify commercial.html` errors with "expected value at line 1
column 1" — the verify subcommand still expects JSON in this build.
Verify-on-HTML is a known gap; the render path itself reads HTML fine.

## Render

`wavelet render commercial.html` (GPU Vello, 30fps) → 1.47 MB MP4 in
14.9s, 360 frames @ 1080×1920. The first attempt timed out (>300s with
no output) because every scene piled `filter: blur(28–30px)` on
1100–1500 px elements, `mix-blend-mode: overlay` grain layers, and
multiple SVG `drop-shadow` filters. Vello redraws those per frame.

Simplified pass drops the heavy filter stack and keeps the look through
gradients, vignette overlays, restrained typography, and cheap
transform-only animation. Same six scenes, same beats.

Cost this run: $0 / $5 (no paid backends available).
