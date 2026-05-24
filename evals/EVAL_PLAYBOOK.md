# Wavelet eval — iterative development playbook

Complete operational reference for working the `*.eval.md` specs through
failure → diagnosis → fix → re-run cycles. Read `EVAL_WORKFLOW.md` first
for the four scripts; this file covers everything you need to actually
move through failures rather than just watch them.

---

## 1. Common failure patterns

### 1.1 Gate-mismatch — agent did A, gate wants B

**What it is.** A gating criterion checks that the agent invoked the right
tool in the right way. If the agent invoked a correct-looking tool but
without the required flag, or against the wrong target, the gate fails with
a structured reason code rather than "failure unknown."

**How to recognize from eval-watch.**  
`wavelet workflow run` exits 0 but with shrinking `stdout_bytes` on each
call, then the eval harness emits:

```
criteria_failed
```

in the workbench log. `eval-status <short>` will show:

```
screenplay validate: 0
lint --mp4: None       ← the tell
render: 0
shot txt2vid calls: 7
```

**Root cause (008 run, 2026-05-23T15:11:25).** The 008 agent ran
`wavelet lint commercial.html --platform tiktok` 16 times in a row without
ever adding `--mp4 commercial.mp4`. The `wavelet_lint_passes` gate requires
the lint invocation to include `--mp4` so the post-render contrast pass
(which operates on composited pixels, not static HTML) actually runs.
The gate returned reason `missing_mp4_postrender_lint` on every retry.

**Fix lever.** The agent's retry prompt reads the `hint` field in the
`FailedCriterion` detail. If the hint is not surfaced visibly to the agent,
it will keep retrying the wrong form. Ensure the skill's compose-stage
closing instruction reads:

```
wavelet lint commercial.html --platform <platform> --mp4 commercial.mp4
```

not just:

```
wavelet lint commercial.html --platform <platform>
```

**Don't confuse with.** A gate-mismatch where the agent ran the correct
command form but the lint itself found real errors (reason `lint_nonzero_exit`).
That is not a gate-mismatch — it is a real defect in the composition.

---

### 1.2 False-positive lint — rule too aggressive

**What it is.** `wavelet lint` exits 1 on content that is visually fine;
the agent cannot fix the finding because there is nothing to fix.

**How to recognize.** The agent repeatedly modifies the HTML around the
flagged element, re-runs lint, and the same finding re-appears with the
same byte count in `stdout_bytes` despite the edits. From eval-watch:

```
T+12s  x1   8241ms  lint commercial.html --platform tiktok
T+10s  x1   8030ms  lint commercial.html --platform tiktok
T+09s  x1   8166ms  lint commercial.html --platform tiktok
```

All same exit code, similar stdout byte counts — the errors are not
changing even though the agent is editing.

**Ground-truth test.** Re-run lint in the workdir with debug output:

```bash
cd <workdir>
WAVELET_LINT_DEBUG_HALO=1 wavelet lint commercial.html \
  --platform tiktok --mp4 commercial.mp4 2>&1 | less
```

The debug output prints per-element halo measurements:

```
halo-debug bbox=(540,1700) 540x220  font=64px halo=12.0px  glyphs=12
  pixels=2880 q=3  dark_mean=0.020 light_mean=0.820  ratio=21.43
```

A WCAG ratio > 4.5 (AA) or > 7.0 (AAA) indicates the element passes
contrast requirements. If the ratio is high but lint still flags it, the
rule is over-firing (likely a false quartile split — see commit `af95eba90`
for the luminance-quartile fix that addressed an earlier version of this).

**Fix lever.** File a bd issue with the specific element, computed WCAG
ratio, and platform. The fix belongs in
`src/lint/text_readability_contrast.rs` or the rule's scoring function —
do not soften the lint rule in the spec or skill without a regression test.

---

### 1.3 Veo rate-limit / quota — parallel calls failing fast at ~300ms

**What it is.** Multiple `wavelet shot txt2vid` calls launched in parallel
all fail within 300–500ms with exit 2 and `stderr_bytes=368` (or similar
small error envelope). This is the Veo API rejecting before any generation
work begins — quota exhausted, concurrent call limit exceeded, or API key
misconfigured.

**How to recognize.** Cluster of sub-500ms exit-2 txt2vid calls in
eval-watch output:

```
T+00s  x2    315ms  shot txt2vid Macro close-up: a thick...
T+00s  x2    311ms  shot txt2vid Four floating droplets...
T+00s  x2    287ms  shot txt2vid Hero shot: the fully scul...
```

Three parallel calls all failing in ~300ms. (Real Veo generations take
30–55 seconds.) Subsequent retries succeed after the agent switches to
serial calls or waits.

**Root cause (008 run, 2026-05-23T15:16:31–39).** The agent issued 3 Veo
calls in parallel for shots 05, 06, 07. All three failed at exit 2 within
315/311/287ms — Veo's parallel-call limit rejection. The agent correctly
retried them serially and all three succeeded.

**Fix lever.** This is a retry the agent should handle automatically. If
the agent is not retrying: update the skill instruction to call
`shot txt2vid` serially (one at a time) unless the spec explicitly says
to pipeline them. The spec `budget_usd` cap must account for retry spend.
If you see dozens of rapid failures: check `~/.config/wavelet/env` for
`GOOGLE_API_KEY` / `FAL_KEY` and verify quota at the provider dashboard.

**Also watch for.** Wrong backend name (exit 3, ~40ms) — the agent tried
`fal-kling-2.5`, `kling-2.5`, `fal-veo-3` (all unregistered backends,
exit 3). These are capability probes the agent makes when its known
backend list is stale; they fail fast without spending budget.

---

### 1.4 Lint loop — agent stuck repeatedly failing the same lint check

**What it is.** The agent is running lint, reading the findings, making
HTML edits, and re-running lint — but the findings are not clearing because
the edits address the symptom (CSS property) rather than the root cause
(layout structure, missing safe-zone margin, wrong halo width).

**How to recognize.** `eval-watch` shows a long sequence of lint failures
with *changing* `stdout_bytes` (the agent is making progress on some
errors) but then plateaus — byte count stops decreasing:

```
T+12s  x1   8441ms  lint commercial.html  [stdout_bytes=10462]
T+10s  x1   8050ms  lint commercial.html  [stdout_bytes=10462]  ← same
T+08s  x1   8166ms  lint commercial.html  [stdout_bytes=2819]   ← progress
T+09s  x1   8031ms  lint commercial.html  [stdout_bytes=2819]   ← plateau
T+08s  x1   8248ms  lint commercial.html  [stdout_bytes=2819]   ← stuck
```

From the 008 run: 19 total lint calls with exit 1 against `commercial.html`
before the run ended, never achieving a passing `--mp4` lint.

**Why it persists.** The `wavelet_lint_passes` gate requires:
1. `lint` called with `commercial.html` (or `scenes/`) in argv, AND
2. `--mp4` in argv, AND
3. exit 0.

If the agent is stuck on a halo-contrast finding that won't clear, it
is making CSS changes that do not affect the composited MP4 pixels. The
`--mp4` flag tells lint to re-render the HTML overlay onto the actual video
frames and measure contrast there — pure HTML changes that look fine on a
white browser background may still fail when overlaid on dark video.

**Fix lever.**
1. Kill the run with `eval-kill <short>`.
2. Re-run lint with `WAVELET_LINT_DEBUG_HALO=1` to get per-element WCAG
   ratios (see §1.2 above for the command).
3. Decide: real defect (improve the overlay's CSS contrast) or
   false-positive (file a bd issue for the lint rule).
4. If real: the fix is usually increasing the text shadow / halo blur
   radius in the scene's HTML, not adjusting the layout.
5. Update the skill's compose-stage instructions with the correct CSS
   pattern; re-fire the eval.

---

### 1.5 Discipline drift — agent picks legacy path despite skill instructions

**What it is.** The agent produces `comp.json` directly (Write tool or
manual JSON construction) rather than following the `commercial.html` →
`scenes/*.html` canonical workflow. The agent then renders `comp.json`
rather than `commercial.html`, bypassing all HTML-path lint coverage.

**How to recognize.** `eval-status` shows `comp.json` in the workdir
alongside `commercial.html`:

```bash
ls <workdir>
# comp.json  commercial.html  commercial.mp4  ...
```

Or from the trace: `wavelet render comp.json` instead of
`wavelet render commercial.html`.

**Gate behavior.** The eval shim sets `WAVELET_STRICT_HTML=1` in the
environment. When that env var is set, `wavelet render comp.json` exits 3:

```
wavelet render: REJECTED — WAVELET_STRICT_HTML=1 is set.
Write a `commercial.html` manifest ...
```

Without `WAVELET_STRICT_HTML=1` (ad-hoc CLI use), it warns on stderr but
proceeds. The gate prevents budget burn on the wrong path; outside eval,
legacy callers still work.

**Fix lever.** If the agent still produces `comp.json` despite the STRICT
rejection: the skill instruction needs to explicitly state that writing
`comp.json` with the Write tool is not part of the workflow and will be
rejected. Update the compose-stage instructions in the skill.

---

### 1.6 Cost runaway — variant rolling burns budget

**What it is.** The agent runs multiple `wavelet shot txt2vid` calls per
shot slot (trying variants, regenerating unsatisfactory clips) such that
the total spend exceeds the spec's `budget_usd` cap before the spot is
complete.

**How to recognize.** `eval-status` `shot txt2vid calls` count is much
higher than the planned shot count (e.g. 14 txt2vid calls for a 7-shot
spot means 2x per slot average). `wavelet.cost_below` check fails.

**Fix lever.** The `wavelet shot txt2vid` `--max-cost` per-call cap limits
each individual call's spend. But the spec `budget_usd` is the aggregate
cap; the `wavelet.cost_below` check reads the trace to sum all costs.
When variant rolling is burning budget:
1. Add explicit `--max-cost` flags in the skill instructions (the agent
   should not exceed $0.60 per shot on the default Veo backend).
2. Reduce `max_revisions_per_stage` in the pipeline's orchestration block
   to hard-stop the retry budget.
3. Increase the spec `budget_usd` if the brief legitimately needs more
   shots — but match it in both the spec-level `budget_usd` and the
   action-block `budget_usd` field (they are distinct; the action-block
   value is what `wavelet.cost_below` reads via `ctx:waveletTrace`).

---

### 1.7 Resolution mismatch — agent generates landscape instead of portrait

**What it is.** The spec declares `width: 1080  height: 1920` (9:16
portrait) but the rendered MP4 is 720×1280 or 1920×1080 because the
agent's `wavelet storyboard plan` call omitted `--aspect 9:16` (or used
the wrong flag).

**How to recognize.** The `wavelet.video_renders` check fails:

```
wavelet.video_renders: width mismatch — expected 1080, got 720
```

Multiple consecutive 005 runs failed this way (2026-05-22T23:07:50,
2026-05-22T23:31:51, 2026-05-23T00:26:15, 2026-05-23T00:30:32). The
storyboard in those runs had `"resolution": [1920, 1080]` (landscape)
instead of `[720, 1280]` or `[1080, 1920]`.

**Fix lever.** Ensure the skill's storyboard-stage instructions include
`--aspect 9:16 --match-runtime <secs>`. The 005 run that eventually
passed (2026-05-23T01:36:46) used:

```
wavelet storyboard plan script.fountain --velocity velocity.json \
  --aspect 9:16 --match-runtime 12 --pretty -o storyboard.json
```

Check the spec's `width`/`height` before firing. If they are portrait
(height > width), the skill must pass `--aspect 9:16` to storyboard plan.

---

### 1.8 Binary missing — `spawn wavelet ENOENT`

**What it is.** The eval shim cannot find the `wavelet` binary.
`wavelet.workflow_complete` check fails with:

```
wavelet.workflow_complete: wavelet exited -1
stderr: spawn wavelet ENOENT
```

**How to recognize.** From 005 run 2026-05-22T10:18:27:

```
wavelet.workflow_complete: wavelet exited -1
stderr: spawn wavelet ENOENT
```

**Fix lever.** Run `cargo build -p wavelet` in `packages/wavelet/` before
firing the eval. `eval-run` does this automatically if
`target/debug/wavelet` is absent, but not if a stale binary exists.
After any source change that affects the evaluated behavior, rebuild first.

---

### 1.9 Timeout without criteria_failed

**What it is.** The eval harness hits `timeout_ms` before the agent
finishes, with no gate-related error message.

**How to recognize.**

```
wavelet.commercial: timed out (2 retries used; last error: attempt timed out after 600000ms)
```

This appeared in 005 runs 2026-05-22T16:31:51-ugc and 2026-05-22T16:37:09
(ugc variants). The agent was generating shots but the total elapsed time
exceeded the 600 000 ms (10 minute) action timeout.

**Fix lever.**
1. Confirm `timeout_ms` is set in the action block (not just spec-level):

   ```yaml
   action:
     kind: wavelet.commercial
     timeout_ms: 3600000   ← must be here, not only at spec top-level
   ```

2. Confirm the spec was last run with `timeout_ms` threaded properly —
   the spec-level `timeoutMs` is a default; the action-block `timeout_ms`
   overrides it. If only the spec level is set and the harness ignores it,
   the action falls back to a shorter built-in default.

3. If the timeout is correct and the run genuinely takes > 1 hour:
   reduce shot count in the brief, add `--dry-run` probes to the skill
   (so the agent doesn't burn 30s probing backend availability), or split
   the spec into two narrower evals.

---

## 2. Debug env vars

These are set in the shell environment; none are spec-level fields.

### `WAVELET_LINT_DEBUG_HALO=1`

**What it does.** Adds per-element halo-contrast debug lines to stderr
during `wavelet lint` for every text element that goes through the WCAG
contrast measurement. Each line prints bounding box, font size, halo
radius, pixel sample count, luminance quartile, and the computed WCAG
ratio.

**When to use.** When lint is failing a text element you believe is
visually fine, or when an element's contrast ratio is borderline and you
want to verify what the lint rule is actually measuring.

**Example output.**
```
halo-debug bbox=(540,1700) 540x220  font=64px halo=12.0px  glyphs=12
  pixels=2880 q=3  dark_mean=0.020 light_mean=0.820  ratio=21.43
```

A `ratio >= 4.5` passes WCAG AA; `>= 7.0` passes AAA. If lint exits 1
on an element with ratio 21.43, that is a false-positive.

**When NOT to use.** In production eval runs — the debug output is high
volume and will fill stderr logs. Only set it in a targeted re-run of lint
in the workdir after the eval has already failed.

**How to set.**
```bash
cd <workdir>
WAVELET_LINT_DEBUG_HALO=1 wavelet lint commercial.html \
  --platform tiktok --mp4 commercial.mp4 2>&1 | less
```

---

### `WAVELET_NO_PREFLIGHT=1`

**What it does.** Skips the render pre-flight gate that normally requires
`wavelet screenplay validate` to have exited 0 (and, for HTML input, a
`lint --mp4` pass) before allowing `wavelet render`. Without this flag,
`wavelet render` reads `.wavelet-trace.jsonl` from cwd and rejects if
those prerequisites are not in the trace.

**When to use.** When debugging render issues on a workdir where you want
to test render output without having gone through the full pipeline:

```bash
cd <workdir>
WAVELET_NO_PREFLIGHT=1 wavelet render commercial.html -o test.mp4
```

Or when testing a render from outside the eval harness (no trace file)
and the gate is blocking a local development workflow.

**When NOT to use.** In eval runs. The preflight gate exists to enforce
pipeline discipline — bypassing it defeats the eval's purpose. Never set
this in `~/.config/wavelet/env` or in the eval shim's environment.

---

### `WAVELET_STRICT_HTML=1`

**What it does.** Makes `wavelet render <non-html-input>` exit 3 with a
clear rejection message instead of warning and continuing. The eval shim
sets this automatically so agents cannot accidentally render via the
legacy `comp.json` path.

**When to use.** It is always set in eval runs via the shim — you do not
need to set it manually for evals. If you want to enforce HTML-only render
in a local workflow or test environment, you can set it.

**When NOT to use.** When doing legacy `comp.json` work outside of eval
context, or when intentionally testing the comp.json render path. Ad-hoc
CLI use does not require HTML.

**What the agent sees when this fires.**
```
wavelet render: REJECTED — WAVELET_STRICT_HTML=1 is set.
Write a `commercial.html` manifest pulling in `scenes/*.html`
and re-run `wavelet render commercial.html`.
```

---

### `WAVELET_REAL`

**What it does.** Tells the `wavelet-traced` PATH shim where the real
`wavelet` binary lives. The shim intercepts every `wavelet` call in the
agent's PATH, logs it to `.wavelet-trace.jsonl`, then forwards to
`$WAVELET_REAL`.

**Set by.** `eval-run` automatically:
```bash
WAVELET_REAL="$PWD/target/debug/wavelet" ...
```

**When to use manually.** Only when running the shim outside of `eval-run`
(e.g. in a custom test harness). In normal eval use, `eval-run` sets it.

**When NOT to use.** Do not set `WAVELET_REAL` in `~/.config/wavelet/env`
— it would affect all wavelet calls in your shell, not just eval runs.

---

### `WAVELET_TRACE`

**What it does.** Tells the `wavelet-traced` shim where to append trace
records. Each record is a JSONL line with `ts`, `argv`, `duration_ms`,
`exit`, `stdout_bytes`, `stderr_bytes`.

**Set by.** `eval-run` (via the shim), which points it at:
```
evals/runs/wavelet/<short>-<ts>/workdir/.wavelet-trace.jsonl
```

**When to use manually.** When running the shim in a custom harness
outside `eval-run`. In normal eval use, `eval-run` sets it.

**When NOT to use.** Do not set `WAVELET_TRACE` globally — it would
accumulate every `wavelet` call system-wide into one file.

---

## 3. Gate behavior reference

The `GATING_CRITERION_KINDS` that block pipeline stage completion in
`wavelet workflow run`:

```rust
const GATING_CRITERION_KINDS: &[&str] = &[
    "brandwork_research_done",
    "adalign_research_done", // transitional alias
    "wavelet_lint_passes",
    "screenplay_duration_fits",
];
```

Other `success_criteria` kinds (`brief_check_passes`, `artifact_exists`,
etc.) are advisory — they appear in pipeline output and grade via the
agent task loop, but do NOT block `workflow run`'s next-stage selection.

---

### `brandwork_research_done` (or `adalign_research_done`)

**What it checks.** Reads `.brandwork-trace.jsonl` (or `.adalign-trace.jsonl`) in the workdir and
verifies that all three verbs `brief`, `brand`, `ads` were invoked as
real research calls — not `--help` probes, not failed calls, not calls
with trivially small output.

**Pass conditions (all must hold for each verb):**
- `argv[1]` matches the verb exactly
- `--help` not present anywhere in argv
- `exit == 0`
- `stdout_bytes >= 256`

**Failure reasons and what they mean:**

| reason | meaning |
|---|---|
| `trace_missing` | `.brandwork-trace.jsonl` (or `.adalign-trace.jsonl`) absent — shim not in PATH or eval harness not running |
| `missing_brandwork_verbs` | One or more of `brief`, `brand`, `ads` not in the trace as real calls |

**Agent retry prompt** includes `missing_verbs` array and `hint`:
> Phase 1 brand research is required. Run `brandwork brief <domain>`,
> `brandwork brand <domain>`, and `brandwork ads <domain>` against the brand's
> actual domain — not `--help` probes — and capture the JSON output.

**Common reasons it stays failed across retries:**

1. Agent runs `brandwork brand --help` (counts as `--help` probe, rejected)
2. Agent runs `brandwork brand fetch <domain>` but the first response is
   < 256 bytes (e.g. the server returns an error envelope)
3. Agent calls the right verbs but against a wrong domain (e.g.
   `newbalance.co.uk` instead of `newbalance.com`) — the gate passes as
   long as exit is 0 and stdout >= 256 bytes; the rubric dimension
   `brand_research` grades domain correctness
4. Trace file never written because `brandwork-traced` shim is not in PATH

---

### `wavelet_lint_passes`

**What it checks.** Reads `.wavelet-trace.jsonl` and verifies a
`wavelet lint` call was made against `commercial.html` (or `scenes/`) with
`--mp4` in argv and that call exited 0.

**Failure reasons:**

| reason | meaning |
|---|---|
| `trace_missing` | `.wavelet-trace.jsonl` absent |
| `no_lint_invocation` | No `lint` verb in the trace at all |
| `missing_mp4_postrender_lint` | lint was called against the HTML target but without `--mp4` |
| `lint_nonzero_exit` | lint ran with `--mp4` but exited non-zero (real findings remain) |
| `lint_target_mismatch` | lint was called but not against `commercial.html` or `scenes/` |

**Agent retry prompt** includes structured detail:
- For `missing_mp4_postrender_lint`: "lint was invoked against commercial.html
  but without `--mp4`. The post-render contrast pass is the only stage that
  sees actual composited pixels..."
- For `lint_nonzero_exit`: "Fix every reported finding and re-run."

**Why `--mp4` is required.** The HTML overlay is composited onto Veo video
frames at render time. A text element that passes contrast on a white HTML
background may fail when overlaid on a dark video frame. The `--mp4` flag
tells lint to render the HTML onto the actual video at the specified
timestamps and measure contrast on the composited result.

**Common reasons it stays failed across retries:**

1. Agent keeps calling `wavelet lint commercial.html --platform tiktok`
   without `--mp4` — the most common failure from 008
2. The post-render pass itself finds real failures (halo contrast too low
   on a dark video background) that require CSS changes to the scene HTML
3. Agent lints `comp.json` instead of `commercial.html` (then hits
   `lint_target_mismatch`)
4. `commercial.mp4` doesn't exist yet when lint is called (render not run)

---

### `screenplay_duration_fits`

**What it checks.** Reads `.wavelet-trace.jsonl` and verifies that
`wavelet screenplay validate <fountain> --duration <secs>` was called
and the **last** such call exited 0. Exit 3 means `over_budget` (script
copy exceeds the declared spot length); exit 0 means within tolerance.

**Failure reasons:**

| reason | meaning |
|---|---|
| `trace_missing` | `.wavelet-trace.jsonl` absent |
| `no_screenplay_validate_call` | No `screenplay validate` in trace |
| `screenplay_over_budget` | Last validate call exited 3 (over budget) |

**Important behavior.** The validator uses the **last** `screenplay validate`
call, not the first — so an agent that first gets exit 3, then rewrites
the script and gets exit 0, is correctly graded as passing. This is
deliberate: the gate reflects the current state after any rewrites.

**The pre-render gate also checks this.** `wavelet render` itself reads
the trace and refuses if `screenplay validate` has not exited 0. This
means the agent cannot skip the screenplay check even if it bypasses
`wavelet workflow run` and calls render directly. (Disable with
`WAVELET_NO_PREFLIGHT=1` for debug-only work.)

**Common reasons it stays failed across retries:**

1. Agent calls `wavelet screenplay parse` but never calls `validate` —
   parse and validate are separate subcommands
2. Script genuinely over budget and the agent is not shortening it
3. Agent rewrites the script but forgets to re-run `validate` after
4. Wrong `--duration` value passed to validate (must match the spec's
   target spot length in seconds)

---

## 4. Pre-launch checklist

Before firing a new eval spec, verify each item. A missed item will cause
a fail that is disconnected from what the spec is actually testing.

### 4.1 Binary is current

```bash
cd packages/wavelet
cargo build -p wavelet
# Verify the build timestamp is fresh:
ls -lh target/debug/wavelet
```

`eval-run` will build automatically if the binary is absent, but NOT if a
stale binary exists. After source changes to lint rules, gate validators,
or render logic, rebuild explicitly.

### 4.2 API keys in env

```bash
cat ~/.config/wavelet/env
# Must contain (at minimum for a Veo + Lyria run):
# GOOGLE_API_KEY=...
# FAL_KEY=...   (optional if not using fal backends)
# ELEVENLABS_API_KEY=...  (optional if not using ElevenLabs)
# OPENROUTER_API_KEY=...  (for the agent LLM)
```

The eval shim sources this file before launching the workbench because
`claude-code` strips API keys from the subprocess environment. If
`~/.config/wavelet/env` is missing or a key is absent, txt2vid calls will
fail fast at exit 2 with a small stderr error envelope.

### 4.3 No stale eval running for the same short name

```bash
eval-status <short>
# Should show: alive=NO
# If alive=YES, kill first: eval-kill <short>
```

Running two evals with the same short name overwrites the `.pid`,
`.log`, and `.workdir` handle files. You will lose track of the older run
and possibly corrupt the trace monitoring.

### 4.4 Spec dry-run passes

```bash
# Check spec is syntactically valid and resolves context vars
workbench eval evals/specs/<spec>.eval.md --dry
```

If the dry-run fails with a context-resolution error, fix the spec before
burning budget on a run.

### 4.5 `timeout_ms` threaded into action block

The spec's top-level `timeoutMs` is a metadata declaration. The action
that actually times out is the `wavelet.commercial` action inside `turns`.
Both must be set:

```yaml
# spec top-level:
timeoutMs: 3600000

# inside turns[0].action:
action:
  kind: wavelet.commercial
  timeout_ms: 3600000   ← this is what the harness enforces
```

If `timeout_ms` is absent from the action block, the harness uses a
shorter built-in default and the run will time out prematurely.

### 4.6 Budget cap consistent

The spec's `budget_usd` (action-block field) should match the rubric's
stated budget and the `wavelet.cost_below` check's `max_usd`:

```yaml
action:
  budget_usd: 7.00
checks:
  - kind: wavelet.cost_below
    max_usd: 7.00
```

If they differ, the agent may spend within its `budget_usd` but still
fail `wavelet.cost_below`. Set them equal unless deliberately testing
a tighter cost ceiling.

### 4.7 Resolution matches the brief

If the spec checks `width: 1080  height: 1920`, the brief must explicitly
say "9:16 portrait" or "vertical" (or similar wording). The agent reads
the brief — if the brief is ambiguous, the storyboard will default to
landscape. Confirm before firing.

---

## 5. Post-fail iteration loop

When an eval fails, work through this sequence in order. Each step
narrows the cause so you only re-fire when you have a concrete hypothesis.

### Step 1 — Identify the failed check

```bash
./evals/bin/eval-status <short>
```

The output shows which checks passed and which failed:

```
wavelet.video_renders: OK
wavelet.cost_below: OK
wavelet.workflow_complete: FAIL  ← tells you the stage
wavelet.workflow_complete: wavelet exited -1 / criteria_failed / ...
```

Also read the log file:

```bash
cat evals/runs/_logs/<short>-<ts>.log
```

The log contains the TAP output with `not ok` lines and inline diagnostics.

### Step 2 — Read the trace

```bash
./evals/bin/eval-status <short>
# Shows: gate-relevant calls, last 6 entries, shot count
```

For a deeper look:

```bash
python3 -c "
import json
for line in open('<workdir>/.wavelet-trace.jsonl'):
    r = json.loads(line)
    argv = r['argv']
    print(r['ts'][-9:-1], f\"exit={r['exit']}\", f\"{r['duration_ms']:>6}ms\",
          ' '.join(argv[1:5]))
"
```

Look for:
- Clusters of fast-failing txt2vid calls (Veo quota)
- Repeated lint calls without `--mp4` (gate-mismatch pattern 1.1)
- Lint byte-count plateau (lint loop pattern 1.4)
- `screenplay validate` present / absent
- `comp.json` render calls (discipline drift pattern 1.5)

### Step 3 — Re-run lint with debug output

If the failed check involves `wavelet_lint_passes` or a rubric contrast
dimension:

```bash
cd <workdir>
WAVELET_LINT_DEBUG_HALO=1 \
  wavelet lint commercial.html --platform <platform> --mp4 commercial.mp4 \
  2>&1 | tee /tmp/lint-debug.txt
```

Read the output:
- Elements with `ratio >= 4.5`: passing WCAG AA — if lint still fails
  them, it is a rule false-positive
- Elements with `ratio < 4.5`: real contrast failure — fix the scene HTML
- Check that `commercial.mp4` exists and is non-trivial (> 1MB) before
  running with `--mp4`; otherwise the MP4 path fails silently

### Step 4 — Decide: real defect or rule false-positive

**Real defect** (agent failed at something it should have done):
- Update the skill instructions so the agent does it correctly next time
- Common fixes:
  - Add `--mp4 commercial.mp4` to the skill's compose-stage lint instruction
  - Add `--aspect 9:16` to the skill's storyboard plan instruction
  - Add `--backend veo-3.1` with serial calls instead of parallel
  - Tighten the `budget_usd` + `--max-cost` per shot to stop cost runaway

**Rule false-positive** (lint fires on content that is visually acceptable):
- File a bd issue: `bd create --title "lint: false-positive on <element type> in <context>" --body "..."`
- Include: the WCAG ratio from debug output, the platform, the HTML
  element involved, and the eval spec it appeared in
- Do NOT modify the lint rule without a regression test that covers the
  false-positive case AND preserves detection of real failures

### Step 5 — File bd issues for each substantive finding

Each distinct finding gets its own issue:

```bash
bd create \
  --title "wavelet: <specific finding>" \
  --body "Observed in eval <short>/<ts>. <symptoms>. Fix: <lever>."
```

Do not bundle multiple findings into one issue — they may have different
owners or fix timelines.

### Step 6 — Land fix and re-fire

After updating the skill or lint rule:

```bash
# Re-run the eval
./evals/bin/eval-run evals/specs/<spec>.eval.md
# Monitor
./evals/bin/eval-watch <short>
```

### Step 7 — Confirm the new run fails differently (or passes)

Compare `eval-status` output between the old and new runs:

- Old run: `lint --mp4: None` → New run: `lint --mp4: 0` means the
  gate-mismatch is fixed
- If the new run fails on a DIFFERENT check, you have made progress —
  the fix worked but uncovered a downstream issue
- If the new run fails on the SAME check with the SAME reason code:
  the fix did not reach the agent's behavior; revisit step 4

---

## 6. Reading a trace quickly

Key fields in each JSONL record:

```jsonc
{
  "ts": "2026-05-23T15:16:31Z",   // UTC timestamp
  "argv": ["wavelet", "shot", "txt2vid", "<prompt>", "--aspect", "9:16", ...],
  "duration_ms": 315,              // wall time — <500ms = fast failure (quota/config)
  "exit": 2,                       // 0=ok, 1=lint fail, 2=api/arg error, 3=hard reject
  "stdout_bytes": 0,               // 0 on failure; >= 256 needed for brandwork gate
  "stderr_bytes": 653              // error detail lives here on failure
}
```

**Exit codes by convention:**

| exit | meaning |
|---|---|
| 0 | success |
| 1 | soft failure (lint findings, validation warnings) |
| 2 | API error, argument error, quota exceeded |
| 3 | hard reject (STRICT_HTML, preflight gate, unrecognized backend) |

**Duration benchmarks:**

| call | typical success duration |
|---|---|
| `--help` probe | 40–100ms |
| `workflow run` | 50–150ms |
| `lint commercial.html --mp4` | 7–15s |
| `shot txt2vid` (Veo 3.1) | 30–55s |
| `music gen` (Lyria) | 10–30s |
| `render commercial.html` | 15–30s |

Any `shot txt2vid` completing in < 500ms and exiting 2 is a rate-limit or
config rejection, not a real generation attempt.

---

## See also

- `EVAL_WORKFLOW.md` — launch / watch / kill / basic post-mortem
- `src/pipelines/workflow.rs` — `GATING_CRITERION_KINDS`, `StageStatus`
- `src/agent/plan/validators/trace.rs` — `BrandworkResearchDone`,
  `WaveletLintPasses`, `ScreenplayDurationFits` implementations
- `src/bin/handlers/render.rs` — `WAVELET_STRICT_HTML`, `WAVELET_NO_PREFLIGHT`
- `src/lint/text_readability_contrast.rs` — `WAVELET_LINT_DEBUG_HALO`
- `evals/bin/wavelet-traced` — shim that writes `.wavelet-trace.jsonl`
