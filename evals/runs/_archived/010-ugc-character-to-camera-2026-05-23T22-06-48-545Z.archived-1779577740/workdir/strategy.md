# Dew Drop Serum — strategy

## Brand identity (designed inline; net-new brand, no brandwork resolution needed)

- **Name**: Dew Drop Serum (`dew drop` as the wordmark, lowercase sentence case)
- **Palette**:
  - cream `#FAF5EE` — base / panel
  - dew aqua `#B8DDE3` — accent
  - deep ink `#1F1F22` — body type
  - soft sage `#C8D4C0` — secondary accent
- **Wordmark type**: clean modern sans, low-tracked, slight italic on `drop`
- **Tagline**: "two drops to glass skin"

## Domain resolution

Skipped. "Dew Drop Serum" is a brand-new fictional brand per the user's
brief ("a new skincare brand"); `brandwork resolve` would surface a
random unrelated domain. Designed identity inline above.

## CTA mode

**direct_response** — reason: this is an explicit "recommending it to a
friend" UGC spot for a *new* brand. Discovery moment for the viewer;
they need a where-to-buy nudge. End on a 2-second HTML card with
wordmark + "shop dew drop" button + URL. Card is HTML/CSS only — no
Veo on the CTA.

## Cinematography lock (UGC register — NOT cinematic)

> handheld iPhone 16 Pro Max at chest height, soft window daylight from
> camera-left, slightly overexposed in highlights, Apple HDR camera-
> native color with no film grade, faint rolling-shutter wobble, no
> anamorphic flare, no film grain, 9:16 portrait, looks like a phone
> someone is actually holding

Every Veo prompt ends with `, ` + this preamble verbatim.

## Character lock

One woman, late twenties, "ALEX" in the screenplay. Reference image
generated via Nano-Banana, then forwarded to every Veo call via
`fal-veo3-ref`. A separate `hands` reference covers the dropper ECU
shot — face-conditioning leaks hand quality if reused.

- ALEX face ref: late-20s woman, light olive skin, glossy collarbone-
  length brunette hair, no makeup or minimal, freckles, wearing a
  cream ribbed tank, bathroom or window setting
- ALEX hands ref: same skin tone + manicure-free natural hands holding
  a small clear glass serum dropper bottle with a black pipette top

## Shot list (~18s, 5 cuts, hard cuts only)

| # | dur | type | content | dialogue (on-camera) |
|---|-----|------|---------|----------------------|
| 1 | 4s | face MCU at bathroom vanity | Alex turns to camera, half-laughs | "Okay I have to tell you about this." |
| 2 | 4s | face CU by window | Alex holds bottle up next to cheek | "Two drops, morning and night." |
| 3 | 4s | ECU on hands (no face) | hands tilt bottle, single drop forms on dropper tip | (no dialogue — ambient room tone) |
| 4 | 4s | face MCU back at vanity | Alex shrugs, smiles wide | "My skin has never looked like this." |
| 5 | 2s | HTML CTA card | wordmark + tagline + button + URL | n/a |

Total runtime: 4 + 4 + 4 + 4 + 2 = **18s** exactly.

## Transitions

All hard cuts. UGC pacing — every soft transition would betray the
"phone someone is holding" register. No `data-transition-in` attrs in
`commercial.html`.

## Overlay strategy

- **Cuts 1-4 (Veo)**: a small persistent `dew drop` wordmark bug in the
  top-left, 7% canvas height, cream-on-translucent. Sits as an HTML
  overlay scene composited over each Veo `data-video-bg`.
- **Cut 5 (CTA)**: HTML-only scene, no Veo. Full wordmark + tagline +
  button + URL on a cream background. Real `<button>` element, CSS-
  styled, brand-aqua fill.

No text rendered inside any Veo clip. Brand wordmark in cuts 1-4 is
strictly an HTML overlay, not baked into the footage. Veo's known text
fidelity issues (wb-lzat) are sidestepped entirely.

## Cost forecast

| Step | cost |
|------|------|
| ALEX face still (nano-banana-3) | $0.04 |
| ALEX hands still (nano-banana-3) | $0.04 |
| Shot 1 (fal-veo3-ref, 4s) | $1.00 |
| Shot 2 (fal-veo3-ref, 4s) | $1.00 |
| Shot 3 (fal-veo3-ref, 4s) | $1.00 |
| Shot 4 (fal-veo3-ref, 4s) | $1.00 |
| **subtotal** | **$4.08** |
| re-roll headroom | $1.50 |
| budget cap | $6.00 |

Music: none. The spot is dialogue-driven UGC — Veo 3 generates the
on-camera audio synced to the speaker. Room tone over the dropper
cutaway can be silence or extracted from an adjacent cut.
