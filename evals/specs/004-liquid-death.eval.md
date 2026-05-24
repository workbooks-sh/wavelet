---
name: wavelet/004-liquid-death
agent: wavelet.commercial
timeoutMs: 1800000
turns:
  - action:
      kind: wavelet.commercial
      brief: packages/wavelet/evals/briefs/004-liquid-death.md
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
          # wavelet/004-liquid-death — 11-dimension rubric

          Score each dimension on a 0-3 scale.

          **Overall threshold:** every dimension scores >= 2 AND the
          total sum >= 26 / 33. Otherwise FAIL.

          The four image frames attached are sampled from the rendered
          commercial in temporal order (1s, 4s, 7s, 10s). They are
          1080×1920 (9:16) — treat them as the primary visual evidence
          for dimensions 5, 7, 8, 9, 10.

          You may also read the agent's `strategy.md`, `notes.md`, and
          `transcript.log` from the workdir to ground process-oriented
          dimensions (1, 2, 4, 6, 11).

          ## 1. stage_coverage

          - Pass-if (2-3): every stage of the commercial pipeline
            produced an artifact (brief, strategy, script, screenplay,
            velocity, storyboard, assets, cuts, comp, render) AND
            `wavelet workflow run commercial` reports every stage
            `status: complete`.
          - Fail-if (0-1): fewer than 7 of 8 stages produced an
            artifact, OR `wavelet workflow run` reports any stage
            `status: pending`.

          ## 2. tool_selection

          - Pass-if (2-3): the verb order in `trace.wavelet.jsonl`
            matches the SKILL ordering (brand fetch → script →
            velocity → storyboard → asset → compose → render);
            `brand.brief`, `brand.ads`, `brand.product` ALL invoked
            at least once; product image splice goes through Veo
            Ingredients-to-Video OR HTML overlay, NOT `txt2vid`.
          - Fail-if (0-1): random verb order, missing core brand.*
            calls, OR product appears via `txt2vid` (generated, not
            spliced from the real asset).

          ## 3. budget_discipline

          - Pass-if (2-3): every paid call carries `--max-cost`;
            total `cost_estimate_usd` across the trace <= $5.00.
          - Fail-if (0-1): at least one paid call without
            `--max-cost`, OR total > $5.00.

          ## 4. error_recovery

          - Pass-if (2-3): any failures in `transcript.log` were
            diagnosed and a different path was tried that worked,
            OR no failures occurred.
          - Fail-if (0-1): hit a non-trivial error and either
            (a) kept marching to produce garbage, (b) gave up early,
            or (c) silently succeeded-by-not-trying.

          ## 5. final_artifact

          - Pass-if (2-3): the four sampled frames show real
            motion-video content matching the brief — Liquid Death
            can visible, no obvious mis-render artifacts (no
            frozen-on-first-frame, no broken chroma, no garbled
            product label). MP4 plays at expected duration.
          - Fail-if (0-1): frames are pure background, broken
            chroma, wrong subject, the file is unplayable, 0 bytes,
            or wrong duration.

          ## 6. documentation_use

          - Pass-if (2-3): transcript shows the agent inspected
            `wavelet pipelines show commercial` and/or `wavelet --help`
            early in the run, before calling any paid backend, OR
            the agent had a wavelet-director skill auto-loaded and
            referenced it.
          - Fail-if (0-1): the agent never inspected wavelet's surface
            and bumbled into errors that documentation would have
            prevented.

          ## 7. single_edit_coherence

          The cohesion gate — the dimension this brief is designed
          to test against. Bag-of-clips is the canonical failure
          mode.

          - Pass-if (2-3): the four sampled frames look like they
            came from the SAME shoot — same lens character (DoF +
            focal-length cues), same lighting register, same color
            grade. The agent's `strategy.md §1.4` declared a
            specific visual register, and the frames demonstrably
            honor it. Cuts feel motivated; the spot reads as one
            commercial, not four stitched clips.
          - Fail-if (0-1): adjacent frames look like different
            cameras OR different lighting OR different grades.
            Strategy declared one register but the spot uses
            multiple.

          ## 8. on_screen_text

          - Pass-if (2-3): at least one sampled frame shows the
            "LIQUID DEATH" wordmark as an INTENTIONAL typographic
            overlay (not just the wordmark printed on the can —
            that's the product, not type). At least one sampled
            frame OR the surrounding context (visible in adjacent
            frames or noted by the agent in `notes.md`) shows
            "Murder your thirst" as visible text. Text is legible.
          - Fail-if (0-1): no on-screen typographic overlay at all,
            OR text only appears as the printed label on the
            product, OR text is illegible.

          ## 9. product_fidelity

          - Pass-if (2-3): the can in frame is THE actual Liquid
            Death Mountain Water can — black aluminum tallboy with
            the iconic skull splitting the top. Label text reads
            "LIQUID DEATH" and/or "MOUNTAIN WATER" (partial allowed
            depending on angle). Recognizably the real SKU.
          - Fail-if (0-1): the can is generic, clearly a different
            brand, has a hallucinated label (Veo invented text), no
            can in frame, or the can is so heavily distorted that
            the brand isn't readable.

          ## 10. format_compliance

          - Pass-if (2-3): commercial.mp4 is 1080×1920 (9:16
            vertical). Composition is framed for vertical viewing —
            primary subject not crammed at top or bottom, no
            obvious 16:9 letterboxing. The first 1.5 seconds work
            as a hook with sound muted (visually arresting enough
            to stop a scroll).
          - Fail-if (0-1): wrong dimensions (16:9 or square), OR
            framing was clearly authored for horizontal and
            crop-rotated into vertical, OR the opening 1.5 seconds
            require sound to make sense.

          ## 11. strategy_completeness

          The process gate. The brief required a full creative
          strategy pass BEFORE scripting. Did it actually happen?

          - Pass-if (2-3): `strategy.md` exists and contains all
            four §1.x sections (brand brief synthesis, LD's own
            ads, competitive scan of ≥3 adjacent brands, synthesis
            with the five required bullets). The competitive scan
            cites SPECIFIC ads from SPECIFIC brands — not generic
            category observations. The "what this avoids"
            declaration names a real, identified pattern. The
            final spot demonstrably avoids that pattern.
          - Fail-if (0-1): `strategy.md` missing, OR present but
            thin (one bullet per section, no specific ad citations,
            generic competitive observations like "Celsius uses
            slow-mo"). OR the declared "what this avoids" pattern
            appears in the final spot anyway. OR the agent jumped
            straight to scripting and reverse-engineered the
            strategy doc at the end.

          ## Threshold

          PASS overall ONLY IF:
            - Every dimension scores >= 2, AND
            - Sum of all 11 dimensions >= 26 / 33.

          Otherwise FAIL.
---

# wavelet/004-liquid-death

Brand-centered Meta Reels eval. Tests the realistic Wavelet deployment
vibe — Wavelet as a Claude Code extension, NOT as a Workbooks Studio
feature (see CLAUDE.md "Wavelet ≠ Workbooks Studio"):

> "I have Claude Code, I installed wavelet + brandwork, I'm sitting at my
> terminal, I want to make a video for my brand."

The agent (`claude`) operates as a fresh external user would — only
the brief, the `wavelet` + `brandwork` CLIs on PATH, and whatever skills
Claude Code's discovery loads. No access to monorepo internals, no
awareness of prior failed runs.

Three failure modes this eval was designed to catch (from the
2026-05-20 Liquid Death YT-ad agent run that surfaced `wb-uory`):

- **Bag of clips with no visual consistency** → `single_edit_coherence`
- **No on-screen text** → `on_screen_text`
- **Wrong / hallucinated product** → `product_fidelity`

Plus three new failure modes the rewritten brief tests against:

- **Skipping creative strategy** → `strategy_completeness`
- **Wrong aspect / horizontal-shaped vertical** → `format_compliance`
- **Imitating a specific competitor frame** → graded inside
  `strategy_completeness` (the "what this avoids" declaration must
  match the final spot)

Brief: [`packages/wavelet/evals/briefs/004-liquid-death.md`](../briefs/004-liquid-death.md)
