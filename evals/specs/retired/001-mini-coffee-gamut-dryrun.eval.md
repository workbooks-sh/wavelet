---
name: wavelet/001-mini-coffee-wavelet-dryrun
agent: wavelet.commercial
timeoutMs: 1800000
turns:
  - action:
      kind: wavelet.commercial
      brief: packages/wavelet/evals/briefs/001-mini-coffee.md
      agent: wavelet
      budget_usd: 0.05
      dry_run: true
      pipeline: commercial
    checks:
      - kind: wavelet.workflow_complete
        workdir: ctx:gamutWorkdir
        pipeline: commercial
---

# wavelet/001-mini-coffee-wavelet-dryrun

First-light eval for the Gemini-3.5-native `wavelet agent` driver
(`wavelet agent run`). Mirrors `001-mini-coffee-codex-dryrun` but spawns
the built-in agent loop instead of Codex CLI, so we can see how the
in-process Gemini-3.5-Flash agent navigates the wavelet tool surface
under a $0.05 dry-run ceiling.

Gate: `wavelet.workflow_complete` only — we drop the MP4 / cost / rubric
gates from the codex variant for now. Dry-run currently does NOT emit
placeholder MP4s (bd: wb-tgkq), so requiring an MP4 would always fail
for this driver too; that gate returns when wb-tgkq lands.

Brief: [`packages/wavelet/evals/briefs/001-mini-coffee.md`](../briefs/001-mini-coffee.md).
