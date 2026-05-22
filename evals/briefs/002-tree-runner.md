# Tree Runner — full commercial eval

You are a creative director. Use the wavelet CLI (already on your PATH)
to produce a 15-second AI commercial for the brief below. This is the
**full pipeline** exercise — research, screenplay, velocity, storyboard,
asset gen, edit, compose, publish (the eight stages declared in
`packages/wavelet/pipeline_defs/commercial.yaml`).

## Brief (9-line)

PRODUCT: Allbirds Tree Runner sneakers
AUDIENCE: 28-40 urban professionals who walk more than they run
INSIGHT: "Sustainable" usually means uncomfortable or ugly
PROMISE: All-day comfort that happens to be made from trees
PROOF: Eucalyptus-fiber upper + sugarcane sole, machine washable
TONE: understated
MUSIC: acoustic minimal → warm indie-folk swell
CALL: Try them barefoot
RUNTIME: 15

## Hard constraints

- Total spend ceiling: **$3.00 USD**. Pass `--max-cost` on every paid call.
- Use **dry-run** for every shot/scene-still/music/i2v call. Live calls
  are only required at `wavelet render` (local CPU render, no spend).
- The point is exercising the pipeline shape end-to-end, not paying for
  every paid stage.

## What to produce

The eight stages of `pipeline_defs/commercial.yaml`, in order:

1. `brief.md` (research)
2. `script.fountain` + `screenplay.json` (script)
3. `velocity.json` (velocity)
4. `storyboard.json` + `transitions.json` (storyboard)
5. `music/track.wav` + per-shot scene-stills + per-shot i2v clips (asset, dry-run OK)
6. `cuts.edl` + `captions.json` (edit)
7. `comp.json` (compose)
8. `commercial.mp4` (publish — live render)

Then `wavelet workflow run commercial --workdir .` to confirm the
state-machine says you're done.

## Where to learn

`packages/workbooks/skills/wavelet-director/SKILL.md` — the canonical recipe.
Read it carefully before you start.

`wavelet --help` lists every subcommand; each has its own `--help`.

`wavelet pipelines show commercial` prints the per-stage tools + success
criteria as JSON or YAML.

## Success criteria

- Every stage in `wavelet workflow run commercial --workdir .` reports
  `status: complete`.
- `commercial.mp4` exists, plays, and matches the brief's tone.
- Total paid spend ≤ $3.00.
- No `error` exits in `trace.wavelet.jsonl` (warnings are fine).
