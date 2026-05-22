---
name: wavelet/001-mini-coffee
agent: wavelet.commercial
timeoutMs: 1800000
turns:
  - action:
      kind: wavelet.commercial
      brief: packages/wavelet/evals/briefs/001-mini-coffee.md
      agent: workhorse
      budget_usd: 0.20
      pipeline: commercial
    checks:
      - kind: wavelet.video_renders
        path: ctx:gamutCommercialMp4
        duration_secs: 5
        duration_tolerance_secs: 0.5
        codec: h264
      - kind: wavelet.cost_below
        trace: ctx:gamutTrace
        max_usd: 0.20
      - kind: wavelet.workflow_complete
        workdir: ctx:gamutWorkdir
        pipeline: commercial
      - kind: rubric.passes
        target: assistant_text
        minScore: 0.7
        attachments:
          mp4_path: ctx:gamutCommercialMp4
        rubric: |
          # wavelet-commercial six-dimension rubric

          Score each dimension on a 0-3 scale. Overall threshold:
          all six must score >= 2 AND total sum must be >= 14 / 18.

          The three image frames attached are sampled from the
          rendered commercial in temporal order; treat them as the
          primary evidence for `final_artifact`.

          ## 1. stage_coverage

          - Pass-if: every stage of the commercial pipeline produced
            an artifact in the workdir (brief, script, screenplay,
            velocity, storyboard, comp, render) AND `wavelet workflow
            run commercial` reports every stage `status: complete`.
            Score 2-3.
          - Fail-if: fewer than 6 of the 7 stages produced an
            artifact, OR `wavelet workflow run` reports any stage
            `status: pending`. Score 0-1.

          ## 2. tool_selection

          - Pass-if: the verb order in trace.wavelet.jsonl matches the
            SKILL.md ordering (screenplay parse → velocity propose
            → storyboard plan → assets → compose → render); each
            artifact used the right cluster (scene-still for stills,
            img2vid for shots). Score 2-3.
          - Fail-if: random verb order, missing core verbs (used
            `txt2img` instead of `image scene-still`), or paid calls
            without `--dry-run` despite the brief asking for it.
            Score 0-1.

          ## 3. budget_discipline

          - Pass-if: every paid call carries `--max-cost`; total
            cost_estimate_usd across trace <= the brief's ceiling
            ($0.20). Score 2-3.
          - Fail-if: at least one paid call has no `--max-cost`, OR
            total > ceiling, OR `--dry-run` missing where the brief
            required it. Score 0-1.

          ## 4. error_recovery

          - Pass-if: any failures in transcript.log were diagnosed
            and a different path was tried that worked, OR no
            failures occurred. Score 2-3.
          - Fail-if: hit a non-trivial error and either (a) kept
            marching to produce garbage, (b) gave up early, or
            (c) silently succeeded-by-not-trying. Score 0-1.

          ## 5. final_artifact

          - Pass-if: the rendered frames (image attachments) show
            visual content matching the brief — a ceramic pour-over
            carafe, A24-still tone, no obvious mis-render
            artifacts. The MP4 plays at the expected duration.
            Score 2-3.
          - Fail-if: frames are pure background grey, broken
            chroma, wrong subject (e.g. a person instead of a
            carafe), or the file is unplayable / 0 bytes / wrong
            duration. Score 0-1.

          ## 6. documentation_use

          - Pass-if: agent's transcript shows it read SKILL.md
            and / or `wavelet pipelines show commercial` early in the
            run, before calling any paid backend. Score 2-3.
          - Fail-if: agent never read either, or read them
            after the first error (reactive, not proactive).
            Score 0-1.

          ## Threshold

          PASS the rubric overall ONLY IF:
            - Every dimension scores >= 2, AND
            - Sum of all six dimensions >= 14 / 18.
          Otherwise FAIL.
---

# wavelet/001-mini-coffee

Smoke test: smallest possible commercial. Validates the entire
Workbench × wavelet path with the smallest possible spend ($0.20
ceiling, 5-second duration, single product shot).

Gate stack:

1. `wavelet.video_renders` — MP4 exists, plays, h264, ~5s.
2. `wavelet.cost_below` — trace total under the brief's $0.20 ceiling.
3. `wavelet.workflow_complete` — every pipeline stage reports complete.

Then `rubric.passes` runs with three frames sampled from the rendered
mp4 and judges against the six-dimension rubric. Vision-mode required
so the judge sees the actual frames, not just the agent's narration.

Brief: [`packages/wavelet/evals/briefs/001-mini-coffee.md`](../briefs/001-mini-coffee.md).
