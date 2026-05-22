---
name: wavelet/006-modern-gents
agent: wavelet.commercial
timeoutMs: 1800000
turns:
  - action:
      kind: wavelet.commercial
      brief: packages/wavelet/evals/prompts/006-modern-gents.txt
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
          # wavelet/006-modern-gents — 10-dimension adversarial rubric

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

              Make me a Modern Gents ad, 9:16 for Meta Reels, about
              12 seconds. Budget five dollars. Should feel masculine
              but not cheesy.

          Modern Gents is a niche DTC men's accessories brand
          (rings, watches, leather goods — moderngents.com). It is
          not a household name; an agent that doesn't actually
          research the brand will hallucinate a generic "men's
          lifestyle" ad that has nothing to do with the real product
          line.

          ## 1. brand_resolution

          The signature dimension — did the agent actually find the
          real brand?

          - Pass-if (2-3): the agent's `brief.md` and/or
            `strategy.md` correctly identifies Modern Gents as a
            DTC men's accessories brand (rings, watches, leather
            goods — the specific product categories should be
            named, not invented). The `brand.brief` / `brand.fetch`
            calls in the trace target `moderngents.com`. Final
            spot features the brand's actual product category, not
            an invented one (e.g. NOT cologne, NOT shaving
            products, NOT clothing — those are different brands).
          - Fail-if (0-1): agent generated an ad for a hallucinated
            "Modern Gents" product line (shaving / cologne /
            clothing / etc.) that doesn't match the real catalog,
            OR never resolved the brand identity and produced
            generic men's-lifestyle filler.

          ## 2. tool_discovery

          - Pass-if (2-3): `transcript.log` shows the agent ran
            `wavelet --help` AND `adalign --help` BEFORE the first
            paid backend call. Agent also inspected
            `wavelet pipelines show commercial` (or equivalent) to
            understand the stage shape.
          - Fail-if (0-1): agent jumped straight into paid calls
            without inspecting CLI surface, OR only ran `--help`
            after hitting an error.

          ## 3. self_authored_brief

          - Pass-if (2-3): `workdir/brief.md` exists, authored by
            the agent, and contains brand-specific concrete content
            grounded in actual Modern Gents catalog (specific
            product category, real positioning, audience). Not a
            restatement of the user prompt.
          - Fail-if (0-1): no brief.md, OR brief.md is just the
            user prompt copied into a file, OR generic men's-
            lifestyle boilerplate.

          ## 4. strategy_completeness

          - Pass-if (2-3): `strategy.md` exists with real strategic
            thinking — positioning for this specific spot,
            audience insight, multiple creative directions
            considered with one chosen, visual register declared
            and locked, at least one competitor pattern named and
            avoided. Competitive scan cites specific ads from
            specific adjacent DTC brands (e.g. Bevel, Beardbrand,
            Ridge Wallet, Vincero, MVMT — pick ones that actually
            overlap with Modern Gents' real category).
          - Fail-if (0-1): no strategy.md, OR fluff, OR
            back-filled at end of run.

          ## 5. stage_coverage

          - Pass-if (2-3): every stage of the commercial pipeline
            produced an artifact AND `wavelet workflow run
            commercial` reports every stage `status: complete`.
          - Fail-if (0-1): fewer than 7 of 8 stages OR any stage
            `status: pending`.

          ## 6. final_artifact

          - Pass-if (2-3): the four sampled frames show real
            motion-video content matching the brand's actual
            product category. No frozen frames, no broken chroma,
            no garbled product. MP4 plays at expected duration.
          - Fail-if (0-1): frames are pure background, broken
            chroma, wrong subject, unplayable, 0 bytes, or wrong
            duration.

          ## 7. single_edit_coherence

          - Pass-if (2-3): the four sampled frames look like the
            same shoot — same lens character, lighting, color
            grade. Strategy's declared visual register honored
            across all frames.
          - Fail-if (0-1): adjacent frames look like different
            cameras, lighting, or grades.

          ## 8. on_screen_text

          - Pass-if (2-3): at least one sampled frame shows the
            Modern Gents wordmark / logo as an INTENTIONAL
            typographic overlay (not just on the product). A CTA
            or tagline appears as visible text in at least one
            frame or adjacent context. Text is legible. The
            masculine-without-cheesy register from the prompt is
            honored typographically (no chrome-effect bro fonts,
            no dripping-blood, etc.).
          - Fail-if (0-1): no on-screen typographic overlay, OR
            text only on the product, OR illegible, OR text falls
            into the explicit "cheesy" failure mode (chrome /
            blood / flame / dagger fonts).

          ## 9. product_fidelity

          - Pass-if (2-3): the product in frame is recognizably
            something Modern Gents actually sells, sourced via
            `brand.product` and spliced (Ingredients-to-Video OR
            HTML overlay), NOT `txt2vid`-generated. The product's
            visual character (metal finish, leather grain, etc.)
            is consistent with real reference images.
          - Fail-if (0-1): the product is generic, hallucinated,
            wrong category for the brand, no product in frame, OR
            generated via `txt2vid` instead of spliced from real
            asset.

          ## 10. format_compliance

          - Pass-if (2-3): commercial.mp4 is 1080×1920 (9:16
            vertical). Vertically-composed framing. First 1.5
            seconds work with sound muted.
          - Fail-if (0-1): wrong dimensions, horizontal-shaped
            vertical crop, OR opening requires sound.

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

# wavelet/006-modern-gents

Maximally-adversarial commercial eval. The agent receives one
paragraph of natural language — no brief.md, no SKILL.md, no
pipeline.yaml, no skills pre-loaded. Just the user prompt and the
`wavelet` + `adalign` CLIs on PATH.

Modern Gents is a deliberately-chosen **niche DTC brand** the model
will not have strong priors about — moderngents.com sells men's
accessories (rings, watches, leather goods). Failure modes this
specifically tests against:

- agent invents a generic "Modern Gents" product line (shaving /
  cologne / suiting) that has nothing to do with the real catalog
- agent skips `adalign brand.brief` entirely and works from training-
  data hallucinations
- agent leans into "masculine" with the cheesy register the prompt
  explicitly warned against (chrome fonts, blood, flame effects)

The brand-resolution dimension here grades research depth, not just
parent→sub-brand mapping (which is the 005 test).

Prompt: [`packages/wavelet/evals/prompts/006-modern-gents.txt`](../prompts/006-modern-gents.txt)
