# @work.books/wavelet-hyperframes

Authoring SDK for HyperFrames scenes inside a wavelet composition.

A "HyperFrames scene" is the visual content of a single `<gm-scene>` element. You write HTML, CSS, and JS by hand — including any animation library (GSAP, anime.js, CSS Web Animations, raw WebGL/Three.js). The wavelet runtime gives you a fixed 1920×1080 canvas, fires `hf:ready` when your scene mounts, fires `hf:tick` every frame, and tears the scene down when its time ends. Everything else is the agent's call.

No recipe library. No `intent="reveal"`. No `kind="fade"`. The runtime renders exactly what you write.

## Install

This package is a workspace dependency in the wavelet umbrella. Authors writing scenes inside a wavelet composition import from here:

```html
<gm-scene id="title" start="0.5s" duration="3s">
  <h1 class="title">Cut from evidence.</h1>
  <script type="module">
    import { onReady } from "@work.books/wavelet-hyperframes/ready";
    import { CANVAS_WIDTH, CANVAS_HEIGHT } from "@work.books/wavelet-hyperframes/canvas";

    onReady("title", () => {
      gsap.from(".title", { y: 60, opacity: 0, duration: 0.6, ease: "back.out(1.6)" });
    });
  </script>
</gm-scene>
```

## Public API

### `onReady(sceneId?, callback)`

Fires once when the scene with the given `id` mounts. Omit `sceneId` to listen for every scene's mount.

```ts
import { onReady } from "@work.books/wavelet-hyperframes/ready";

onReady("hero", ({ fps, startMs, durationMs }, target) => {
  // target is the scene container element
  // fps is the document's fps
  // startMs / durationMs are the scene's window in milliseconds
});
```

Returns an unsubscribe function.

### `onTick(sceneId?, callback)`

Fires every rAF tick while the scene is active. Use for scrub-safe motion that needs explicit playhead awareness (most authors don't need this — GSAP timelines drive themselves).

```ts
import { onTick } from "@work.books/wavelet-hyperframes/ready";

onTick("captions", ({ frame, fps, durationFrames }, target) => {
  // local frame within the scene; advance manually if you need scrub-perfect rewind
});
```

### `CANVAS_WIDTH` / `CANVAS_HEIGHT` / `CANVAS_ASPECT`

Constants for the design canvas size (1920 / 1080 / "1920 / 1080"). Use these when computing layout coordinates programmatically.

### `px(fraction, axis)`

Project a 0..1 fraction of the canvas to absolute pixels along an axis:

```ts
import { px } from "@work.books/wavelet-hyperframes/canvas";
const heroX = px(0.5, "x"); // → 960
const headlineY = px(0.4, "y"); // → 432
```

## Authoring docs

The `docs/` directory contains the full authoring guide. Read these before writing scenes:

- [SKILL.md](./docs/SKILL.md) — the canonical entry point: visual identity gate, layout-before-animation, motion principles.
- [house-style.md](./docs/house-style.md) — motion defaults (eases, durations, entrance patterns) for when no specific style is requested.
- [patterns.md](./docs/patterns.md) — composition patterns (hero + subtitle, multi-column grids, card flips, etc.).
- [visual-styles.md](./docs/visual-styles.md) — 8 named visual styles (Swiss Pulse, Velvet Standard, Deconstructed, Maximalist Type, Data Drift, Soft Signal, Folk Frequency, Shadow Cut) as REFERENCE — each is a complete thumbnail (palette, typography, motion signature) the agent can read for inspiration. Not enforced by code; never substituted in.
- [data-in-motion.md](./docs/data-in-motion.md) — data-driven motion patterns.
- [references/](./docs/references/) — deeper references: captions, TTS, audio-reactive, CSS patterns, transitions, typography, motion principles, transcript guide.
- [palettes/](./docs/palettes/) — palette references by mood.

## Relationship to wavelet-runtime

This package is the *authoring surface* — what scenes import. `wavelet-runtime` is the *registry* that runs in the browser and registers the `<gm-*>` custom elements. The two are decoupled so the runtime can ship as a single `<script>` tag and the authoring SDK can be tree-shaken into the scene bundle separately.
