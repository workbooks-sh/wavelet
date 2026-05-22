#!/usr/bin/env bash
# Spawn a coding agent against a wavelet brief, capture every wavelet
# invocation + tool call to a structured run directory.
#
# Usage:
#   ./runner.sh --brief briefs/001-mini-coffee.md --run-id <slug> [--agent claude|codex] [--budget USD]
#
# Output: runs/<run-id>/ with workdir, transcript.log, trace.wavelet.jsonl,
# trace.tool-calls.jsonl, and workflow.json.

set -euo pipefail

# --- args ------------------------------------------------------------
BRIEF=""
RUN_ID=""
AGENT="claude"
BUDGET="0.50"
PIPELINE="commercial"

while [[ $# -gt 0 ]]; do
  case "$1" in
    --brief)   BRIEF="$2"; shift 2 ;;
    --run-id)  RUN_ID="$2"; shift 2 ;;
    --agent)   AGENT="$2"; shift 2 ;;
    --budget)  BUDGET="$2"; shift 2 ;;
    --pipeline) PIPELINE="$2"; shift 2 ;;
    -h|--help)
      sed -n '3,/^$/p' "$0" | sed 's/^# \?//'
      exit 0
      ;;
    *)
      echo "unknown arg: $1" >&2
      exit 2
      ;;
  esac
done

if [[ -z "$BRIEF" || -z "$RUN_ID" ]]; then
  echo "usage: $0 --brief <file> --run-id <slug> [--agent claude|codex] [--budget USD]" >&2
  exit 2
fi
if [[ ! -f "$BRIEF" ]]; then
  echo "brief not found: $BRIEF" >&2
  exit 2
fi

# --- paths -----------------------------------------------------------
EVAL_ROOT="$(cd "$(dirname "$0")" && pwd)"
WAVELET_ROOT="$(cd "$EVAL_ROOT/.." && pwd)"
REAL_WAVELET="$WAVELET_ROOT/target/debug/wavelet"
SHIM="$EVAL_ROOT/bin/wavelet-traced"

if [[ ! -x "$REAL_WAVELET" ]]; then
  echo "wavelet binary not built at $REAL_WAVELET — run 'cargo build --bin wavelet' first" >&2
  exit 2
fi
if [[ ! -x "$SHIM" ]]; then
  echo "shim missing: $SHIM" >&2
  exit 2
fi

RUN_DIR="$EVAL_ROOT/runs/$RUN_ID"
WORKDIR="$RUN_DIR/workdir"
TRANSCRIPT="$RUN_DIR/transcript.log"
WAVELET_TRACE="$RUN_DIR/trace.wavelet.jsonl"
TOOL_TRACE="$RUN_DIR/trace.tool-calls.jsonl"

if [[ -e "$RUN_DIR" ]]; then
  echo "run dir already exists: $RUN_DIR — pick a fresh --run-id" >&2
  exit 2
fi
mkdir -p "$WORKDIR"
touch "$WAVELET_TRACE" "$TOOL_TRACE"

# --- shim PATH wiring ------------------------------------------------
# Create a per-run `bin/` containing only `wavelet` (a symlink to the
# shim). Prepend that to PATH so the agent's `wavelet` calls go through
# the trace shim. The shim forwards to $WAVELET_REAL.
SHIM_BIN="$RUN_DIR/bin"
mkdir -p "$SHIM_BIN"
ln -sf "$SHIM" "$SHIM_BIN/wavelet"

export WAVELET_REAL="$REAL_WAVELET"
export WAVELET_TRACE="$WAVELET_TRACE"
export PATH="$SHIM_BIN:$PATH"

# Sanity: confirm the agent will pick up the shim.
which_wavelet=$(/usr/bin/env which wavelet)
if [[ "$which_wavelet" != "$SHIM_BIN/wavelet" ]]; then
  echo "warning: 'which wavelet' resolved to $which_wavelet instead of the shim" >&2
fi

# --- brief copy + prompt assembly ------------------------------------
cp "$BRIEF" "$WORKDIR/brief.md"

REPO_ROOT="$(cd "$WAVELET_ROOT/../.." && pwd)"
SKILL_SRC="$REPO_ROOT/vendor/workbooks/skills/wavelet-director/SKILL.md"
# Copy the SKILL into the workdir so the agent reads it from its own
# scoped directory, no --add-dir needed for the wider monorepo.
if [[ -f "$SKILL_SRC" ]]; then
  cp "$SKILL_SRC" "$WORKDIR/SKILL.md"
fi
# Also copy the pipeline YAML so the agent has the eight-stage spec
# in-workdir alongside the brief + skill.
if [[ -f "$WAVELET_ROOT/pipeline_defs/$PIPELINE.yaml" ]]; then
  cp "$WAVELET_ROOT/pipeline_defs/$PIPELINE.yaml" "$WORKDIR/$PIPELINE.yaml"
fi

PROMPT_FILE="$RUN_DIR/prompt.txt"
{
  echo "You are running inside a fresh evaluation harness for the wavelet commercial-pipeline."
  echo ""
  echo "Your working directory is: $WORKDIR"
  echo "The 'wavelet' binary is on your PATH — every call is logged."
  echo ""
  echo "Three files have been placed in your working directory for you:"
  echo "  brief.md — the creative brief"
  echo "  SKILL.md — the canonical wavelet-director recipe (READ THIS BEFORE ACTING)"
  echo "  $PIPELINE.yaml — the eight-stage pipeline spec"
  echo ""
  echo "Hard ceiling on paid spend: \$$BUDGET USD."
  echo ""
  echo "--- BRIEF ---"
  cat "$BRIEF"
  echo "--- END BRIEF ---"
  echo ""
  echo "When you're done, write a short notes.md in the workdir describing"
  echo "what went well, what surprised you, and what you'd do differently."
} > "$PROMPT_FILE"

# --- agent launch ----------------------------------------------------
echo "=== run $RUN_ID ===" | tee -a "$TRANSCRIPT"
echo "brief:    $BRIEF" | tee -a "$TRANSCRIPT"
echo "agent:    $AGENT" | tee -a "$TRANSCRIPT"
echo "budget:   \$$BUDGET" | tee -a "$TRANSCRIPT"
echo "workdir:  $WORKDIR" | tee -a "$TRANSCRIPT"
echo "wavelet:    $REAL_WAVELET (via shim)" | tee -a "$TRANSCRIPT"
echo "started:  $(date -u +%Y-%m-%dT%H:%M:%SZ)" | tee -a "$TRANSCRIPT"
echo "---" | tee -a "$TRANSCRIPT"

cd "$WORKDIR"

case "$AGENT" in
  claude)
    if ! command -v claude >/dev/null; then
      echo "claude CLI not found on PATH" >&2
      exit 127
    fi
    # stream-json gives one JSON object per agent action (tool call,
    # message, etc.). We tee that to tool-trace and a human-readable
    # text version to transcript.
    claude -p "$(cat "$PROMPT_FILE")" \
      --output-format stream-json \
      --verbose \
      --add-dir "$WORKDIR" \
      --dangerously-skip-permissions \
      2> >(tee -a "$TRANSCRIPT" >&2) \
      | tee -a "$TOOL_TRACE" \
      | /usr/bin/python3 "$EVAL_ROOT/bin/stream-to-transcript.py" \
      >> "$TRANSCRIPT" || true
    ;;
  codex)
    if ! command -v codex >/dev/null; then
      echo "codex CLI not found on PATH" >&2
      exit 127
    fi
    codex exec "$(cat "$PROMPT_FILE")" --skip-git-repo-check \
      2>&1 | tee -a "$TRANSCRIPT" || true
    ;;
  *)
    echo "unknown agent: $AGENT (want: claude|codex)" >&2
    exit 2
    ;;
esac

echo "---" | tee -a "$TRANSCRIPT"
echo "finished: $(date -u +%Y-%m-%dT%H:%M:%SZ)" | tee -a "$TRANSCRIPT"

# --- post-run report -------------------------------------------------
cd "$EVAL_ROOT"
"$REAL_WAVELET" workflow run "$PIPELINE" --workdir "$WORKDIR" \
  > "$RUN_DIR/workflow.json" 2>&1 || true

# Summary line.
wavelet_calls=$(wc -l < "$WAVELET_TRACE" | tr -d ' ')
artifacts=$(find "$WORKDIR" -type f | wc -l | tr -d ' ')

echo ""
echo "=== run $RUN_ID summary ==="
echo "wavelet calls logged:  $wavelet_calls"
echo "artifacts in workdir: $artifacts"
echo "workflow.json:        $RUN_DIR/workflow.json"
echo "transcript:           $TRANSCRIPT"
echo "wavelet trace:          $WAVELET_TRACE"
echo "tool trace:           $TOOL_TRACE"
echo ""
echo "next: cp $EVAL_ROOT/verdict-template.md $RUN_DIR/verdict.md && edit"
