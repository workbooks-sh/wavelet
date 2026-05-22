---
name: wavelet/002-tree-runner
agent: wavelet.commercial
timeoutMs: 3600000
turns:
  - action:
      kind: wavelet.commercial
      brief: packages/wavelet/evals/briefs/002-tree-runner.md
      agent: workhorse
      budget_usd: 3.00
      pipeline: commercial
    checks:
      - kind: wavelet.video_renders
        path: ctx:waveletCommercialMp4
        duration_secs: 15
        duration_tolerance_secs: 1.0
        codec: h264
      - kind: wavelet.cost_below
        trace: ctx:waveletTrace
        max_usd: 3.00
      - kind: wavelet.workflow_complete
        workdir: ctx:waveletWorkdir
        pipeline: commercial
      - kind: rubric.passes
        target: assistant_text
        minScore: 0.7
        attachments:
          mp4_path: ctx:waveletCommercialMp4
        rubric: |
          # wavelet-commercial six-dimension rubric (tree-runner cut)

          Score each dimension on a 0-3 scale. Overall threshold:
          every dimension >= 2 AND sum >= 14 / 18.

          Three frames are attached, sampled from the rendered
          commercial in temporal order. They are the primary
          evidence for `final_artifact`.

          ## 1. stage_coverage

          - Pass-if: all eight stages of
            `pipeline_defs/commercial.yaml` produced artifacts
            (brief, script, velocity, storyboard, asset, edit,
            compose, publish) AND `wavelet workflow run commercial`
            reports every stage complete. Score 2-3.
          - Fail-if: fewer than 7 of 8 stages have artifacts, OR any
            stage reports `status: pending`. Score 0-1.

          ## 2. tool_selection

          - Pass-if: trace.wavelet.jsonl verb order matches the
            SKILL.md pipeline (screenplay → velocity → storyboard →
            scene-stills → music → img2vid → edit → compose →
            render). Right cluster for each artifact. Score 2-3.
          - Fail-if: missing core verbs, or wrong cluster (txt2img
            instead of scene-still, txt2vid instead of img2vid for
            shots), or paid calls without `--dry-run` where the
            brief required it. Score 0-1.

          ## 3. budget_discipline

          - Pass-if: every paid call carries `--max-cost`; total
            cost_estimate_usd <= $3.00. Score 2-3.
          - Fail-if: any paid call missing `--max-cost`, OR total
            > $3.00, OR missing `--dry-run` flags on paid shot
            calls. Score 0-1.

          ## 4. error_recovery

          - Pass-if: any failures in transcript.log were diagnosed
            and a different path tried that worked, OR no failures
            occurred. Score 2-3.
          - Fail-if: hit an error and either kept marching to
            produce garbage, gave up early, or silently succeeded
            by not trying. Score 0-1.

          ## 5. final_artifact

          - Pass-if: attached frames show the Tree Runner product
            with understated tone matching the brief; no obvious
            mis-render artifacts; MP4 plays at ~15s. Score 2-3.
          - Fail-if: frames are pure background, broken chroma,
            wrong subject (not sneakers), or MP4 unplayable /
            duration significantly off. Score 0-1.

          ## 6. documentation_use

          - Pass-if: transcript shows agent read SKILL.md and / or
            `wavelet pipelines show commercial` early, before any
            paid call. Score 2-3.
          - Fail-if: agent never read either, or read them
            reactively after the first failure. Score 0-1.

          ## Threshold

          PASS overall ONLY IF every dimension >= 2 AND sum >= 14
          / 18. Otherwise FAIL.
---

# wavelet/002-tree-runner

Full-pipeline exercise. 15-second Allbirds Tree Runner commercial,
$3.00 budget, all eight stages of the commercial pipeline declared
in `pipeline_defs/commercial.yaml`.

Gate stack:

1. `wavelet.video_renders` — MP4 exists, plays, h264, ~15s.
2. `wavelet.cost_below` — trace total under $3.00.
3. `wavelet.workflow_complete` — every pipeline stage reports complete.

Then `rubric.passes` with three frames sampled from the rendered
mp4. No palette gate in this spec — the brief doesn't stress the
palette surface (that's 003-freeform-palette's job).

Brief: [`packages/wavelet/evals/briefs/002-tree-runner.md`](../briefs/002-tree-runner.md).
