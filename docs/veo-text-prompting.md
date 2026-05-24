# Veo Text-in-Generation Prompting

**Context:** When a director wants text baked into a clip — a wordmark forming
from paint, kinetic typography, a brand name built into the motion — Veo's text
fidelity is the weak link. This doc summarises what is known as of 2026-05.
Sources are labelled: (official) = Google-published; (community) = third-party
review, blog, or forum consensus; (unknown) = no clear provenance.

---

## 1. Font Naming: Genre Terms vs Specific Fonts

**No documented evidence** that Veo responds meaningfully to specific typeface
names ("Helvetica Black", "Druk", "Univers"). Official Google prompting guides
for Veo 3 / 3.1 make no mention of font family names as levers.

**Style descriptors are the documented path** (community):

- "clean, white, sans-serif font" — works (arsturn.com guide)
- "bold letterforms", "modern grotesque", "geometric sans" — plausible by
  analogy from Imagen 4 prompting, unverified on Veo
- "bold lettering", "floral lettering", "fantasy typography" — effective on
  Imagen 4 (official Imagen guidance); likely to transfer to Veo since Veo's
  image backbone is Imagen-lineage

**Practical recommendation:** Use genre descriptors, not font names. The
most reliable pattern is `"[weight] [category] font"` — e.g. `"bold white
sans-serif font"` or `"heavy condensed uppercase lettering"`. Specific names
are untested and unlikely to produce different results from genre equivalents.

---

## 2. Text Positioning

No official Veo documentation on text position. Community guidance:

- **Spatial language works inferentially**: "centered in frame", "upper-left
  corner", "lower third" — standard cinematographic framing applies. Veo
  generally respects compositional framing in prompts. (community, general
  Veo prompting guides)
- **Close-up framing improves legibility**: "tight shot on the text" or
  "close-up of a sign" reduces competing visual noise and forces Veo to
  resolve the letterforms with more detail budget. (community, arsturn.com)
- **Flat, head-on surfaces outperform curved ones**: "the word 'OPEN' on a
  flat wall" renders cleaner than text on rounded or moving surfaces.
  (community, arsturn.com)
- **Stable shots > camera movement**: motion blur degrades temporal
  consistency of letterforms across frames. (community)

---

## 3. Style Descriptors: Color, Weight, Animation

**Animation framing matters.** The most documented success pattern uses
image-to-video (i2v), not txt2vid, for motion-graphics text effects
(replicate.com blog). Two-frame interpolation — clean start frame → text
already rendered in end frame — lets Veo animate *between* a known text
state rather than hallucinate letterforms from scratch.

For pure txt2vid, the Replicate blog documents two working prompt patterns:

- **Ribbon reveal**: `"The text swirls in as cremé-colored ribbons, beautifully
  spelling out 'Build with Replicate'"` — Veo resolved the text, though fidelity
  varied across generations. (community, replicate.com)
- **Environmental emergence**: `"The words 'Run VEO 3' swirl dynamically out of
  the environment"` — abstract motion, not clean letterforms. (community,
  replicate.com)

"Bold white sans-serif text fading in from the bottom" is less prone to
hallucination than "kinetic typography of the word 'KitchenAid'" because the
former constrains style without triggering the subtitle-generation pathway
(community inference). Naming a brand without explicit text-render framing
tends to produce an abstract logo hallucination, not legible text.

---

## 4. Anti-Patterns and Known Failure Modes

| Failure | Notes | Source |
|---|---|---|
| Long strings | Beyond ~3-5 words, fidelity degrades significantly | community |
| Subtitles pathway | Including dialogue + text-like description triggers hallucinated captions | community (replicate.com) |
| Curved/moving surface | Letterforms distort on curved geometry or during camera motion | community |
| Numbers and mixed-case | Unreliable; numbers harder than all-caps; mixed case harder than uppercase | community (general AI image) |
| Italic | No Veo-specific data; italics historically harder for diffusion models | community inference |
| Complex logos | "Even with the best prompting, there will be times when the AI just can't nail the text" for complex marks | community |
| Veo 3 vs 3.1 | No improvement in text rendering between versions; same limitation in 3.1 | community (veo3ai.io) |

**Suppress unwanted text:** When you do NOT want baked text, use negative
prompt `"no text overlays, no subtitles, no on-screen text"`. Negatives
work well in Veo. (community, replicate.com)

---

## 5. Concrete Prompt Templates

For `wavelet shot txt2vid`, these templates are ordered by expected reliability
(most to least). All use the i2v / two-frame approach where noted.

**Template A — Plain sign or surface (highest reliability, txt2vid)**

```
Close-up shot of [material] surface showing the word "[TEXT]" in clean,
[color], bold [category] font, head-on framing, stable camera, studio light,
no motion blur.
```

Example:
```
Close-up of a matte white wall with the word "NEW BALANCE" in clean black bold
sans-serif font, head-on framing, flat studio lighting, no motion blur, no
subtitles.
```

**Template B — Environmental reveal (txt2vid, experimental)**

```
Cinematic shot: the words "[TEXT]" emerge [from/as] [material/substance],
[motion description], [cinematography preamble], no subtitles.
```

Example:
```
Cinematic: the word "BALANCE" forms letter-by-letter from black acrylic paint
dripping downward on a white canvas, slow motion, 50mm full-frame, studio
light, no subtitles, no watermark.
```

**Template C — i2v interpolation (highest control, recommended for wordmarks)**

Prepare two frames: a clean scene frame and a frame with the text already
rendered as a real image (CSS overlay screenshotted, or compositor output).
Feed them as start/end frames to `wavelet shot i2v`. Veo animates the
transition without having to hallucinate the letterforms.

```bash
wavelet shot i2v --first clean-scene.png --last wordmark-hold.png \
  --prompt "the word 'NEW BALANCE' assembles from black paint brushstrokes,
            cinematic, 50mm, warm studio light" \
  --duration 4
```

**Template D — Kinetic single word (txt2vid, short string only)**

```
Typography animation: the single word "[WORD]" in [weight] [category] type
[animation verb] [from/into] [visual metaphor], [cinematography preamble],
no subtitles.
```

Example:
```
Typography animation: the word "BALANCE" in heavy condensed sans-serif type
sweeps in from left to right like a brushstroke, black on white, slow motion,
macro lens, no subtitles.
```

**Template E — Ribbon / particle build (txt2vid, abstract, least reliable)**

```
The text "[TEXT]" forms from [particle/element], [color] [material] wisps
spelling each letter in sequence, [cinematography preamble], no subtitles.
```

---

## Summary

- Veo text fidelity is a **known, unresolved limitation** as of Veo 3.1.
- Genre style descriptors outperform specific font names (no evidence names
  help; style terms are the documented lever).
- Short strings (1-3 words), all-caps, high-contrast, stable shot, flat
  surface = best chance of legible txt2vid text.
- **i2v two-frame interpolation is the recommended path for wordmark-critical
  shots** — let Veo animate, not hallucinate.
- After generation, run `wavelet image ocr` on a frame to grade legibility
  before accepting the clip.

---

*Sources: [replicate.com Veo 3 image guide](https://replicate.com/blog/veo-3-image),
[replicate.com Veo 3 prompting](https://replicate.com/blog/using-and-prompting-veo-3),
[arsturn.com clean text guide](https://www.arsturn.com/blog/how-to-get-clean-text-in-veo-3-a-guide-to-fixing-ai-gibberish),
[Google DeepMind Veo prompt guide](https://deepmind.google/models/veo/prompt-guide/),
[Google Cloud Veo 3.1 prompting guide](https://cloud.google.com/blog/products/ai-machine-learning/ultimate-prompting-guide-for-veo-3-1),
[Kittl Imagen 4 text analysis](https://www.kittl.com/blogs/how-google-imagen-4-fixes-the-text-problem-in-ai-art-ais/),
[veo3ai.io 3.1 update notes](https://www.veo3ai.io/blog/veo-3-1-new-features-update-2026),
[DreamHost Veo 3.1 guide](https://www.dreamhost.com/blog/veo-3-1-prompt-guide/)*
