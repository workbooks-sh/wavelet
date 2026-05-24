---
name: wavelet/010-ugc-character-to-camera
agent: wavelet.commercial
timeoutMs: 3600000
turns:
  - action:
      kind: wavelet.commercial
      brief: packages/wavelet/evals/prompts/010-ugc-character-to-camera.txt
      adversarial: true
      agent: claude
      budget_usd: 9.00
      timeout_ms: 3600000
      pipeline: commercial
    checks:
      - kind: wavelet.video_renders
        path: ctx:waveletCommercialMp4
        duration_secs: 24
        duration_tolerance_secs: 4.0
        width: 1080
        height: 1920
        codec: h264
      - kind: wavelet.cost_below
        trace: ctx:waveletTrace
        max_usd: 9.00
      - kind: wavelet.workflow_complete
        workdir: ctx:waveletWorkdir
        pipeline: commercial
      - kind: rubric.passes
        target: assistant_text
        minScore: 0.75
        attachments:
          mp4_path: ctx:waveletCommercialMp4
          frame_at_secs: [1.5, 6.0, 11.0, 16.0]
        rubric: |
          # wavelet/010-ugc-character-to-camera — 8-dimension rubric

          Score each dimension on a 0-3 scale.

          **Overall threshold:** every dimension scores >= 2 AND the
          total sum >= 18 / 24. Otherwise FAIL.

          Four image frames attached (1.5s, 6s, 11s, 16s) — primary
          evidence for dimensions 1, 2, 3, 4, 5, 8. Dimensions 6 and 7
          read from `transcript.log` and the workdir.

          ## Adversarial context

          The agent received ONE paragraph of natural language, no
          markdown, no instructions, no skills staged. The brand is
          Bubble Skincare — a real Gen Z UGC-native skincare line
          (bubbleskincare.com) — so `brandwork resolve` should
          surface the real domain and the research gate should pass
          cleanly.

          The signature test is whether the agent uses the new
          `wavelet character define` primitive (wb-cx08) + a separate
          hands ref (wb-jwnk) + `fal-veo3-ref` backend (wb-3pg7) to
          keep the same woman across face shots and a credible ECU
          hand cutaway, with `fal-veo3-ref-fast`'s 8s-per-clip lock
          dictating ~3 hero shots of 8s each.

          ## 1. character_face_consistency

          The new dimension this eval exists to probe.

          - Pass-if (2-3): the woman in the three sampled face frames
            (1.5s / 6s / 11s) is recognizably the same individual —
            consistent build, hair, skin tone, eye color, and
            wardrobe (or coherent wardrobe progression).
            `transcript.log` shows the agent invoked
            `wavelet character define ... --type full-body`
            at least once before any shot generation, AND used
            `--backend fal-veo3-ref` with `--reference` on the face
            shots.
          - Fail-if (0-1): different actors per cut, OR severe
            identity drift, OR the agent never used the
            character-define primitive (storyboard fell through to
            stock / plain txt2vid for the dialogue shots).

          ## 2. hand_shot_separate

          - Pass-if (2-3): the 16s sampled frame is an ECU of hands
            (palm-forward or mid-grip on the product), distinct from
            the face shots. The hands read as belonging to the same
            woman (skin tone, jewelry, nails consistent) but were
            generated from a separate `--type hands` reference.
            `transcript.log` shows
            `wavelet character define ... --type hands` was called.
          - Fail-if (0-1): the agent reused the face ref for the
            hand shot (face features leak into the cutaway), OR the
            hand shot is replaced by a non-hand shot, OR no ECU
            cutaway exists at all.

          ## 3. product_visible

          - Pass-if (2-3): a serum bottle / dropper is visible in at
            least one face frame AND in the hand cutaway. The bottle
            looks like a coherent product (consistent shape, color,
            and label across shots). The wordmark on the bottle is
            either HTML-overlaid or legibly generated.
          - Fail-if (0-1): no product visible, OR three different-
            looking bottles, OR the product is just stock
            cosmetic-aisle b-roll.

          ## 4. ugc_register

          - Pass-if (2-3): every sampled frame reads as authentic
            creator-to-camera UGC — handheld feel, natural light
            (bathroom / vanity / window light), one woman talking
            directly to camera. No tripod-locked product-on-pedestal
            framing. No professional studio register.
          - Fail-if (0-1): frames look like a polished brand spot,
            OR feature multiple actors, OR have voiceover-only with
            no face-to-camera.

          ## 5. html_text_only

          - Pass-if (2-3): the brand wordmark and the CTA appear as
            clean HTML overlays in at least one frame (not garbled
            letterforms baked into a Veo clip). All on-screen type
            passes the halo-contrast lint
            (`wavelet lint commercial.html --mp4 commercial.mp4`
            exits 0 on text scenes).
          - Fail-if (0-1): brand wordmark or CTA is baked into a Veo
            clip with garbled kerning, OR no on-screen brand
            identification, OR halo-contrast lint reports errors.

          ## 6. duration_compliance

          - Pass-if (2-3): MP4 duration falls within 20-28s (target
            24s ±4s). `wavelet screenplay validate` invoked and
            exited 0 before shot generation. Note: `fal-veo3-ref-fast`
            is locked to 8s per clip — typical structures are
            3×8s = 24s or 2×8s + 8s HTML CTA scene = 24s.
          - Fail-if (0-1): duration < 20s or > 28s, OR screenplay
            validate skipped, OR validate exited non-zero and the
            agent shipped anyway.

          ## 7. pipeline_discipline

          The same DSL discipline gate 005 / 008 / 009 ship with,
          tightened for the new pipeline gates.

          - Pass-if (2-3): NO hand-authored `.json` artifacts; every
            JSON in workdir corresponds to a wavelet CLI call in
            trace. NO `comp.json` (HTML-only render rule).
            `wavelet screenplay validate` invoked + exit 0 before
            storyboard. `wavelet lint commercial.html --mp4
            commercial.mp4` invoked + exit 0 before compose
            completed. Storyboard verify pass.
          - Fail-if (0-1): hand-authored .json, OR skipped validate,
            OR skipped post-render lint, OR shipped with any lint
            rule failing.

          ## 8. final_artifact

          - Pass-if (2-3): four frames show real motion-video content
            — no frozen frames, broken chroma, garbled product. MP4
            plays at expected duration. 1080×1920 (9:16). First 1.5s
            works with sound muted.
          - Fail-if (0-1): frames are static, broken, wrong subject,
            duration < 16s / > 20s, wrong aspect ratio, or opening
            1.5s requires sound.

          ## Threshold

          PASS overall ONLY IF:
            - Every dimension scores >= 2, AND
            - Sum of all 8 dimensions >= 18 / 24.

          Otherwise FAIL.
---

# wavelet/010-ugc-character-to-camera

The character-consistency acid test. Exercises three features that
landed in the current session as a pipeline:

- **wb-7pxv** — `fountain::screenplay_characters()` extractor (lets
  the planner reason about cues)
- **wb-cx08** — `wavelet character define <NAME> --reference <PATH>
  --type full-body|hands|product-hands`, clip-HTML refs auto-loaded
  by the storyboard planner, `Generation::RefConditioned` routing
- **wb-jwnk** — ECU hand-cutaway heuristic prefers a same-character
  `--type hands` ref over `full-body`, verifier WARNs on face-leak
- **wb-3pg7** — Fal Veo 3.1 reference-to-video backend
  (`fal-veo3-ref` / `fal-veo3.1-ref`) with `--reference` flag on
  `wavelet shot txt2vid`

Companion failure modes the rubric specifically probes:

- Agent skips `wavelet character define` and lets Veo identity-drift
  the actor between shots → `character_face_consistency`
- Agent uses the face ref for the hand cutaway, face features leak →
  `hand_shot_separate`
- Agent bakes "DEW DROP SERUM" wordmark into a Veo clip →
  `html_text_only`
- Agent skips screenplay-validate or post-render lint →
  `pipeline_discipline`

Brief mentions a fictional brand — `brandwork resolve` should surface
no real domain. The agent has to fabricate a credible visual identity
without the usual brand-research crutch.

Prompt: [`packages/wavelet/evals/prompts/010-ugc-character-to-camera.txt`](../prompts/010-ugc-character-to-camera.txt)
