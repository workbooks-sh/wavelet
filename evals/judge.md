# wavelet eval rubric

Score each run on six dimensions. Each is 0–3:

- **0** — not attempted / failed
- **1** — partial / wrong shape but trying
- **2** — works but with issues a reviewer would call out
- **3** — clean, indistinguishable from a careful human

## Dimensions

### 1. Stage coverage (0–3)

Did the agent touch every stage of the pipeline that the brief
implied? Walk `wavelet workflow run <pipeline> --workdir <run>/workdir`.

- 0: < 3 stages produced any artifact
- 1: 3–5 stages, gaps in the middle (skipped velocity or storyboard verify)
- 2: 6–7 stages, only the optional ones missed (transitions, captions)
- 3: every stage in the YAML reports `status: complete`

### 2. Tool selection (0–3)

Did the agent invoke the *right* wavelet subcommand at each stage?
Walk `trace.wavelet.jsonl` and check the verb sequence against
`wavelet-director/SKILL.md`'s "The pipeline (run in order)" section.

- 0: random verb order, missing core verbs (e.g. did stills with
  `txt2img` instead of `image scene-still`)
- 1: rough order right but extras / wrong-cluster calls
- 2: correct verbs, wrong sequence (e.g. ran storyboard before velocity)
- 3: matches the SKILL.md order; uses the right cluster for each
  artifact (scene-still vs txt2img, img2vid vs txt2vid, etc.)

### 3. Budget discipline (0–3)

Sum `cost_estimate_usd` across `trace.wavelet.jsonl`. Compare to the
brief's ceiling. Inspect for missing `--max-cost`.

- 0: no `--max-cost` flag anywhere; overspends
- 1: some `--max-cost`, blew the ceiling
- 2: every paid call has `--max-cost`; came in under
- 3: every paid call has `--max-cost`, came in well under, used
  `--dry-run` for sanity sweeps where the brief allowed

### 4. Error recovery (0–3)

Read `transcript.log` for failures. Did the agent rescue itself, or
ask for help, or quietly succeed-by-not-trying?

- 0: hit an error, kept marching, produced garbage
- 1: hit an error, gave up early
- 2: hit an error, retried (sometimes counter-productively)
- 3: hit an error, diagnosed, took a different path that worked

### 5. Final artifact (0–3)

Open `commercial.mp4`. Does it look like the brief asked?

- 0: file missing / 0 bytes / unplayable
- 1: file plays but doesn't match the brief (wrong product, wrong tone)
- 2: file plays, broadly matches, has rough edges
- 3: file plays, matches the brief, would be acceptable as a draft cut

### 6. Documentation use (0–3)

Did the agent read `SKILL.md` and pipeline YAML before acting? Walk
`trace.tool-calls.jsonl` for early `Read` calls on those paths.

- 0: never read either
- 1: read partial / late
- 2: read SKILL.md at the start
- 3: read SKILL.md, opened the right sections at the right times,
  consulted `wavelet pipelines show commercial` for stage criteria

## Total

Sum / 18. The rubric isn't a leaderboard — it's a diagnostic. Look at
which dimension is low and that's the next thing to fix in either
wavelet or the skill.

## Where to look

Pull these from a run directory before scoring:

```bash
# stage coverage
wavelet workflow run commercial --workdir runs/<id>/workdir --text

# tool selection sequence
jq -r '.argv | join(" ")' runs/<id>/trace.wavelet.jsonl

# budget
jq -s 'map(.argv | join(" ")) | length' runs/<id>/trace.wavelet.jsonl

# what the agent thought (its own notes.md, if it wrote one)
cat runs/<id>/workdir/notes.md
```
