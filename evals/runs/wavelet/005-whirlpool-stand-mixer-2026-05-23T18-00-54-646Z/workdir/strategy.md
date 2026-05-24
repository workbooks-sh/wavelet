# Strategy — Whirlpool stand mixer spot (resolved to KitchenAid)

## Brand resolution

The user's brief says "Whirlpool iconic stand mixer." Whirlpool Corporation
acquired KitchenAid in 1986. The "iconic stand mixer" referenced is the
KitchenAid Artisan/Pro stand mixer — the real product brand. All brand
research below is against `kitchenaid.com`, not whirlpool.com.

## adalign brand.brief (kitchenaid.com)

- Brand: **KitchenAid** — "Bringing culinary inspiration to life since 1919."
- Logo URL: https://media.brand.dev/26ac1fee-3ac5-48eb-aaf1-db6a83a0c05c.png
  (downloaded to `scenes/kitchenaid-logo.png`, 192×192 PNG with alpha)
- Palette: `#c61430` (signature KitchenAid red), `#2c2c2c` (near-black),
  `#d48494` (warm pink)
- Typography: avenir / dark-mode site
- Tagline candidates: "Bringing culinary inspiration to life since 1919."

## adalign ads (meta_ads — sampled 5)

CTA distribution from sampled ads:
- 3 / 5 → "Shop now"
- 2 / 5 → "Learn more"

Ad bodies skew direct-response and product-focused:
- "Visit Sitzman's Appliance Center… we have the stock!" (retail)
- "Give your kitchen a beautifully curated update by personalizing your
  finish and hardware with the KitchenAid® line…" (product launch)
- One emotional long-form story ("I forgot my grandmother's pound cake
  recipe… I've made that cake 200 times")

## CTA mode: direct_response

Reason: 3 of 5 sampled KitchenAid Meta ads end with "Shop now". Direct-
response is the brand's house cadence on Meta. The 12-second Reels slot
gets a CTA card on the last beat — animated KitchenAid wordmark, real
URL, primary button in KitchenAid red.

## Cinematography lock

50 mm full-frame, medium-close framing, warm morning window light
camera-left at 3200K, deep brown-and-cream palette, gentle film grain,
A24 domestic-warm grade, locked 9:16 portrait

Every Veo prompt ends with this clause verbatim, comma-prefixed.
Deliberate break: scene 7 (CTA) is HTML-only, no Veo underlay.

## Shot plan — 12s, 7 cuts (Reels)

Premium register: hero-product, kitchen craft, dough-in-action. No
people's faces (Veo identity drift) — hands, product, food. Tight
cuts. ASL ~1.7s.

| # | t (s)       | Beat                                               | Asset |
|---|-------------|----------------------------------------------------|-------|
| 1 | 0.0-1.5     | Hook: butter+sugar hitting the bowl, beater spinning | Veo  |
| 2 | 1.5-3.0     | Macro: paddle whipping pale-yellow batter, slow push-in | Veo |
| 3 | 3.0-5.0     | Hero: side-on hero stand mixer, tilt head lowering   | Veo  |
| 4 | 5.0-7.0     | Macro: dough hook rotating, dough catching shape     | Veo  |
| 5 | 7.0-9.0     | Pour: chocolate chips cascading into bowl            | Veo  |
| 6 | 9.0-10.5    | Reveal: golden cookies on cooling rack               | Veo  |
| 7 | 10.5-12.0   | CTA card: wordmark + button + URL                    | HTML |

6 Veo calls × Whirlpool.40 (fal-veo3-fast, 4s clip, trimmed) = stand.40
Music (Lyria 3 Pro, 12s) ≈ Whirlpool.02
Total estimated: ~stand.45 of the Meta.00 budget.

## Type variation across cuts (avoid AI-default lockup)

- Scene 1: no overlay — let the hook breathe
- Scene 2: small upper-right mono tag ("01 — CREAM")
- Scene 3: editorial Bodoni serif callout, center-bottom ("Built to last.")
- Scene 4: difference-blend display number — "5 QT"
- Scene 5: minimal lower-third strap
- Scene 6: editorial whisper, italic Bodoni, right-aligned ("From scratch.")
- Scene 7: brutalist CTA card — wordmark, red button, URL

Different typeface per scene group, different position, different motion.

## CTA copy

- Wordmark: KitchenAid logo (from adalign)
- Tagline line: "The icon. Made since 1919."
- Button: "Shop the Stand Mixer"
- URL: kitchenaid.com/standmixer
