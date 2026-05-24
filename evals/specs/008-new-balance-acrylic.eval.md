---
name: wavelet/008-new-balance-acrylic
agent: wavelet.commercial
timeoutMs: 3600000
turns:
  - action:
      kind: wavelet.commercial
      brief: packages/wavelet/evals/prompts/008-new-balance-acrylic.txt
      adversarial: true
      agent: claude
      budget_usd: 7.00
      timeout_ms: 3600000
      pipeline: commercial
    checks:
      - kind: wavelet.video_renders
        path: ctx:waveletCommercialMp4
        duration_secs: 25
        duration_tolerance_secs: 4.0
        width: 1080
        height: 1920
        codec: h264
      - kind: wavelet.cost_below
        trace: ctx:waveletTrace
        max_usd: 7.00
      - kind: wavelet.workflow_complete
        workdir: ctx:waveletWorkdir
        pipeline: commercial
      - kind: rubric.passes
        target: assistant_text
        minScore: 0.7
        attachments:
          mp4_path: ctx:waveletCommercialMp4
          frame_at_secs: [2.0, 8.0, 15.0, 22.0]
        rubric: |
          # wavelet/008-new-balance-acrylic — 10-dimension rubric

          Score each dimension on a 0-3 scale.

          **Overall threshold:** every dimension scores >= 2 AND the
          total sum >= 22 / 30. Otherwise FAIL.

          The four image frames attached are sampled from the rendered
          commercial (2s, 8s, 15s, 22s). They are 1080×1920 (9:16) —
          treat them as the primary visual evidence for dimensions 4,
          5, 6, 7, 8.

          You may also read `brief.md`, `strategy.md`, `notes.md`, and
          `transcript.log` to ground process-oriented dimensions (1, 2,
          3, 9, 10).

          ## Adversarial context

          The agent received ONE paragraph of natural language, no
          markdown, no instructions, no skills staged. The user wrote:

              Make me a New Balance ad for their 9060 sneaker in one of
              the colorful colorways like Mint Julep or Sea Salt, 9:16
              for Reels and TikTok, about 25 seconds long. The vibe is
              acrylic 3D motion graphics — fluid liquid color, viscous
              paint, swirling pigment, premium fashion-house energy,
              NOT a lifestyle UGC spot. Lean into text inside the
              actual generated shots: brand wordmark forming from
              acrylic paint, the product name materializing in motion,
              type as a primary visual element. End with a clean
              overlay CTA so the call-to-action stays crisp. Budget
              seven dollars.

          The signature test is whether the agent EXECUTES THE STATED
          STYLE — production motion graphics with text-in-generation —
          instead of falling back to the default UGC/lifestyle pattern
          baked into the skill.

          ## 1. brand_research

          - Pass-if (2-3): `transcript.log` shows `brandwork brief`,
            `brandwork brand`, and `brandwork ads` calls against
            newbalance.com with non-trivial JSON output captured.
            `brief.md` / `strategy.md` reference real New Balance brand
            attributes (heritage running, "fearlessly independent",
            grey/silver/orange palette, etc.) sourced from those calls.
          - Fail-if (0-1): no brandwork calls, OR calls against the wrong
            domain, OR `--help` probes only.

          ## 2. style_adherence

          The signature dimension. Did the agent COMMIT to the
          requested motion-graphics aesthetic?

          - Pass-if (2-3): every sampled frame shows acrylic / fluid /
            3D motion-graphics treatment (viscous paint, swirling
            color, ribbon flows, geometric solids in liquid). NO
            handheld UGC framing, NO product-on-pedestal stock, NO
            lifestyle "person wearing the shoe" cuts. Strategy.md
            explicitly names the visual register and locks it.
          - Fail-if (0-1): one or more frames default to lifestyle UGC
            or generic product shots; the agent ignored the style
            direction in favor of the skill's default UGC pattern.

          ## 3. text_in_generation

          The agent was told to lean on text-in-generation. Did it?

          - Pass-if (2-3): at least 2 of the 4 sampled frames contain
            text that was generated INSIDE the Veo clip (brand
            wordmark forming from paint, product name materializing in
            motion). The text is recognizable as "NEW BALANCE" / "9060"
            / similar — not garbled letterforms. Distinct from the
            final HTML CTA overlay.
          - Fail-if (0-1): no text-in-generation evident, OR every
            piece of type is an HTML overlay, OR the baked text is
            garbled to the point of being illegible.

          ## 4. final_cta_overlay

          The agent was instructed to keep the FINAL CTA as a clean
          HTML overlay — not text-in-generation — so the call-to-action
          stays crisp.

          - Pass-if (2-3): the last sampled frame (22s) shows a clean,
            HTML-rendered CTA — recognizable New Balance wordmark,
            product URL or shop CTA, legible at 1080×1920 scale. The
            text passes the halo-contrast check (validated by
            `wavelet lint --mp4 commercial.mp4` finding 0 contrast
            errors on the CTA scene).
          - Fail-if (0-1): the final CTA is a Veo-generated clip with
            unreliable text, OR there is no CTA scene at all, OR the
            CTA text fails contrast.

          ## 5. shot_count_and_pacing

          25s spot at the requested 6-9 shot count.

          - Pass-if (2-3): 6-10 distinct shots, average shot duration
            2.5-3.5s, hook shot (first 1.5-2s) visually arresting
            enough to stop a scroll. Pacing matches the motion-graphics
            register (longer dwell per shot than UGC, but not static).
          - Fail-if (0-1): fewer than 5 shots OR more than 12 shots,
            OR every shot identical duration (machine pacing), OR the
            opening 2s doesn't earn the scroll.

          ## 6. final_artifact

          - Pass-if (2-3): the four sampled frames show real motion-
            video content. No frozen-on-first-frame, no broken chroma,
            no garbled product. MP4 plays at expected duration (~25s).
          - Fail-if (0-1): frames are pure background, broken chroma,
            wrong subject, the file is unplayable, 0 bytes, or
            duration < 18s or > 32s.

          ## 7. product_fidelity

          - Pass-if (2-3): the 9060 sneaker visible in at least one
            frame is recognizable as a New Balance silhouette — chunky
            mesh upper, "N" logo, retro-runner profile. The chosen
            colorway is real (Mint Julep, Sea Salt, Generation 9000,
            etc. — not a hallucinated palette).
          - Fail-if (0-1): the shoe is generic, clearly a different
            brand silhouette, has a hallucinated wordmark, OR no shoe
            in frame.

          ## 8. single_edit_coherence

          - Pass-if (2-3): the four frames look like they came from
            the SAME directed piece — consistent color script (the
            acrylic palette repeats), consistent lighting register,
            consistent motion-graphics language. The strategy declared
            a specific visual register and the frames honor it.
          - Fail-if (0-1): adjacent frames look like different
            templates / different art directions / different brand
            properties.

          ## 9. discipline_artifacts

          The DSL discipline gate. Same rules as 005.

          - Pass-if (2-3): every .json artifact in the workdir
            corresponds to a wavelet CLI invocation in
            `trace.wavelet.jsonl`. No Write tool call targets a .json
            path. No `comp.json` exists. `wavelet screenplay validate`
            was invoked and exited 0 before storyboard. `wavelet lint
            commercial.html --mp4 commercial.mp4` was invoked and
            exited 0 before compose completed.
          - Fail-if (0-1): the agent hand-authored a .json file, OR
            skipped the screenplay-validate step, OR skipped the
            post-render lint pass.

          ## 10. format_compliance

          - Pass-if (2-3): commercial.mp4 is 1080×1920 (9:16). First
            1.5s works with sound muted — visually arresting enough to
            stop a scroll. Composition framed for vertical viewing.
          - Fail-if (0-1): wrong dimensions, framing clearly authored
            for horizontal, OR opening 1.5s requires sound to make
            sense.

          ## Threshold

          PASS overall ONLY IF:
            - Every dimension scores >= 2, AND
            - Sum of all 10 dimensions >= 22 / 30.

          Otherwise FAIL.
---

# wavelet/008-new-balance-acrylic

Style-direction eval. Tests whether the agent can EXECUTE a stated
visual style (acrylic 3D motion graphics with text-in-generation)
instead of falling back to the lifestyle-UGC default the skill biases
toward.

The pipeline supports this style without code changes, but the
director skill's bias is strongly UGC. This eval checks whether the
agent reads the brief carefully enough to lock the style register.

Companion failure modes:

- Agent defaults to lifestyle UGC despite explicit direction →
  `style_adherence`
- Agent uses only HTML overlays for all type, never leveraging Veo's
  text generation → `text_in_generation`
- Agent bakes the CTA into a Veo clip and gets garbled letterforms →
  `final_cta_overlay`
- Agent skips `wavelet screenplay validate` or `wavelet lint --mp4` →
  `discipline_artifacts`

Prompt: [`packages/wavelet/evals/prompts/008-new-balance-acrylic.txt`](../prompts/008-new-balance-acrylic.txt)
