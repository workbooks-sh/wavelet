# __NAME__

A wavelet composition. Open `wavelet.html` in any modern browser, or run `wavelet preview wavelet.html` for a dev server with reloads.

## Structure

- `wavelet.html` — the composition: `<gm-doc>` with the timeline, tracks, and inline `<gm-scene>` content. The scaffold exercises every major element family: `<gm-asset>`, `<gm-clip>`, `<gm-scene>`, `<gm-audio>`, `<gm-adjustment>`, and four z-ordered `<gm-track>` layers.
- `styles.css` — visual identity (palette, typography). You own this — the runtime reads no design tokens.
- `assets/` — drop video, audio, image, and transcript files here. Reference them with `<gm-asset id="..." kind="..." src="./assets/...">`. If you had ffmpeg installed during `wavelet init`, this directory was pre-populated with two placeholders:
  - `demo.mp4` — 1920×1080 / 30fps / 6s calm dark gradient (gentle motion, no loud test pattern). Replace with real footage.
  - `vo-tone.mp3` — 6s of 440Hz sine. Replace with your narration (use `wavelet transcribe <audio>` to also emit a word-timing JSON for caption scenes).
- `scenes/` — external scene HTML files referenced via `<gm-scene src="scenes/...html" ...>`. Use this when a scene gets large; inline scenes inside `<gm-scene>...</gm-scene>` are equally valid.

## Iterate

- `wavelet inspect wavelet.html` — resolved timeline summary.
- `wavelet lint wavelet.html` — structural checks (missing fields, dangling refs, schedule overflow, sub-frame time precision, duplicate ids).
- `wavelet preview wavelet.html` — live preview.
- `wavelet verify wavelet.html` — headless-Chromium render-query: catches per-scene fade-out bugs, missing subject selectors, asset 404s.
- `wavelet render wavelet.html -o out.mp4` — render to MP4 (audio included).

Every animation lives in scene `<template>` HTML. The composition HTML carries schedule + composition only.
