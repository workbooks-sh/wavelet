# wavelet evals — pipeline evaluation harness

Run a fresh coding agent against a creative brief and watch how it uses
`wavelet` to produce a commercial. Captures every CLI invocation, every
file write, every cost gate, every stage transition — so the grader
(you or me) can answer:

- Did the agent **plan** the pipeline correctly? (research → script →
  velocity → storyboard → asset → edit → compose → publish, per
  `vendor/workbooks/skills/wavelet-director/SKILL.md`)
- Did it **call the right wavelet subcommands** in the right order?
- Did it **honor the budget** (`--max-cost` on every paid call)?
- Where did it **get stuck** or **rescue itself**?
- What **artifacts** did it actually produce?

## Layout

```
evals/
├── README.md              # this file
├── bin/
│   └── wavelet-traced       # PATH shim — logs every wavelet invocation
├── briefs/
│   ├── 001-mini-coffee.md       # smallest-possible exercise (1 shot, 5s)
│   ├── 002-tree-runner.md       # full 9-line brief, 10-15s spot
│   └── 003-freeform-palette.md  # palette-stress brief (12s, $5)
├── specs/                  # workbook-eval .eval.md specs
│   ├── 001-mini-coffee.eval.md
│   ├── 002-tree-runner.eval.md
│   └── 003-freeform-palette.eval.md
├── judge.md               # 6-dimension rubric (referenced from specs)
├── verdict-template.md    # form to copy into a run's verdict.md (manual mode)
└── runner.sh              # legacy shell runner — still works for ad-hoc work
```

## Running via `workbook-eval` (canonical)

```bash
node vendor/workbooks/packages/workbook-cli/bin/workbook-eval.mjs \
  packages/wavelet/evals/specs/001-mini-coffee.eval.md
```

Each spec drives `wavelet.commercial` (the action that spawns the agent
with `bin/wavelet-traced` on PATH) and gates the result with:

- `wavelet.video_renders` — MP4 plays at expected duration/codec
- `wavelet.cost_below` — `cost_estimate_usd` across trace under brief ceiling
- `wavelet.workflow_complete` — every pipeline stage reports `complete`
- `wavelet.palette_uses` (003 only) — required CSS/HTML features present
- `wavelet.frame_probe` (003 only) — pixel sample asserts visible content
- `rubric.passes` — vision-mode codex judge against the six-dimension rubric,
  fed three frames sampled from the rendered MP4

`runner.sh` still works for one-off ad-hoc runs without the full eval
machinery.

## Running one

```bash
cd packages/wavelet/evals
./runner.sh \
  --brief briefs/001-mini-coffee.md \
  --run-id 2026-05-19-a-mini-claude \
  --agent claude \
  --budget 0.50
```

Output lands in `runs/<run-id>/`:

```
runs/2026-05-19-a-mini-claude/
├── workdir/                  # the agent's workspace
│   ├── brief.md
│   ├── script.fountain       # whatever the agent produced
│   ├── storyboard.json
│   ├── comp.json
│   └── commercial.mp4
├── transcript.log            # raw stdout/stderr from the agent process
├── trace.wavelet.jsonl         # every wavelet invocation: args + duration + exit + cost
├── trace.tool-calls.jsonl    # every tool call the agent made (Claude stream-json)
├── workflow.json             # final `wavelet workflow run commercial.yaml --workdir workdir`
└── verdict.md                # YOU fill this out (or I do)
```

## Agents

Two are wired by default:

- `--agent claude` — spawns `claude -p <brief> --output-format stream-json`
  in a subprocess. Clean context (no memory bleed from the parent session).
  Default.
- `--agent codex` — spawns `codex exec <brief>` for a different model
  family entirely. Maximum separation.

Adding a third agent is one shell-script arm in `runner.sh`.

## The trace

`bin/wavelet-traced` is a one-page bash script that prepends itself to
`PATH` and records every `wavelet` invocation:

```jsonl
{"ts":"2026-05-19T12:34:56Z","argv":["wavelet","brief","check","brief.md"],"duration_ms":42,"exit":0,"stdout_bytes":12,"stderr_bytes":0}
{"ts":"2026-05-19T12:35:01Z","argv":["wavelet","screenplay","parse","script.fountain"],"duration_ms":18,"exit":0,"stdout_bytes":4291,"stderr_bytes":0}
```

That gives you a clean machine-readable history independent of whichever
agent is driving — you can grep, diff, replay, or pipe into a grader.

## Grading

After a run completes, copy `verdict-template.md` into the run dir as
`verdict.md`, fill it in. Or feed `trace.wavelet.jsonl` + `transcript.log`
to me and I'll grade it.

The judge rubric (`judge.md`) covers: stage coverage, tool selection,
budget discipline, error-recovery, final-artifact viability.
