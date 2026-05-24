---
name: wavelet/009-new-balance-office-ugc
agent: wavelet.commercial
timeoutMs: 3600000
turns:
  - action:
      kind: wavelet.commercial
      brief: packages/wavelet/evals/prompts/009-new-balance-office-ugc.txt
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
          # wavelet/009-new-balance-office-ugc — 10-dimension rubric

          Score each dimension on a 0-3 scale.

          **Overall threshold:** every dimension scores >= 2 AND the
          total sum >= 22 / 30. Otherwise FAIL.

          Four image frames attached (2s, 8s, 15s, 22s) — primary
          evidence for dimensions 4, 5, 6, 7, 8.

          You may also read `brief.md`, `strategy.md`, `notes.md`, and
          `transcript.log` to ground process-oriented dimensions.

          ## Adversarial context

          The agent received ONE paragraph of natural language, no
          markdown, no instructions, no skills staged. The user wrote:

              Make me a New Balance ad for the Made in USA 990v6, 9:16
              for Reels, about 25 seconds. The vibe is corporate-
              lifestyle UGC — a person wearing the shoes through a
              modern office (coffee, standup, walking the floor, desk
              work), shot like a coworker filmed it on their phone.
              Slightly on-the-nose New-Balance-y energy: comfort,
              quality, made-for-the-real-day positioning. Use HTML
              text overlays for the brand wordmark, product code, and
              CTA — not text inside the Veo clips. Try to keep the
              same person recognizable across the cuts. Budget seven
              dollars.

          The signature test is whether the agent can EXECUTE a
          25-second lifestyle UGC spot with CONSISTENT CHARACTER
          across multiple cuts and ALL TEXT VIA HTML OVERLAYS —
          standard pipeline patterns, longer-form duration than 005.

          ## 1. brand_research

          - Pass-if (2-3): `transcript.log` shows `brandwork brief`,
            `brand`, `ads` calls against newbalance.com with non-
            trivial JSON output captured. Strategy.md cites real New
            Balance positioning (heritage running, Made in USA
            premium, "fearlessly independent", etc.).
          - Fail-if (0-1): no brandwork calls, wrong domain, or `--help`
            probes only.

          ## 2. style_adherence

          - Pass-if (2-3): every sampled frame reads as authentic
            corporate-lifestyle UGC — modern office environment,
            handheld feel, natural light, candid framing. The
            New-Balance "real-day" positioning shows up in the
            choices (coffee, standup, walking, desk — not gym, not
            running, not fashion editorial). Strategy.md explicitly
            locks the register.
          - Fail-if (0-1): frames default to product-on-pedestal
            stock, OR fashion editorial, OR sports/athletic register
            instead of office lifestyle.

          ## 3. character_consistency

          The new dimension this eval introduces. Veo's identity
          drift between clips is the known failure mode.

          - Pass-if (2-3): the person in the sampled frames is
            recognizably the same individual across cuts —
            consistent wardrobe (or coherent wardrobe progression),
            consistent build / hair / skin tone. `transcript.log`
            shows the agent used Ingredients-to-Video (or a
            reference-image conditioning step) at least once OR
            scripted shots that avoid identity-revealing close-ups.
          - Fail-if (0-1): frames show clearly different people in
            different cuts, OR identity drift is so severe the spot
            reads as a montage of different actors.

          ## 4. on_screen_text_via_html

          The agent was told to use HTML overlays, NOT text-in-
          generation.

          - Pass-if (2-3): at least one sampled frame shows the New
            Balance wordmark or product code as a clean HTML overlay
            (not garbled letterforms from a Veo clip). The CTA is HTML.
            All on-screen type passes the halo-contrast lint (verified
            by `wavelet lint --mp4 commercial.mp4` finding 0 contrast
            errors on text scenes).
          - Fail-if (0-1): text is baked into Veo clips and shows
            kerning artifacts / typos / illegibility, OR there is no
            on-screen brand identification at all.

          ## 5. shot_count_and_pacing

          25s spot at the UGC pace — 8-12 cuts.

          - Pass-if (2-3): 8-12 distinct shots, average duration
            1.8-2.8s, hook ≤ 2s, no scene drags past 4s. Pacing reads
            like a real UGC edit, not stock-footage slideshow.
          - Fail-if (0-1): fewer than 6 shots (stock-footage feel),
            OR more than 14 (frenetic / unmotivated), OR every shot
            identical duration.

          ## 6. final_artifact

          - Pass-if (2-3): four frames show real motion-video content
            — no frozen frames, broken chroma, garbled product. MP4
            plays at expected duration (~25s, 21-29s acceptable).
          - Fail-if (0-1): frames are static, broken, wrong subject,
            or duration < 18s / > 32s.

          ## 7. product_fidelity

          - Pass-if (2-3): the 990v6 sneaker visible in at least one
            frame is recognizable as a New Balance silhouette —
            classic running-shoe profile, "N" logo, grey/silver/cream
            palette typical of Made in USA. Worn on a person, not
            propped on a surface.
          - Fail-if (0-1): generic sneaker, different brand
            silhouette, hallucinated wordmark, OR the shoe never
            appears worn (only static product shots).

          ## 8. single_edit_coherence

          - Pass-if (2-3): frames look like the SAME office, SAME
            day, SAME light direction. Consistent grade (the
            mid-afternoon office-light register holds across cuts).
            Strategy.md locks the look.
          - Fail-if (0-1): adjacent frames look like different
            offices / different times of day / different grades.

          ## 9. discipline_artifacts

          Same DSL discipline gate as 005 + 008. Adds the new
          pipeline gates that landed this session.

          - Pass-if (2-3): every .json artifact corresponds to a
            wavelet CLI call in trace. No hand-authored .json. No
            `comp.json`. `wavelet screenplay validate` invoked and
            exited 0 before storyboard. `wavelet lint commercial.html
            --mp4 commercial.mp4` invoked and exited 0 before compose
            completed. Layout-axis, glyph-clip (incl. canvas-viewport),
            and halo-contrast all clear.
          - Fail-if (0-1): hand-authored .json, OR skipped validate,
            OR skipped post-render lint, OR shipped with any of the
            new lint rules failing.

          ## 10. format_compliance

          - Pass-if (2-3): 1080×1920 (9:16). First 1.5s works with
            sound muted. Composition framed for vertical.
          - Fail-if (0-1): wrong dimensions, horizontal-framed and
            crop-rotated, OR opening 1.5s requires sound.

          ## Threshold

          PASS overall ONLY IF:
            - Every dimension scores >= 2, AND
            - Sum of all 10 dimensions >= 22 / 30.

          Otherwise FAIL.
---

# wavelet/009-new-balance-office-ugc

Long-form lifestyle eval. Tests the existing pipeline's bread and
butter — UGC + HTML overlays — at 25 seconds (~2× the duration of
005). Introduces the `character_consistency` dimension to probe Veo's
identity drift across cuts, and explicitly checks the new pipeline
gates (screenplay-validate + post-render lint with `--mp4`) shipped
this session.

Companion failure modes:

- Agent drifts into product-on-pedestal stock instead of UGC →
  `style_adherence`
- Different actor per cut (Veo identity drift) →
  `character_consistency`
- Agent puts text inside Veo clips and gets garbled output →
  `on_screen_text_via_html`
- Agent skips `wavelet screenplay validate` or the post-render lint →
  `discipline_artifacts`

Prompt: [`packages/wavelet/evals/prompts/009-new-balance-office-ugc.txt`](../prompts/009-new-balance-office-ugc.txt)
