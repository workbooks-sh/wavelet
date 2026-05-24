# wavelet

**A composition substrate for agent-authored video.** Write a video
timeline as plain HTML. The browser is the renderer; the DOM is the
intermediate representation.

Most "AI video" tools either generate finished MP4s from a prompt (no
hand-off, no edits) or hide behind a proprietary timeline format
(opaque, untoolable). wavelet does neither: an agent writes a single
`.html` file with composition elements, a scene-side script does the
animation in regular HTML/CSS/JS (GSAP or anything else), and a
headless browser composites the result. The IR is inspectable, the
animation is debuggable, and every operation on the timeline is a CLI
command.

## What ships

- **`runtime/`** — `@work.books/wavelet-runtime`. HTML Web Components
  that schedule and composite a video timeline. Authors write
  `<gm-doc>` / `<gm-timeline>` / `<gm-track>` / `<gm-clip>` /
  `<gm-scene>` / `<gm-audio>` / `<gm-shader>` / `<gm-adjustment>` /
  `<gm-include>`. Animation lives inside `<gm-scene>` via regular
  HTML/CSS/JS — GSAP, vanilla, whatever the agent picks.
- **`cli/`** — `@work.books/wavelet-cli`, binary `wavelet`. Imperative
  edits the agent can run without rewriting the composition HTML:
  `wavelet trim`, `split`, `cut`, `concat`, `move`, `inspect`, `lint`,
  `verify`, `preview`, `transcribe`, `render`.
- **`hyperframes/`** — `@work.books/wavelet-hyperframes`. Authoring
  docs + a tiny scene-side helper (`onReady`, canvas constants) for
  scripts inside `<gm-scene>`.

## Design principles

- **No recipes, no enums, no fallbacks.** Every required attribute is
  required — missing fields are errors, never substituted defaults.
  The XML carries schedule and composition; the HTML scene carries
  animation. An agent that thinks it specified motion actually did,
  because nothing was invented behind its back.
- **Guardrails come from observation, not constraint.** A linter does
  structural checks. A render-query verifier loads the comp in a
  headless browser and reports what's actually visible — so the agent
  can self-correct against ground truth.
- **The composition is the source.** No build step turns the HTML
  into a different IR. What you write is what the runtime sees.

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

## Componentized assets — clip-refs

Every generated asset (shot, still, music, dialogue, screenplay scene)
is paired with a `.clip.html` file under `<workdir>/refs/<kind>/`.
The file carries YAML front matter with lineage metadata and an HTML
body that previews the asset in any browser. The compose pre-pass
substitutes `<wavelet-clip src="…">` with the asset element implied
by the clip-ref's `kind` before the renderer sees the DOM.

```
<workdir>/
  refs/
    shot/             generated video clips
    still/            generated images
    scene-still/      scene-scoped stills
    music/            generated music
    tts/              text-to-speech audio
    caption/          caption tracks
    screenplay-scene/ one per scene of the source fountain
    overlay/          hand-authored HTML/CSS overlays
```

Example:

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

Edit chains are represented via `parent` + `edit-kind` (`refine-face`,
`upscale`, `nano-banana-edit`, `regenerate`, `manual`). Hand-authored
clip-refs are syntactically indistinguishable from procedurally
generated ones — the agent doesn't need to know which is which.

Inspect with `wavelet clip ls`, `wavelet clip show <short-id>`,
`wavelet clip lineage <short-id>`. Backfill an existing workdir with
`wavelet clip import`.
