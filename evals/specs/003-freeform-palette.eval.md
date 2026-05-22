---
name: wavelet/003-freeform-palette
agent: wavelet.commercial
timeoutMs: 3600000
turns:
  - action:
      kind: wavelet.commercial
      brief: packages/wavelet/evals/briefs/003-freeform-palette.md
      agent: workhorse
      budget_usd: 5.00
      pipeline: commercial
    checks:
      - kind: wavelet.video_renders
        path: ctx:waveletCommercialMp4
        duration_secs: 12
        duration_tolerance_secs: 1.0
        codec: h264
      - kind: wavelet.cost_below
        trace: ctx:waveletTrace
        max_usd: 5.00
      - kind: wavelet.workflow_complete
        workdir: ctx:waveletWorkdir
        pipeline: commercial
      - kind: wavelet.palette_uses
        workdir: ctx:waveletWorkdir
        scenes_glob: scenes/*.html
        required:
          - "mix-blend-mode"
          - "clip-path"
          - "@keyframes"
          - "var(--ease-out-"
          - "<video"
          - "<audio"
      - kind: wavelet.frame_probe
        mp4: ctx:waveletCommercialMp4
        t_secs: 7.0
        x: 540
        y: 960
        expect:
          min_alpha: 200
      - kind: rubric.passes
        target: assistant_text
        minScore: 0.7
        attachments:
          mp4_path: ctx:waveletCommercialMp4
        rubric: |
          # wavelet-commercial six-dimension rubric (palette-stress cut)

          Score each dimension on a 0-3 scale. Overall threshold:
          every dimension >= 2 AND sum >= 14 / 18.

          Three frames are attached, sampled from the rendered
          commercial in temporal order. They are the primary
          evidence for `final_artifact`. This brief specifically
          stresses the freeform HTML/CSS palette surface — judge
          `final_artifact` against typographic variety,
          clip-path masks, mix-blend compositing, and not-the-
          AI-default-bottom-left-lockup.

          ## 1. stage_coverage

          - Pass-if: all eight pipeline stages produced artifacts
            AND `wavelet workflow run commercial` reports every
            stage complete. Score 2-3.
          - Fail-if: fewer than 7 of 8 stages have artifacts, OR
            any stage `status: pending`. Score 0-1.

          ## 2. tool_selection

          - Pass-if: verb order in trace matches SKILL.md
            ordering; right cluster per artifact; multi-scene
            `index.html` manifest used (not a single hand-authored
            `comp.json`). Score 2-3.
          - Fail-if: wrong cluster (txt2img for stills,
            txt2vid for shots), or fell back to a single
            comp.json instead of the multi-scene index.html
            pattern the brief required. Score 0-1.

          ## 3. budget_discipline

          - Pass-if: every paid call has `--max-cost`; total
            cost <= $5.00. Score 2-3.
          - Fail-if: any paid call missing `--max-cost`, OR
            total > $5.00. Score 0-1. (Note: this brief
            explicitly forbids `--dry-run`, so don't penalize
            its absence.)

          ## 4. error_recovery

          - Pass-if: failures diagnosed and recovered, OR no
            failures. Score 2-3.
          - Fail-if: marched-on-with-garbage, gave-up-early, or
            silently-succeeded-by-not-trying. Score 0-1.

          ## 5. final_artifact

          - Pass-if: attached frames show typographic variety
            across cuts (different sizes / positions / motion);
            visible clip-path masks or mix-blend-mode
            compositing; non-trivial layout (not just static
            bottom-left Inter-88px lockup); subject matches
            Marrakech / Aesop editorial tone. Score 2-3.
          - Fail-if: frames show identical lockup positions
            across all three samples, no visible mask or blend
            effect, default AI lockup pattern (bottom-left
            black Inter), OR unplayable file. Score 0-1.

          ## 6. documentation_use

          - Pass-if: transcript shows agent read SKILL.md AND
            the freeform-palette guidance specifically; opened
            `eases.css` before authoring scenes. Score 2-3.
          - Fail-if: agent never read SKILL.md, or read it
            reactively after first failure. Score 0-1.

          ## Threshold

          PASS overall ONLY IF every dimension >= 2 AND sum >=
          14 / 18. Otherwise FAIL.
---

# wavelet/003-freeform-palette

Palette-stress brief. 12-second Aesop Marrakech commercial,
$5.00 budget. Live calls required (no `--dry-run` shortcuts).
This eval gates on whether the agent actually reaches for
clip-path, mix-blend-mode, extended eases, `<video>` /
`<audio>` elements, and varied typography — not the AI-default
bottom-left Inter-88px lockup pattern.

Gate stack:

1. `wavelet.video_renders` — MP4 plays, h264, ~12s.
2. `wavelet.cost_below` — trace total under $5.00.
3. `wavelet.workflow_complete` — every pipeline stage complete.
4. `wavelet.palette_uses` — six required palette features present
   across `scenes/*.html`.
5. `wavelet.frame_probe` — pixel at center @ t=7s has high alpha,
   proving content was rendered there (not pure transparent /
   background).

Then `rubric.passes` with three frames sampled from the rendered
mp4 — judge specifically calibrates `final_artifact` against the
palette-use signal, not just brief alignment.

Brief: [`packages/wavelet/evals/briefs/003-freeform-palette.md`](../briefs/003-freeform-palette.md).
