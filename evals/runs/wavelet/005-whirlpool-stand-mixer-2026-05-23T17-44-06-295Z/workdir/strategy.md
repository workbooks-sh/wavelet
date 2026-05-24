# Strategy — Whirlpool / KitchenAid Stand Mixer, 12s Meta Reels

## Brand resolution (Phase 1 gate)

User brief: "Whirlpool ad for their iconic stand mixer."

Whirlpool Corporation acquired Hobart Manufacturing's KitchenAid line
in **1986**. The "iconic stand mixer" universally refers to the
**KitchenAid Artisan tilt-head stand mixer** (KSM150PS family), which
ships under the **KitchenAid** wordmark with "A WHIRLPOOL BRAND"
attribution. The hero in this spot is therefore the KitchenAid
wordmark, with Whirlpool corporate attribution as a small co-brand
line — matching how the product is actually packaged and merchandised.

### adalign brief kitchenaid.com — results (recorded verbatim)

```
name:    KitchenAid
logo:    https://media.brand.dev/26ac1fee-3ac5-48eb-aaf1-db6a83a0c05c.png
palette: ["#c61430", "#2c2c2c", "#d48494"]
slogan:  Bringing culinary inspiration to life since 1919.
fonts:   ["avenir"]   → close substitute on the open web: "Nunito Sans" or system Avenir
mode:    dark
```

Logo fetched to `assets/kitchenaid-logo.png` (192×192 PNG, transparent
background, signature red Hobart-shape mark).

### adalign brief — ads sample (Meta library)

5 ads sampled from the KitchenAid / KitchenAid-retailer Meta feed:

| # | Advertiser            | CTA        | Topic                          |
|---|-----------------------|------------|--------------------------------|
| 0 | The Home Depot        | Shop now   | Memorial Day Savings           |
| 1 | The Home Depot        | Shop now   | Suites $4219+                  |
| 2 | Sitzman's Appliance   | Shop now   | Local retail                   |
| 3 | (off-brand wellness)  | Learn more | (irrelevant — drop)            |
| 4 | Ferguson Home         | Learn more | "The KitchenAid® Line" finishes|

**4 of 5 relevant ads use a direct-response "Shop now" pattern.**

### CTA mode

**direct_response** — reason: 4 of 5 sampled KitchenAid-merchandised
Meta ads end with "Shop now" / "Learn more" against a shoppable SKU.
The final 1.5-2s holds an HTML CTA card with the real KitchenAid
wordmark, a red CSS button, and a URL line.

### Cinematography lock

> 50 mm full-frame macro, medium-close framing, warm morning window
> light camera-left at 3200K, deep cream-and-brown palette with a
> single KitchenAid-red accent, gentle 35mm film grain, A24 domestic-
> warm grade, locked 9:16 portrait

Every Veo prompt ends with this clause, comma-prefixed, character-for-
character. No paraphrasing.

The single deviation: shot 7 (the CTA scene) is an HTML composition
with no Veo underlay — it's a clean wordmark + button card on a flat
red field, per the direct-response CTA rule.

## Shot list (7 cuts / 12 s — breaks the 3-clip default)

| # | t (s)      | dur  | Beat                                                         |
|---|------------|------|--------------------------------------------------------------|
| 1 | 0.0 - 1.2  | 1.2  | Scroll-stop hook: bowl impact — flour bloom against red mixer|
| 2 | 1.2 - 2.5  | 1.3  | Hands tilting the head down, click sound (visual only)       |
| 3 | 2.5 - 4.5  | 2.0  | Macro: dough hook rotating, dough catching shape (rest beat) |
| 4 | 4.5 - 6.5  | 2.0  | Beater whipping cream into stiff peaks, slow rotation        |
| 5 | 6.5 - 8.5  | 2.0  | Sourdough loaf pulled from oven (proof of payoff)            |
| 6 | 8.5 - 10.0 | 1.5  | Wide: finished bread on a wooden table, KitchenAid in soft bg|
| 7 |10.0 - 12.0 | 2.0  | HTML CTA card — wordmark + button + URL on KitchenAid red    |

Veo strategy: 6 txt2vid calls × 4s @ Veo 3.1 Fast (~$0.20/call) = ~$1.20.
Each clip will be trimmed in compose to the table's `dur` value. Scene 7
is HTML-only, zero Veo cost.

## Type / motion variation per scene (anti-AI-default)

| # | Type treatment                                      | Animation                          |
|---|-----------------------------------------------------|------------------------------------|
| 1 | None — pure image, scroll-stop                      | n/a                                |
| 2 | Tiny corner timestamp, JetBrains Mono 14px BR       | flicker step                       |
| 3 | Editorial pull-quote, Bodoni-style 7vw lower-left   | drift-up on ease-out-quint         |
| 4 | None — let the macro breathe                        | n/a                                |
| 5 | Word stamp "Made yours", display 18vw mix-blend     | settle + difference blend          |
| 6 | None — silent beat into CTA                         | n/a                                |
| 7 | CTA card: wordmark + "Shop the icon" + URL          | wordmark fade + button rise        |

Three scenes are type-free on purpose. Two scenes use heroic display.
No two adjacent scenes share a lockup. Extended ease curves
(`--ease-out-quint`, `--ease-out-back`) used on scenes 3 and 7.
`mix-blend-mode: difference` used on scene 5.

## Music

Lyria 3 (clip): warm-domestic, gentle piano + light strings,
slow-build that lifts on the bread reveal (scene 5). ~12 s, $0.005.

## Budget (target $5.00 ceiling)

| Step                    | Cost     |
|-------------------------|----------|
| adalign brief (already) | $0.00    |
| Music (Lyria clip 12s)  | $0.01    |
| 6 × Veo 3.1 Fast 4s     | $1.20    |
| Re-roll buffer (≤2)     | $0.40    |
| Final render + lint     | $0.00    |
| **Total target**        | **$1.61**|

Leaves ~$3.39 headroom for retries / variants on hero shots.
