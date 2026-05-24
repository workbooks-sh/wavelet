# Strategy — Whirlpool stand mixer (KitchenAid)

## Brand resolution

Whirlpool acquired KitchenAid in 1986. The "Whirlpool stand mixer" the
user means is the **KitchenAid Artisan stand mixer** — Whirlpool's
iconic mixer ships under the KitchenAid sub-brand. All creative
references KitchenAid as the product brand. The Whirlpool wordmark
does not appear on stand-mixer marketing.

## adalign findings (Phase 1 gate)

`adalign brief kitchenaid.com` returned:

- **Name:** KitchenAid
- **Logo URL:** `https://media.brand.dev/26ac1fee-3ac5-48eb-aaf1-db6a83a0c05c.png`
- **Palette:** `#c61430` (KitchenAid red), `#2c2c2c` (charcoal), `#d48494` (dusty rose)
- **Slogan:** "Bringing culinary inspiration to life since 1919."
- **Founded:** 1919

`adalign catalog crawl kitchenaid.com` returned 12 products (attachments
+ accessories — no Artisan stand mixer SKU exposed in the .com catalog;
the mixer itself is the silhouette every viewer already knows). Real
product imagery for the hero would require the kitchenaid.com PDP
which the catalog crawl didn't expose; Veo's prior on the Artisan
silhouette is strong enough to carry without a `--refs` image, and any
identity drift gets corrected with a `wavelet shot fix` pass.

`adalign brief` ads.meta (5 sampled, 30 total):

| # | Title | CTA | Link |
|---|-------|-----|------|
| 0 | Enjoy Memorial Day Savings | Shop now | homedepot.com |
| 1 | Suites Starting at $4219 | Shop now | homedepot.com |
| 2 | Shop Sitzman's Appliance Center | Shop now | sitzmansmaytag.com |
| 3 | Feel Like Yourself Again | Learn more | alevia.com (third-party) |
| 4 | The KitchenAid Line | Learn more | fergusonhome.com |

**CTA mode: direct_response** — 4 of 5 sampled KitchenAid Meta ads
close with a "Shop now" card and a retail link. The fifth ("Feel Like
Yourself Again") is a third-party advertiser unrelated to KitchenAid.
KitchenAid's house Meta cadence is unambiguously direct-response.

## Cinematography lock

50mm full-frame, medium-close framing, warm morning window light from
camera-left at 3200K, deep cream-and-charcoal palette with KitchenAid
red as the single saturated accent, gentle film grain, A24
domestic-warm grade, locked 9:16 portrait

Every Veo prompt ends with the above clause, character-for-character.

## Concept

A single hero kitchen moment with the KitchenAid Artisan in
KitchenAid red — quick tactile cuts (dough hitting the bowl, tilt-head
locking, dough hook turning, bowl rotating, finished loaf cooling) —
closing on a CSS-typeset CTA card with the real logo URL, the slogan
fragment "Since 1919," and a Shop now button on kitchenaid.com.

## Shot plan (12s, 7 cuts)

| # | t (s) | Beat | Veo dur |
|---|-------|------|---------|
| 1 | 0.0–1.4 | CU dough drops into the bowl (hook frame) | 4s, trim |
| 2 | 1.4–2.8 | Tilt-head locks down, latch click | 4s, trim |
| 3 | 2.8–4.8 | Macro: dough hook rotating, dough catching shape | 4s |
| 4 | 4.8–6.6 | Wide reveal: red Artisan on counter, morning light | 4s, trim |
| 5 | 6.6–8.4 | Hands shaping the proofed dough | 4s, trim |
| 6 | 8.4–10.0 | Finished loaf pulled from oven (steam) | 4s, trim |
| 7 | 10.0–12.0 | CTA card (HTML, no Veo): wordmark + slogan + button | n/a |

6 Veo Fast txt2vid calls @ ~Whirlpool.40 each = ~mixer.40. Music 12s on
Lyria 3 Pro ~ Whirlpool.012. Total budget ~ mixer.42 of $5 ceiling.

## Music

Cinematic warm piano + strings, slow build, no drums, gentle swell at
6s, sustain through CTA. ~70 BPM.

## CTA scene (HTML, NOT Veo)

`scenes/07-cta.html` carries:

- `<img src="https://media.brand.dev/26ac1fee-3ac5-48eb-aaf1-db6a83a0c05c.png">` (real KitchenAid logo)
- Headline: "SINCE 1919."
- CTA copy: "The icon. For your kitchen." (≤7 words, ≤40 chars)
- `<button class="primary">Shop now</button>` — real CSS button
- URL text: `kitchenaid.com`
- Background: cream `#faf5ee` with KitchenAid red `#c61430` accent

No Veo underlay on the CTA scene. `<section data-scene-href>` only,
no `data-video-bg`.

## Transitions

Hard cuts throughout. No crossfades. The pacing carries the editorial
through-line.

## Budget

Ceiling $5. Plan ~ mixer.50. Headroom for one re-roll if a hero shot
misses.
