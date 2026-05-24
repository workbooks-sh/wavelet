# Eval workflow — launch, monitor, kill, post-mortem

Standard process for running wavelet `*.eval.md` specs with live
telemetry and a kill switch. Built after the 008 post-mortem surfaced
the agent looping silently on false-positive lint findings with no
visibility into what was happening.

## The four scripts

All live in `packages/wavelet/evals/bin/`:

| Script | Role |
|---|---|
| `eval-run <spec.eval.md>` | Launch a spec in the background. Records PID + workdir + log paths under `evals/runs/_logs/<short>.{pid,workdir,log}`. Prints `<short> <pid> <log> <workdir>`. |
| `eval-watch <short>` | Stream curated trace events. One line per failure, paid generation, or stage transition. Lint repeats deduped to every 5th call. Designed to be wrapped by Claude Code's Monitor tool. |
| `eval-status <short>` | One-shot snapshot. Process alive/dead, latest 6 trace calls, gate-relevant call summary (screenplay validate exit, lint --mp4 status, render exit, shot txt2vid count), final MP4 size. |
| `eval-kill <short>` | SIGTERM the workbench + all claude-p children. Workdir preserved. SIGKILL after 5s if not exited. |

## Recommended launch sequence

```bash
# 1. Build the binary if you haven't (eval-run will if needed)
cd packages/wavelet && cargo build -p wavelet

# 2. Launch the eval
./evals/bin/eval-run evals/specs/008-new-balance-acrylic.eval.md
# prints: 008-new-balance-acrylic 15590 evals/runs/_logs/008-...-T151125.log /path/to/workdir

# 3. From inside Claude Code, arm a Monitor on the curated stream
Monitor(
  description: "008 eval — failures, spend, stage transitions",
  persistent: true,
  command: "./evals/bin/eval-watch 008-new-balance-acrylic"
)
# Notifications arrive in chat: one per emitted line.

# 4. While running, snapshot whenever you want a full picture
./evals/bin/eval-status 008-new-balance-acrylic

# 5. Kill if it's stuck or you see a spend pattern you want to stop
./evals/bin/eval-kill 008-new-balance-acrylic
```

## Why these specific signals

`eval-watch` emits on:
- **Any non-zero exit** — every failure path. Never silent on crashes.
- **`shot txt2vid`** — every paid generation. Watch for unintended
  spend or runaway variant rolling.
- **`render`, `verify`, `screenplay validate`, `workflow run`** — stage
  transitions. Tell you where the agent is in the pipeline.
- **`lint`** — every change in exit code OR every 5th repeat OR if
  > 60s elapsed since previous event. Catches the "stuck in lint
  fix loop" pattern from 008 without spamming on each retry.

Filter is wider than "happy path." A monitor that emits only on
success is silent on a crashloop, which looks identical to "still
running." Every terminal state has a signal here.

## Post-mortem checklist

After a fail, check in order:

1. **Run `eval-status <short>`** — what state did the run end in?
2. **Check trace for gate-relevant calls** — already in eval-status
   output. Did `screenplay validate` run? `lint --mp4`? `render`?
3. **Look at the lint output on the final HTML** — re-run
   `wavelet lint commercial.html --platform <p> --mp4 commercial.mp4`
   in the workdir. With `WAVELET_LINT_DEBUG_HALO=1` for per-element
   contrast measurements.
4. **Read the workbench log** — look for "criteria_failed" lines that
   pinpoint the failed gate.
5. **Inspect the MP4** if it exists — extract frames at the rubric
   sample times, view them.

## Discipline issues caught by 008

- Agent wrote `comp.json` directly (legacy path) → fixed in commit
  `9ec2019ee` with `WAVELET_STRICT_HTML=1` rejection at render time
  and pre-render trace check.
- Agent skipped `lint --mp4` post-render pass → tightened
  `WaveletLintPasses` validator to require `--mp4` in argv.
- Lint surfaced false-positive contrast errors on legitimate
  black-on-white text → fixed in commit `af95eba90` with
  luminance-quartile class separation.

## Beads tracking

Each non-trivial finding from an eval run should become a `bd` issue.
The eval workflow itself is `bd` issue `wb-pj4k`. Sub-issues track
specific lint / pipeline improvements.
