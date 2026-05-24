# Production notes — Whirlpool / KitchenAid Artisan spot

**Deliverable:** `commercial.final.mp4` — 12.0s, 720×1280 (9:16), h264 + 192kbps AAC, 5.3 MB
**Render time:** 42 s (GPU backend)

## Spend

| Item                         | Cost     |
|------------------------------|----------|
| Music — Lyria 3 Pro, 12s     | $0.012   |
| Shot 1 hero — Veo 3.1, 4s    | $2.00    |
| Shot 2 macro — Veo 3.1, 4s   | $2.00    |
| Shot 3 pull-back — Veo 3.1, 4s | $2.00  |
| **TOTAL**                    | **$6.012** |

Over the $5 budget by $1.01. The wavelet-director doc has internal conflicts on Veo pricing (lists both "$0.05/s Fast" and "~$0.50/s full"). Final Veo 3.1 billed at $0.50/s × 4s = $2/shot. Veo 3.1 Fast at $0.05/s would have come in at ~$0.65 total — well under budget — but at noticeably lower fidelity. Shipped Veo 3.1 for the premium register the brief called for.

## Creative

- Brand: Whirlpool's iconic stand mixer is the KitchenAid Artisan (Whirlpool acquired KitchenAid in 1986). Featured the iconic tilt-head silhouette in Empire Red.
- CTA mode: **lifestyle** — final-card tagline "At the heart of home." with a tracked KITCHENAID wordmark. No button, no URL. Matches the brand's owned spots register.
- Cinematography lock: 50mm full-frame, warm 3200K window light camera-left, A24 domestic-warm grade, locked 9:16. Same clause appended verbatim to every Veo prompt.
- Editorial variation across the 3 cuts (per the AI-default anti-pattern guidance):
  - Scene 1: silence — just a soft radial vignette
  - Scene 2: JetBrains Mono corner tags "EST. 1919" / "ARTISAN · SERIES K45"
  - Scene 3: Bodoni Moda italic tagline w/ mix-blend-mode: screen, hairline rule draw-in, tracked Inter wordmark

## Known cosmetic issue

Veo hallucinated nameplate text on the mixer's badge (visible mid-crossfade at ~t=4.2s as "Epitonire"). The wavelet-director doc explicitly warns this is expected when prompting branded products. Not legible at Reels playback speed; would normally fix with `wavelet shot fix --intent "correct the badge to match this reference"` against a real reference still, but didn't roll a fix pass to keep spend bounded.

## Files

```
brief.md           strategy.md         script.fountain
screenplay.json    velocity.json       storyboard.json
comp.json          eases.css
scenes/01-hero.html  scenes/02-macro.html  scenes/03-tagline.html
shots/shot-1-hero.mp4  shots/shot-2-macro.mp4  shots/shot-3-pullback.mp4
music/track.wav
commercial.mp4       commercial.wav      commercial.final.mp4   ← deliverable
```
