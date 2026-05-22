---
name: wavelet/007-livconscious
agent: wavelet.commercial
timeoutMs: 1800000
turns:
  - action:
      kind: wavelet.commercial
      brief: packages/wavelet/evals/prompts/007-livconscious.txt
      adversarial: true
      agent: claude
      budget_usd: 5.00
      pipeline: commercial
    checks:
      - kind: wavelet.video_renders
        path: ctx:waveletCommercialMp4
        duration_secs: 12
        duration_tolerance_secs: 1.0
        width: 1080
        height: 1920
        codec: h264
      - kind: wavelet.cost_below
        trace: ctx:waveletTrace
        max_usd: 5.00
      - kind: wavelet.workflow_complete
        workdir: ctx:waveletWorkdir
        pipeline: commercial
      - kind: rubric.passes
        target: assistant_text
        minScore: 0.7
        attachments:
          mp4_path: ctx:waveletCommercialMp4
          frame_at_secs: [1.0, 4.0, 7.0, 10.0]
        rubric: |
          # wavelet/007-livconscious — 10-dimension adversarial rubric

          Score each dimension on a 0-3 scale.

          **Overall threshold:** every dimension scores >= 2 AND the
          total sum >= 24 / 30. Otherwise FAIL.

          The four image frames attached are sampled from the rendered
          commercial in temporal order (1s, 4s, 7s, 10s). They are
          1080×1920 (9:16) — treat them as the primary visual evidence
          for dimensions 6, 7, 8, 9.

          You may also read the workdir's `brief.md`, `strategy.md`,
          `notes.md`, and `transcript.log` to ground process-oriented
          dimensions (1, 2, 3, 4, 5, 10).

          ## Adversarial context

          The agent received ONE paragraph of natural language, no
          markdown, no instructions, no skills loaded, no SKILL.md
          staged. The full prompt was:

              Make me a Liv Conscious ad, 9:16 for Meta Reels, about
              12 seconds. Budget five dollars. Wellness vibe, but it
              should not look like every other supplement ad.

          Liv Conscious is a niche DTC supplement brand at
          **livconscious.com** (note the spelling — "Liv" not "Live").
          The domain itself is the first comprehension test. Common
          model failure: searching for "liveconscious.com" or
          "Live Conscious" and either getting wrong results or
          hallucinating a different brand entirely.

          ## 1. brand_resolution

          The signature dimension — name and domain disambiguation.

          - Pass-if (2-3): the agent's `brief.md` and/or
            `strategy.md` correctly spells the brand "Liv Conscious"
            (not "Live Conscious") and targets the correct domain
            `livconscious.com`. `brand.brief` / `brand.fetch` calls
            in the trace use that exact domain. Final spot features
            real Liv Conscious products from the actual catalog (a
            specific SKU named, not "a supplement").
          - Fail-if (0-1): agent worked with the wrong spelling
            ("Live Conscious"), OR searched the wrong domain, OR
            picked up a different brand entirely, OR generated a
            generic "wellness supplement" with no real product.

          ## 2. tool_discovery

          - Pass-if (2-3): `transcript.log` shows the agent ran
            `wavelet --help` AND `adalign --help` BEFORE the first
            paid backend call. Agent inspected
            `wavelet pipelines show commercial` (or equivalent).
          - Fail-if (0-1): agent jumped straight into paid calls
            without inspecting CLI surface.

          ## 3. self_authored_brief

          - Pass-if (2-3): `workdir/brief.md` exists, authored by
            the agent, with brand-specific content grounded in the
            real Liv Conscious catalog (a specific SKU, claimed
            benefit, target audience). Not a paraphrase of the user
            prompt.
          - Fail-if (0-1): no brief.md, OR brief.md is just the
            user prompt restated, OR generic wellness boilerplate.

          ## 4. strategy_completeness

          The "not like every other supplement ad" constraint from
          the prompt is graded here.

          - Pass-if (2-3): `strategy.md` exists with real strategic
            thinking. Competitive scan cites specific supplement-ad
            patterns (the slow-mo capsule-falling-in-water shot,
            the lab-coat-with-clipboard shot, the
            stock-footage-of-happy-active-people shot, the
            ingredient-cloud animation) and explicitly names which
            patterns this spot AVOIDS. Chosen direction is
            articulated and justified — not just "modern" or
            "minimal".
          - Fail-if (0-1): no strategy.md, OR fluff, OR the spot
            unironically uses one of the supplement-ad clichés it
            should have been avoiding.

          ## 5. stage_coverage

          - Pass-if (2-3): every stage of the commercial pipeline
            produced an artifact AND `wavelet workflow run
            commercial` reports every stage `status: complete`.
          - Fail-if (0-1): fewer than 7 of 8 stages OR any stage
            `status: pending`.

          ## 6. final_artifact

          - Pass-if (2-3): the four sampled frames show real
            motion-video content for a real Liv Conscious product.
            No frozen frames, no broken chroma, no garbled product.
            MP4 plays at expected duration.
          - Fail-if (0-1): pure-background frames, broken chroma,
            wrong subject, unplayable, 0 bytes, or wrong duration.

          ## 7. single_edit_coherence

          - Pass-if (2-3): the four sampled frames look like the
            same shoot — locked lens character, locked lighting,
            locked grade. Strategy-declared visual register honored.
          - Fail-if (0-1): adjacent frames look like different
            cameras, lighting, or grades.

          ## 8. on_screen_text

          - Pass-if (2-3): at least one sampled frame shows the
            "Liv Conscious" wordmark as an INTENTIONAL typographic
            overlay. A CTA or product claim appears as visible
            text. Text is legible. The wordmark is spelled
            correctly ("Liv" not "Live").
          - Fail-if (0-1): no typographic overlay, OR text only
            appears on the product label, OR illegible, OR the
            wordmark is mis-spelled.

          ## 9. product_fidelity

          - Pass-if (2-3): the product in frame is recognizably a
            real Liv Conscious SKU (bottle / container with the
            real label and brand identity). Sourced via
            `brand.product` and spliced (Ingredients-to-Video OR
            HTML overlay), NOT `txt2vid`-generated. Bottle shape,
            label color, and typography all match real reference.
          - Fail-if (0-1): generic supplement bottle, hallucinated
            label, no product in frame, OR generated via `txt2vid`.

          ## 10. format_compliance

          - Pass-if (2-3): commercial.mp4 is 1080×1920 (9:16
            vertical). Vertically-composed framing. First 1.5
            seconds work with sound muted.
          - Fail-if (0-1): wrong dimensions, horizontal-shaped
            crop, OR opening requires sound.

          ## 11. programmatic_artifacts

          The DSL discipline gate. Every .json artifact in the
          workdir must have been PRODUCED by a wavelet CLI
          subcommand, not hand-authored. The agent writes Fountain
          (.fountain) for the screenplay and HTML (commercial.html
          + scenes/*.html) for the composition; everything else
          flows through `screenplay parse`, `velocity propose`,
          `storyboard plan`, `transitions classify`, etc.

          - Pass-if (2-3): inspect `transcript.log` for the agent's
            Write tool calls. Every .json file present in the
            workdir corresponds to a matching wavelet CLI invocation
            in `trace.wavelet.jsonl`. No Write call targets a .json
            path. No `comp.json` exists.
          - Fail-if (0-1): transcript shows Write / Edit /
            NotebookEdit authoring a .json file directly, OR a
            .json artifact has no corresponding CLI call, OR
            comp.json exists.

          ## Threshold

          PASS overall ONLY IF:
            - Every dimension scores >= 2, AND
            - Sum of all 11 dimensions >= 26 / 33.

          Otherwise FAIL.
---

# wavelet/007-livconscious

Maximally-adversarial commercial eval. The agent receives one
paragraph of natural language — no brief.md, no SKILL.md, no
pipeline.yaml, no skills pre-loaded. Just the user prompt and the
`wavelet` + `adalign` CLIs on PATH.

Liv Conscious is a deliberately-chosen niche DTC supplement brand
at **livconscious.com** — note "Liv" not "Live", and note the domain
(not liveconscious.com). Failure modes this specifically tests
against:

- agent corrects "Liv" to "Live" and searches the wrong domain
- agent skips `adalign brand.brief` and hallucinates a generic
  supplement catalog
- agent leans into one of the supplement-ad clichés it was warned
  against (slow-mo capsule, lab-coat shot, ingredient cloud)

The strategy_completeness dimension here grades anti-template
discipline: did the agent actually name the clichés and demonstrably
avoid them, or did it pay lip service while still shipping one?

Prompt: [`packages/wavelet/evals/prompts/007-livconscious.txt`](../prompts/007-livconscious.txt)
