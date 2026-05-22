# Verdict — 2026-05-19-tree-runner-real-paid

**Brief:** `briefs/003-freeform-palette.md` (Aesop Marrakech Intense, freeform-palette stress)
**Agent:** claude (workhorse, fresh context, `--dangerously-skip-permissions`, stream-json)
**Started:** 2026-05-19T20:24:26Z
**Finished:** 2026-05-19T20:34:09Z
**Wall time:** 9m 43s
**Paid spend (sum from `.gamut-cache/**/*.manifest.json`):** **$0.46**
- elevenlabs-music: $0.06 (one 12s phrase)
- fal-wan-t2v ×4: $0.40 (four 5s shots @ $0.10)
- Brief ceiling was $5.00; runner budget was $10.00. Came in at 9.2% of the runner ceiling.

## Scores (0–3 per dimension; see judge.md)

| Dimension          | Score | Notes |
|--------------------|-------|-------|
| Stage coverage     | 3     | `gamut workflow run commercial` reports every one of the 8 stages (research/script/velocity/storyboard/asset/edit/compose/publish) as `complete`; `next_stage: null`. Agent had to stub `cuts.edl` + `captions.json` to satisfy the `edit` gate for a no-VO music spot — a soft-failure note, not a coverage gap. |
| Tool selection     | 3     | Verb order matches SKILL.md's eight-stage recipe: `brief check` → `screenplay parse` → `velocity propose` → `storyboard plan` → `storyboard verify` → `continuity check` → `transitions classify` → `music gen` → `shot txt2vid` ×4 → `render`. No wrong-cluster calls (no `txt2img` for stills, etc.). Picked the cheapest backend for shots (Wan-T2V at $0.10 instead of Veo at $0.25, even though $0.25 was budgeted). |
| Budget discipline  | 3     | Every paid call carried `--max-cost`: music `--max-cost 0.15`, each shot `--max-cost 0.25`. Came in at $0.46 vs $5.00 brief ceiling — well under, with room left over for variants if the brief had asked. No `--dry-run` sweeps used; the brief explicitly forbade them ("Live mode required"). |
| Error recovery     | 3     | Two recoveries, both clean: (a) `gamut brief check brief.md` rejected the eval's long-form brief — agent extracted the 9-line slot-filled block, wrote it back as brief.md, preserved the original at `eval-instructions.md`, retry passed. (b) First `gamut render index.html -o commercial.mp4` panicked (exit 101) on the original scenes containing crossfade transitions — agent diagnosed the shader bug (`unknown identifier 'progress'`), switched to plain cuts, render passed second time. Diagnosed-and-rerouted, not "retried until it worked." |
| Final artifact     | 3     | `commercial.mp4` is 1280×720 h264 @ 30fps, 11.97s, 9.96 MB (6.66 Mbit/s), with synced 12s music. Plays clean. Four visually + typographically distinct scenes. On-brief: editorial, restrained, mid-century print-magazine grade. Spot reads as a draft cut a creative director would mark up, not rerun. |
| Documentation use  | 3     | Read SKILL.md early (the runner copies it into workdir, and the transcript shows Reads against it before tool calls). Used the eases.css table by reference per the recipe. Consulted `gamut shot txt2vid --help` and `gamut render --help` before invoking them. Walked `gamut pipelines show commercial` indirectly via the workdir-copied `commercial.yaml`. |
| **Total / 18**     | **18** | |

## Palette checklist (the actual point of this brief)

All seven required items present across `scenes/*.html`:

1. **`clip-path` non-rectangle** — `02-souk.html` hexagonal `.port` (polygon, 6 vertices) AND `04-alley.html` chevron `.seal` (polygon, 6 vertices). Both shapes visible in rendered frames.
2. **`mix-blend-mode` non-`normal`** — `03-tannery.html` "MARRAKECH" set in `mix-blend-mode: difference` (clearly visible as cyan/blue inversion against saffron+oxblood leather in sample-7s.png); `02-souk.html` port in `mix-blend-mode: lighten`. Two distinct modes.
3. **3+ distinct `@keyframes`** — eight distinct animations: `drift-up`, `iris`, `hud-flicker`, `settle`, `tag-rise`, `whisper-in`, `pop-cta`, `seal-in`. Far past the floor.
4. **Extended eases** — four different `var(--ease-out-*)` across four scenes: `ease-out-quint` (S1), `ease-out-circ` (S2), `ease-out-expo` (S3), `ease-out-back` (S4). Past the two-scene floor.
5. **`<video>` element** — all 4 scenes use inline `<video src="../shots/…" muted>` full-bleed. The Stylo + per-frame seek path (newly wired this epic) works end-to-end.
6. **Top-level `<audio>`** — `<audio src="music/track.wav" data-spans="all" data-volume="0.85" data-fade-in="0.3s" data-fade-out="1.2s">` lives in `index.html`, not a sidecar `comp.json` cue. The agent went with the manifest pattern.
7. **Typographic variety** — four distinct typefaces across four cuts: Bodoni Moda 6.4vw italic-mixed (editorial serif) → JetBrains Mono 13px (HUD monospace, 4-corner layout) → Helvetica Neue 16vw 900 (knockout difference-blend) → Inter 24px 200-weight wide-tracked (whisper CTA). Zero shared lockups, zero adjacent matches.

**Verdict on palette use:** the agent did NOT default to the AI-default bottom-left Inter-88px static lockup the brief was stress-testing against. It reached for the freeform palette deliberately. This is the headline finding.

## What worked

- The SKILL.md updates + new freeform-palette idioms got picked up. The agent specifically cited the canonical "type carved through video via difference blend" and used it on the tannery scene as the spot's hero shot.
- Parallel asset gen: agent launched music + 4 shots concurrently via Bash background tasks, total wall time for the paid stage was ~4 minutes (vs ~17 min serial).
- Cost-conscious backend selection: chose Wan-T2V over Veo even when Veo was within `--max-cost`. The agent appears to be doing implicit budget-vs-quality reasoning, not just gating on the `--max-cost` flag.
- Manifest pattern adoption was clean — `<section data-scene-href data-duration>` plus top-level `<audio data-spans="all">`. No `comp.json`-first authoring, exactly the post-MotionSpec authoring shape we wanted.
- Two graceful recoveries (brief format, transition shader bug) with no flailing — agent diagnosed, took a different path, kept moving.

## What broke

- **First render attempt panicked.** `gamut render index.html` exited 101 with `shady parse: unknown identifier 'progress'` from `render_offline.rs:299`. Root cause appears to be the crossfade transition shader. Agent worked around by switching to hard cuts. The transition itself needs filing — `progress` must be passed as a uniform or let-bound in the shader entry point.
- **`brief check` rejected the eval's brief file.** The brief at `briefs/003-freeform-palette.md` is a long-form markdown spec, not a 9-line slot-filled brief. `gamut brief check` expects PRODUCT/AUDIENCE/INSIGHT/PROMISE/PROOF/TONE/MUSIC/CALL/RUNTIME slots. The eval brief embeds them, but the slots aren't first-class. Agent had to extract them and rewrite the file. This is a brief-format mismatch — the eval harness should provide the slot-filled brief directly, or `brief check` should be lenient about leading prose.
- **Workflow runner expects `music.wav` at the workdir root**, not `music/track.wav` (the SKILL.md convention). The agent had to copy the file to satisfy the `asset` stage gate. File-name convention drift between SKILL.md and `pipeline_defs/commercial.yaml`.
- **`edit` stage requires `cuts.edl` + `captions.json` for a music-only no-VO spot.** Agent stubbed both to satisfy the gate. The pipeline YAML should mark these optional for music-only commercials, or the stage should accept a "no-vo" sentinel.

## Surprises

- The agent unprompted re-read the gamut source (`src/inline_video.rs`, `tests/inline_video_smoke.rs`) to verify inline `<video>` actually paints. That's adversarial-checking behavior on top of just trusting SKILL.md.
- The agent's notes.md flagged TWO concrete updates SKILL.md needs:
  1. The "what doesn't work" section says inline `<video>` doesn't paint — it does.
  2. The `clip-path` exclusion list is stale on `inset()`.
- The `gamut storyboard plan` auto-generated 8 shots from a 4-scene screenplay (each scene → est + action sub-shots), totaling 13.4s instead of 12s. Agent overrode this by hand-authoring 4 shots × 3s. Storyboard auto-plan needs a `--match-runtime` flag or stricter timing constraint.
- Workdir size: 25 MB total (10 MB final + 8.8 MB shot rushes + audio + frames). Lighter than expected.

## Findings for gamut (file as follow-up issues in wb-e8jh epic)

1. **Crossfade transition shader broken in `render_offline.rs:299`** — `shady parse: unknown identifier 'progress' (no let binding and not called as a function)`. Blocks the transitions classifier output from actually being honored at render time. (Highest-priority finding.)
2. **`gamut brief check` is too strict on leading prose** — eval briefs ship with markdown context around the 9-line block; agent has to extract. Either parse loose markdown with slot detection, or document the strict format more loudly.
3. **`commercial.yaml` workflow gate expects `music.wav` at workdir root** but SKILL.md says `music/track.wav`. Convention drift — fix one or accept both layouts.
4. **`edit` stage requires `cuts.edl` + `captions.json` even for music-only spots** with no dialogue. Either mark optional in the YAML, or accept a "no-vo" sentinel value.
5. **`storyboard plan` auto-shot-count doesn't match runtime** — generated 8 shots @ 13.4s for a 12s brief. Add `--match-runtime` constraint or shot-count cap.
6. **SKILL.md stale claims** — agent flagged that the "doesn't work" section incorrectly excludes inline `<video>` (it does work now) and possibly `clip-path: inset()`. Audit + update.
7. **One scene reached an `--ease-out-quint`/`circ`/`expo`/`back` each** — the agent didn't use `--ease-in-*` or `--ease-bounce` or the rest of the 24-curve table. Could the gamut-director skill nudge "use at least one in-out or bounce somewhere" to push variety further?

## Findings for the agent loop

- Strong. No prompting changes needed for this brief shape. The agent read SKILL.md, planned, parallelized, recovered cleanly, and shipped under budget. If anything, the `notes.md` self-report is more rigorous than the `verdict.md` we usually write — worth keeping `notes.md` as a permanent output requirement.

## Frame thumbnails (sampled at 1s, 4s, 7s, 10s)

Extracted via `ffmpeg -y -ss <t> -i workdir/commercial.mp4 -frames:v 1 frames/sample-<t>s.png`:

- `/Users/shinyobjectz/Apps/workbooks/packages/gamut/evals/runs/2026-05-19-tree-runner-real-paid/frames/sample-1s.png` — Scene 1 (rooftop, "A place, not a product." in Bodoni Moda over a Marrakech golden-hour skyline; subtle right-bottom uppercase kicker).
- `/Users/shinyobjectz/Apps/workbooks/packages/gamut/evals/runs/2026-05-19-tree-runner-real-paid/frames/sample-4s.png` — Scene 2 (souk, dappled light on brass dishes of spices; JetBrains Mono HUD overlays in four corners — NOTE 01 CARDAMOM / NOTE 02 NEROLI / NOTE 03 SANDALWOOD / ● single distillation).
- `/Users/shinyobjectz/Apps/workbooks/packages/gamut/evals/runs/2026-05-19-tree-runner-real-paid/frames/sample-7s.png` — Scene 3 (tannery, stacked saffron + oxblood leather, steam; huge "MARRAKECH" knockout in mix-blend-mode: difference — the type renders as inverted cyan/blue against the warm leather, exactly the canonical idiom).
- `/Users/shinyobjectz/Apps/workbooks/packages/gamut/evals/runs/2026-05-19-tree-runner-real-paid/frames/sample-10s.png` — Scene 4 (arched alley at dusk, "AESOP · EST 1987" chevron-clipped seal at top, "FIND IT ON AESOP.COM" wide-tracked whisper CTA at bottom with pill).

Also extracted 12 frames at 1fps in the same directory (`frame-001.png` … `frame-012.png`); file-size variation across them (700kB → 1.34 MB → 920kB) confirms genuine scene changes, not a static lockup.

## Headline

**18 / 18.** This is the best gamut eval run on record so far. The agent did exactly what the brief stress-tested for: it reached for the freeform palette deliberately, used clip-path / mix-blend-mode / extended eases / inline `<video>` / top-level `<audio>` / typographic variety in earnest, and produced a commercial that reads as editorial mid-century print rather than AI-default. The two product gaps it surfaced (crossfade shader bug, brief.check strictness) are real and worth filing.
