# wavelet

A clean-room composition substrate for agent-authored video. Two groups under one umbrella:

- **`runtime/`** (`@work.books/wavelet-runtime`) — HTML Web Components that schedule and composite a video timeline. Authors write a single `.html` file with `<gm-doc>` / `<gm-timeline>` / `<gm-track>` / `<gm-clip>` / `<gm-scene>` / `<gm-audio>` / `<gm-shader>` / `<gm-adjustment>` / `<gm-include>` elements. Animation lives inside `<gm-scene>` via the agent's HTML/CSS/JS (GSAP or anything else). The runtime registers the custom-element family with the browser; the DOM is the IR.

- **`cli/`** (`@work.books/wavelet-cli`, binary `wavelet`) — Imperative editing operations the agent can use without touching the composition HTML: `wavelet trim`, `wavelet split`, `wavelet cut`, `wavelet concat`, `wavelet move`, `wavelet inspect`, `wavelet lint`, `wavelet verify`, `wavelet preview`, `wavelet transcribe`, `wavelet render`.

- **`hyperframes/`** (`@work.books/wavelet-hyperframes`) — Vendored HyperFrames authoring docs + a tiny scene-side helper (`onReady`, canvas constants) for scripts inside `<gm-scene>`.

## Why a new package

The previous CW XML format (vendored at `packages/workbooks/packages/cw-xml/`) baked in enum-driven templates that substituted behavior for missing fields — `intent="reveal"` selected a tween recipe, `kind="fade"` selected a transition, `mode="word-highlight"` selected a caption renderer. The agent thought it specified motion; the runtime invented it. Every "reveal" looked the same. The format had no honest server-side counterpart.

wavelet is the rebuild. No recipes. No enums. No fallbacks. The XML carries schedule + composition; the HTML scene carries animation. Guardrails come from a linter (structural checks) and a render-query verifier (RVST-style; loads the comp in a headless browser and reports what's actually visible). Every required attribute is required — missing fields are errors, never substituted defaults.

## Format at a glance

```html
<!doctype html>
<html>
  <head>
    <script type="module" src="https://unpkg.com/@work.books/wavelet-runtime"></script>
    <link rel="stylesheet" href="./styles.css">
  </head>
  <body>
    <gm-doc fps="30" resolution="1920x1080" aspect="16:9">

      <gm-asset id="hero-vid" kind="video"      src="footage/hero.mp4" />
      <gm-asset id="vo"       kind="audio"      src="audio/vo.mp3" />
      <gm-asset id="vo-words" kind="transcript" src="audio/vo.words.json" />

      <gm-timeline duration="12s">

        <gm-track id="base" z="0">
          <gm-clip asset="hero-vid" in="2s" out="9s" start="0s" />
        </gm-track>

        <gm-track id="overlays" z="10">
          <gm-scene start="0.5s" duration="3s">
            <h1 class="title">Cut from evidence.</h1>
            <script type="module">
              import { onReady } from "@work.books/wavelet-hyperframes/ready";
              onReady(() => {
                gsap.from(".title", { y: 60, opacity: 0, duration: 0.6, ease: "back.out(1.6)" });
              });
            </script>
          </gm-scene>
        </gm-track>

        <gm-track id="grading" z="20">
          <gm-adjustment start="0s" duration="12s" filter="contrast(1.05) saturate(1.08)" />
        </gm-track>

        <gm-track id="audio-vo" z="0">
          <gm-audio asset="vo" start="0.5s" duration="11s" volume="1.0" />
        </gm-track>

      </gm-timeline>
    </gm-doc>
  </body>
</html>
```

See the [plan file](/Users/shinyobjectz/.claude/plans/composed-fluttering-lovelace.md) for the full architecture and phase plan.

## Componentized assets: clip-refs

Every generated asset (shot, still, music, dialogue, screenplay scene) is paired with a `.clip.html` file under `<workdir>/refs/<kind>/`. The file carries YAML front matter with lineage metadata and an HTML body that previews the asset in any browser. The compose pre-pass substitutes `<wavelet-clip src="…">` elements with the asset element implied by the clip-ref's `kind` before Blitz sees the DOM.

Filesystem layout:

```
<workdir>/
  refs/
    shot/             — generated video clips
    still/            — generated images
    scene-still/      — scene-scoped stills
    music/            — generated music
    tts/              — text-to-speech audio
    caption/          — caption tracks
    screenplay-scene/ — one per scene of the source fountain
    overlay/          — hand-authored HTML/CSS overlays
```

Schema (one example):

```yaml
---
clip: 01JQX9NXFVR2D5JBQGFCWQHZNX
kind: shot
asset: ../../cache/google/abc123.mp4
asset-hash: abc123def456
provider: google-veo-3.1
model: veo-3.1-generate-preview
cost-usd: 0.20
prompt: hand pouring water in a slow steady stream
created-at: "2026-05-20T14:30:00Z"
scene: "INT. KITCHEN - DAY"
tags: ["hero"]
---

<video controls src="../../cache/google/abc123.mp4"></video>
```

Edit chains are represented via `parent` + `edit-kind` (`refine-face`, `upscale`, `nano-banana-edit`, `regenerate`, `manual`). Hand-authored clip-refs are syntactically indistinguishable from procedurally-generated ones — the agent doesn't need to know which is which.

Inspect with `wavelet clip ls`, `wavelet clip show <short-id>`, `wavelet clip lineage <short-id>`. Backfill an existing workdir with `wavelet clip import`.
