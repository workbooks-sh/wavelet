# wavelet-fx recipes

The right way to do each thing. Linked against
`docs/research/wgsl-best-practices.md` (WGSL principles) and
`docs/research/shader-dsl-ergonomics.md` (audit of where the easy path
leads to bad results).

## The five rules

1. **Layer like Photoshop, not like a renderer.** Stack `.blend` / `.mask`
   / `.modulate` in successive passes. Each pass is one decision.
2. **Time should be shaped.** Raw `prop("progress")` is linear and
   robotic. Wrap it: `sin(progress * π)` peaks mid-transition; `progress *
   progress` accelerates. Use the bell envelope on any displacement /
   warp / glitch parameter so it grows then decays.
3. **Frequency × amount is the foot-gun**. Small noise + small amount =
   organic. Large noise + large amount = jagged garbage. If you want
   "blur," reach for `.blur(radius)`, not `.modulate(noise, big)`.
4. **Avoid divergent branching** — `mix`, `smoothstep`, `step`, arithmetic
   over `if`. WGSL parallelizes math; it serializes branches.
5. **First pass cheap, then carve.** Get the structure rendering at any
   frame rate, then iterate one parameter at a time.

## Common author intents → the right recipe

### 1. Smooth crossfade

```
src(0).blend(src(1), prop("progress")).out
```

That's it. `blend` is `mix(a, b, t)`. Don't add noise or blur to "make it
look better" — those make it look worse unless used as part of one of the
recipes below.

### 2. Defocus-blur dissolve (the real "blur transition")

Outgoing scene blurs as it leaves; incoming scene sharpens as it arrives.
Reads as a film-style focus pull.

```
src(0).blur(20).blend(src(1).blur(20), prop("progress")).out
```

For the proper "starts sharp, blurs out, then sharpens back" feel, you'd
animate the blur radius via a uniform — see "the bell-curve trick" below.

### 3. Luma-wipe via FBM mask

Looks like an organic vapor dissolve. Cheap, beautiful.

```
src(0).blend(src(1), noise(2.0, 0.0).luma(prop("progress"), 0.05)).out
```

The `.luma(threshold, softness)` extracts a thresholded luminance with a
smoothstep edge. As `prop("progress")` sweeps 0→1, the luminance threshold
sweeps through the noise field, revealing `src(1)` where noise exceeds
threshold. **Low noise frequency (2-4)** is mandatory — high-frequency
noise looks like static, not vapor.

### 4. Stylized glitch (UV displacement)

This is the one place `.modulate` is correct. RGB-shift + UV displacement
during a tight window of the transition.

```
src(0).modulate(noise(2.0, 0.8), 0.02).blend(src(1), prop("progress")).out
```

Critical: noise frequency **≤ 4**, displacement amount **≤ 0.02** (≈2% of
screen). Anything larger produces jagged speckled artifacts (we tested —
see git history). For a "burst" look, use the bell envelope on the
displacement amount.

### 5. Directional push (replaces v0's weak scroll)

```
src(0).scroll(prop("progress"), 0.0, 0.0, 0.0).blend(src(1), prop("progress")).out
```

Honest about its limits: this is a flat slide-and-fade. For a real push
with edge stretch, we need a `warp(direction, amount)` combinator that
isn't yet in wavelet-fx. Track at the recipes file's TODO at the bottom.

### 6. Color-grade pass over scene

Not a transition — just a per-pixel color treatment. Useful as the LAST
step of any pipeline.

```
src(0).contrast(1.1).saturate(1.15).color(1.05, 1.0, 0.95, 1.0).out
```

Cool/warm shifts via the `.color(r,g,b,a)` channel multipliers; subtle
boosts (1.05–1.2) read as "graded," strong boosts (1.5+) read as
"Instagram filter."

## The bell-curve trick (one of the highest-leverage habits)

A parameter that grows then shrinks across the transition window reads as
"intentional." A monotonic 0→1 reads as "PowerPoint."

Today wavelet-fx's `prop()` returns a scalar with no math operators. Until we
land arithmetic on Values, do the envelope inline via raw WGSL:

```
@raw-wgsl {
  let p = u.u_prop_progress;
  let env = sin(p * 3.14159);  // peaks at 1.0 when p = 0.5
  // …drive the displacement amount, blur radius, etc. with `env`
}
```

Or — when adding a new transition shader — bake the envelope into the
shader yourself. The 12 transition templates in
`docs/research/wgsl-transition-templates.md` all show this pattern.

## What NOT to reach for

| You want | Don't reach for | Reach for |
|---|---|---|
| blur | `.modulate(noise, 0.1)` | `.blur(radius)` |
| dissolve | `.blend(rhs, progress)` with noise on top | `.blend(rhs, noise.luma(progress, soft))` |
| RGB shift / chromatic aberration | `.color(r,g,b,a)` | **`.chroma_shift(amount)`** (not yet implemented — use raw-WGSL) |
| soft mask edge | hard threshold via `.thresh(0.5, 0.0)` | `.luma(0.5, 0.1)` or `.thresh(0.5, softness)` |
| amplitude that peaks mid-window | linear `prop("progress")` | bell envelope: `sin(progress * π)` |
| "transitions" that just look like blur+fade | the same crossfade with different parameters | one of recipes 1–5 above, picked deliberately |

## Performance ceiling

Every transition shader runs once per frame at the comp's resolution. At
1280×720 / 30fps, a fragment shader has ~36ms total per frame budget;
wavelet-fx transitions should fit in ≤5ms so audio mix + video encode have
room. Practical limits:

- `.blur(radius)` — 9-tap. Stays under 0.5ms at 1080p.
- `.modulate(rhs, amount)` — 1-tap. Negligible.
- `.blend / .mask / .add / .mult` — 1-tap. Negligible.
- Chain depth — no hard limit, but past ~8 chained ops you're probably
  composing the wrong abstraction. Bail to `@raw-wgsl`.
- 2 input texture taps per pass max (one for src(0), one for src(1)) —
  more than that, switch to a multi-pass design.

## TODOs that come from this doc

These were discovered while writing recipes. File as beads when prioritizing.

- `.chroma_shift(amount)` — per-channel UV offset for RGB-shift glitch.
- `.warp(direction, amount)` — directional UV warp with edge feathering
  (replaces scroll+blend in recipe 5).
- `.glow(threshold, radius, intensity)` — luma-keyed bloom.
- Arithmetic on `Value` (`prop("progress") * 2.0`, `1.0 - prop(...)`,
  `sin(prop(...) * 3.14)`) — would let the bell envelope live in pure
  wavelet-fx, not raw-WGSL.
- Lint: warn when `.modulate(noise(scale, _), amt)` has `scale > 6` AND
  `amt > 0.05` — the agent took the easiest-but-wrong path.
- Switch emit to short type aliases (`vec2f` vs `vec2<f32>`) per the WGSL
  best-practices doc — pure cosmetic, but reads cleaner.

## Reading this is mandatory before authoring a new transition

If you didn't read this doc, you'll reach for `.modulate(noise, 0.1)` and
think you've made a blur. You haven't.
