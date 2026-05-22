# Mini coffee — smallest-possible eval

You are a creative director. Use the wavelet CLI (already on your PATH)
to produce a single 5-second coffee-product shot. This is a SANITY
test, not a full commercial — the point is to exercise the pipeline
end-to-end on the cheapest possible path.

## Brief (9-line)

PRODUCT: A generic ceramic pour-over coffee carafe (no brand)
AUDIENCE: Specialty-coffee enthusiasts who care about brew ritual
INSIGHT: Most ads show coffee being chugged; the ritual is the product
PROMISE: Slow brewing turns morning into a moment
PROOF: Hand-poured, hand-held, hand-watched
TONE: still, observational, A24
MUSIC: ambient piano, single phrase
CALL: Pour better
RUNTIME: 5

## Hard constraints

- Total spend ceiling: **$0.50 USD**. Pass `--max-cost` on every paid call.
- Use **dry-run** mode on every `wavelet shot txt2vid|img2vid`, `wavelet image scene-still`,
  and `wavelet music gen` for this sanity test. The point is exercising the
  pipeline shape, not paying for one shot.
- For the final render (`wavelet render`), live calls are fine — render is
  local (no API spend) and produces an MP4.

## What to produce

In your working directory:

1. `brief.md` — the 9-line above (you may copy it verbatim)
2. `script.fountain` — a one-scene fountain screenplay
3. `screenplay.json` — `wavelet screenplay parse` output
4. `velocity.json` — `wavelet velocity propose` output
5. `storyboard.json` — `wavelet storyboard plan` output
6. `comp.json` — your composition manifest
7. `commercial.mp4` — the final render

When you're done, write a one-paragraph note to `notes.md` describing
what went well, what surprised you, and what you'd do differently.

## Where to learn

The canonical recipe is at
`packages/workbooks/skills/wavelet-director/SKILL.md` (relative to the repo
root). Read it before you start — it covers the 9-line brief shape,
fountain conventions, the stage order, the negative-prompt defaults,
and the C2PA signing step.

`wavelet --help` lists every subcommand. Every subcommand has its own
`--help`.

## Success criteria

- `commercial.mp4` exists, is non-zero size, and plays.
- `wavelet workflow run commercial --workdir .` (where `commercial` is
  the pipeline name at `packages/wavelet/pipeline_defs/commercial.yaml`)
  reports every stage as complete.
- Total paid spend ≤ $0.50.
