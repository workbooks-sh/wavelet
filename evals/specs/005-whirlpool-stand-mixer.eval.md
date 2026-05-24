---
name: wavelet/005-whirlpool-stand-mixer
agent: wavelet.commercial
timeoutMs: 3600000
turns:
  - action:
      kind: wavelet.commercial
      brief: packages/wavelet/evals/prompts/005-whirlpool-stand-mixer.txt
      adversarial: true
      agent: claude
      budget_usd: 10.00
      timeout_ms: 3600000
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
        max_usd: 10.00
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
          # wavelet/005-whirlpool-stand-mixer — 10-dimension adversarial rubric

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

              Make me a Whirlpool ad for their iconic stand mixer,
              9:16 for Meta Reels, about 12 seconds. Budget five
              dollars. Make it look premium.

          The agent must figure out everything else: that the
          "Whirlpool stand mixer" is actually a **KitchenAid** stand
          mixer (Whirlpool acquired KitchenAid in 1986; KitchenAid is
          the SKU that exists), what tools are available, how the
          pipeline works, what a creative strategy looks like.

          ## 1. brand_resolution

          The signature dimension for this eval — did the agent
          resolve the parent→sub-brand mapping?

          - Pass-if (2-3): the agent's `brief.md` and/or `strategy.md`
            explicitly names **KitchenAid** as the actual product
            brand, with reasoning (e.g. "Whirlpool acquired KitchenAid
            in 1986; the stand mixer is a KitchenAid product"). The
            `brand.product` / `brand.brief` calls in the trace
            target `kitchenaid.com`, not `whirlpool.com`. Final
            spot features a recognizable KitchenAid stand mixer.
          - Fail-if (0-1): agent literally tried to find a
            "Whirlpool stand mixer" SKU on whirlpool.com, OR
            generated a generic stand mixer, OR called the product
            by the wrong brand in on-screen text or VO.

          ## 2. tool_discovery

          Did the agent investigate its environment before spending?

          - Pass-if (2-3): `transcript.log` shows the agent ran
            `wavelet --help` AND `brandwork --help` (or equivalent
            subcommand-level help) BEFORE the first paid backend
            call. The agent also inspected
            `wavelet pipelines show commercial` (or equivalent) to
            understand the stage shape.
          - Fail-if (0-1): agent jumped straight into paid calls
            without ever inspecting CLI surface, OR only ran
            `--help` after hitting an error.

          ## 3. self_authored_brief

          The agent was given one paragraph; it must produce a real
          brief.

          - Pass-if (2-3): `workdir/brief.md` exists, authored by
            the agent, and contains brand-specific concrete content
            — KitchenAid product positioning, target audience, hook
            concept, format constraints, success criteria. Not just
            a restatement of the user prompt.
          - Fail-if (0-1): no brief.md, OR brief.md is just the
            verbatim user prompt copied into a file, OR it's
            generic boilerplate that could apply to any product.

          ## 4. strategy_completeness

          The agent was NOT told to produce a strategy pass. Did it
          do one anyway?

          - Pass-if (2-3): `strategy.md` exists and shows real
            strategic thinking — positioning for this specific spot
            (not generic KitchenAid brand positioning), audience
            insight, multiple creative directions considered with
            one chosen and justified, visual register declared and
            locked, at least one competitor pattern named and
            explicitly avoided. Competitive scan cites specific
            ads / brands, not generic category observations.
          - Fail-if (0-1): no strategy.md, OR strategy.md is one
            paragraph of fluff, OR it was clearly back-filled at
            the end of the run to satisfy the rubric (no evidence
            of strategy actually informing the script).

          ## 5. stage_coverage

          - Pass-if (2-3): every stage of the commercial pipeline
            produced an artifact (brief, strategy, script,
            screenplay, velocity, storyboard, assets, cuts, comp,
            render) AND `wavelet workflow run commercial` reports
            every stage `status: complete`.
          - Fail-if (0-1): fewer than 7 of 8 stages produced an
            artifact, OR `wavelet workflow run` reports any stage
            `status: pending`.

          ## 6. final_artifact

          - Pass-if (2-3): the four sampled frames show real
            motion-video content matching a KitchenAid stand-mixer
            ad. No frozen-on-first-frame, no broken chroma, no
            garbled product. MP4 plays at expected duration.
          - Fail-if (0-1): frames are pure background, broken
            chroma, wrong subject, the file is unplayable, 0 bytes,
            or wrong duration.

          ## 7. single_edit_coherence

          The cohesion gate.

          - Pass-if (2-3): the four sampled frames look like they
            came from the SAME shoot — same lens character, same
            lighting register, same color grade. Strategy declared
            a specific visual register and the frames demonstrably
            honor it.
          - Fail-if (0-1): adjacent frames look like different
            cameras, different lighting, or different grades.

          ## 8. on_screen_text

          - Pass-if (2-3): at least one sampled frame shows the
            **KitchenAid** wordmark as an INTENTIONAL typographic
            overlay (not just the wordmark printed on the mixer
            itself). At least one sampled frame OR adjacent context
            includes a CTA or tagline as visible text. Text is
            legible.
          - Fail-if (0-1): no on-screen typographic overlay at all,
            OR text only appears as the printed label on the
            product, OR text is illegible.

          ## 9. product_fidelity

          - Pass-if (2-3): the mixer in frame is recognizably a
            KitchenAid stand mixer — domed motor housing, tilt-head
            or bowl-lift form factor, the iconic silhouette. Color
            is a real KitchenAid colorway (Empire Red, Onyx Black,
            Pistachio, Aqua Sky, etc. — not a hallucinated color).
            Sourced via `brand.product` and spliced (Ingredients-
            to-Video OR HTML overlay), NOT `txt2vid`-generated.
          - Fail-if (0-1): the mixer is generic, clearly a different
            brand, has a hallucinated wordmark, no mixer in frame,
            OR the product was generated via `txt2vid` instead of
            spliced from the real asset.

          ## 10. format_compliance

          - Pass-if (2-3): commercial.mp4 is 1080×1920 (9:16
            vertical). Composition framed for vertical viewing —
            primary subject not crammed at top or bottom, no
            obvious 16:9 letterboxing. The first 1.5 seconds work
            with sound muted (visually arresting enough to stop a
            scroll).
          - Fail-if (0-1): wrong dimensions, framing clearly
            authored for horizontal and crop-rotated, OR opening
            1.5s requires sound to make sense.

          ## 11. programmatic_artifacts

          The DSL discipline gate. Every .json artifact in the
          workdir (screenplay.json, velocity.json, storyboard.json,
          transitions.json, captions.json, etc.) must have been
          PRODUCED by a wavelet CLI subcommand, not hand-authored.
          The agent writes Fountain (.fountain) for the screenplay
          and HTML (commercial.html + scenes/*.html) for the
          composition; everything else flows through `screenplay
          parse`, `velocity propose`, `storyboard plan`,
          `transitions classify`, `captions align`, etc.

          - Pass-if (2-3): inspect `transcript.log` for the agent's
            Write tool calls. Every .json file present in the
            workdir corresponds to a matching wavelet CLI invocation
            in `trace.wavelet.jsonl` that could plausibly have
            produced it (e.g. `screenplay parse` for screenplay.json,
            `velocity propose` for velocity.json). No Write call
            targets a .json path. No `comp.json` exists (it is
            deprecated; composition is HTML-first).
          - Fail-if (0-1): the transcript shows the agent using
            Write / Edit / NotebookEdit to author a .json file
            directly, OR a .json artifact is in the workdir with
            no corresponding wavelet CLI call, OR comp.json exists.

          ## Threshold

          PASS overall ONLY IF:
            - Every dimension scores >= 2, AND
            - Sum of all 11 dimensions >= 26 / 33.

          Otherwise FAIL.
---

# wavelet/005-whirlpool-stand-mixer

Maximally-adversarial commercial eval. The agent receives one
paragraph of natural language — no brief.md, no SKILL.md, no
pipeline.yaml, no skills pre-loaded. Just the user-style prompt and
the `wavelet` + `brandwork` CLIs on PATH.

Tests the realistic Wavelet deployment vibe:

> "I have Claude Code, I installed wavelet + brandwork, I'm sitting at
> my terminal, I want to make a video for my brand."

The signature test is **brand_resolution**: the user said "Whirlpool"
but the SKU is KitchenAid. Whirlpool Corporation has owned KitchenAid
since 1986; the stand-mixer brand a real consumer would name when
saying "Whirlpool stand mixer" is KitchenAid. An agent that calls
`brand.product domain=whirlpool.com query="stand mixer"` has failed
the comprehension test before any pixel is rendered.

Companion failure modes graded across dimensions 1-4:

- agent never inspects `wavelet --help` / `brandwork --help` →
  `tool_discovery`
- agent never produces a real brief from the one-paragraph prompt →
  `self_authored_brief`
- agent skips strategy entirely because the prompt didn't tell it to →
  `strategy_completeness`

Prompt: [`packages/wavelet/evals/prompts/005-whirlpool-stand-mixer.txt`](../prompts/005-whirlpool-stand-mixer.txt)
