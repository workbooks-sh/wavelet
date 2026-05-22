# Verdict — 2026-05-19-b-mini-claude

**Brief:** `briefs/001-mini-coffee.md`
**Agent:** claude (Claude Code subprocess, fresh context, permissions bypassed)
**Started:** 2026-05-19T18:18:43Z
**Finished:** 2026-05-19T18:22:01Z
**Wall time:** 3m 18s
**Paid spend:** $0.00 (all paid calls were `--dry-run`)
**Agent total cost (Anthropic API):** ~$2.00 (driving the agent itself)

## Scores (0–3 per dimension; see judge.md)

| Dimension          | Score | Notes |
|--------------------|-------|-------|
| Stage coverage     | **3** | all 8 stages report `status: complete` in `gamut workflow run commercial` |
| Tool selection     | **3** | verb sequence matches SKILL.md "The pipeline (run in order)" exactly: brief check → screenplay parse → velocity propose → storyboard plan/verify → continuity check → transitions classify → music gen → shot txt2vid → velocity validate (against music) → onsets-to-edl → dialogue captions → verify → render → c2pa sign → c2pa verify |
| Budget discipline  | **3** | `--dry-run` on every paid surface (music gen, shot txt2vid). No `--max-cost` flag on most calls but no spend either; the brief asked for dry-run, agent honored it |
| Error recovery     | **3** | hit two errors and diagnosed both without help: (1) `brief check` rejected the brief with the constraints section attached — agent trimmed to the 9 slot lines and re-ran; (2) `velocity validate` exit=1 — agent noted and proceeded since it's a soft gate |
| Final artifact     | **2** | `commercial.mp4` (200 KB) plays. Content is a placeholder solid-color MP4 from ffmpeg, not a real gen — that's faithful to the brief's "dry-run for paid calls" constraint. For a real run the placeholder would become a real Wan-i2v / Seedream output |
| Documentation use  | **3** | first `Read` calls in `trace.tool-calls.jsonl` are `SKILL.md` (line ranges 1–250 then 486–773 then 769–913), then `commercial.yaml`. Then started executing. Pulled `gamut brief check --help` / `gamut pipelines show --help` / etc. when needed |
| **Total / 18**     | **17** | |

## What worked

- The harness captured 29 gamut invocations in `trace.gamut.jsonl` with
  per-call durations + exit codes — clean grep / diff / replay surface.
- The agent read `SKILL.md` *before* acting and followed the 8-stage
  order verbatim. The stage advancement reads almost like a textbook
  example of how to use gamut.
- C2PA sign+verify worked end-to-end including the post-mux re-sign
  the SKILL.md callout warns about.
- `gamut workflow run commercial` correctly reported all 8 stages
  complete at the end — the cooperative state-machine pattern (wb-oemp)
  did its job as a self-grading hook.

## What broke

- **`gamut brief check` is stricter than the brief file format we ship
  in `evals/briefs/`.** The eval briefs include a "Hard constraints"
  prose section after the 9 slot lines; `brief check` returns exit=1
  with "unknown slot" on those lines. Workaround the agent found:
  copy the 9 slot lines into a separate file before calling
  `brief check`. **Real bug** — the parser should tolerate trailing
  prose or the eval briefs should split the brief from the constraints.

- **`--dry-run` doesn't write placeholder files.** `gamut music gen
  --dry-run` and `gamut shot txt2vid --dry-run` emit the request spec
  to stdout but don't produce a file at the cache path. The pipeline's
  `artifact_exists` gates (music.wav, shots/) then fail to advance.
  The agent papered over with `ffmpeg -f lavfi -i anullsrc -t 5
  music.wav` and a solid-color MP4. **Real UX gap** — dry-run should
  write a zero-content placeholder of the right shape so workflow
  graphs run end-to-end on $0.

- **`gamut velocity validate` exited 1.** The agent didn't dig into
  why — it's a soft gate and the workflow runner counts the artifact's
  presence, not its grade. Worth probing: is the canned 5s ambient
  music validation actually wrong, or is the gate misconfigured?

## Surprises

- The agent treated `--dry-run` as a sanity hint and aggressively
  reached for `ffmpeg` to bridge the placeholder gap. That's clever
  but it's a workaround for a real product gap — surface that gap as
  a feature request, don't normalize the workaround.
- Time to first gamut call was about 30s — entirely spent reading
  SKILL.md + commercial.yaml. Good behavior; suggests SKILL.md is
  doing its job as the canonical entry point.
- Cost per eval: ~$2 in Anthropic API spend to drive the agent. Cheap
  enough to run on every PR. The actual gamut surface paid $0 because
  dry-run was honored.

## Findings for gamut

1. **`brief check` should accept briefs with prose suffixes.** Or:
   change the parser to skip lines that aren't `KEY: value`-shaped
   instead of erroring on them. Or: add a `--strict` flag and default
   to lenient.

2. **`--dry-run` should write placeholder files at the cache path.**
   `gamut music gen --dry-run --duration 5` → write a 5s silent WAV.
   `gamut shot txt2vid --dry-run --duration 5` → write a 5s solid
   black MP4. The placeholder lets workflow gates advance; the
   request hash is already computed so on the next paid run the
   placeholder is replaced by the real asset.

3. **`gamut velocity validate` failure mode unclear.** Inspect the
   trace's stderr — currently we only capture stdout/stderr bytes,
   not content. Consider extending `gamut-traced` to keep stderr
   bodies for non-zero exits so the grader can diagnose without
   re-running.

## Findings for the agent loop

1. The agent's first move was to read SKILL.md. Confirms that placing
   SKILL.md in the workdir (rather than relying on `--add-dir` for the
   monorepo path) is the right harness move — the agent treats the
   workdir as the world.

2. The agent never invoked `gamut director synthesize` (LLM-as-director).
   That's fine for a 1-shot brief, but for the full Tree Runner brief
   it would be a real omission. Worth checking whether SKILL.md surfaces
   that step prominently enough.

3. The agent didn't run `gamut workflow run` to **plan** ahead of
   acting — it ran it incrementally to check progress. Both are valid
   patterns; documenting "start with `workflow run` to see the stage
   list" in SKILL.md might bias planning earlier.

## Next runs to do

- Run the same brief on Codex (`--agent codex`) to compare cross-model
  behavior. If Codex follows the same SKILL.md order, the recipe is
  model-agnostic; if it diverges, the prompt needs more scaffolding.
- Run the full Tree Runner brief (`002-tree-runner.md`) to see how the
  agent handles a 15s spot with reference images, scene-stills, and
  multi-scene compose.
- Once `brief check` + `--dry-run` placeholder are fixed, rerun this
  brief and expect zero ffmpeg workaround.
