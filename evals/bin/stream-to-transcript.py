#!/usr/bin/env python3
"""Turn Claude Code stream-json output into a readable transcript.

Reads JSONL from stdin (one event per line as emitted by
`claude -p ... --output-format stream-json`), writes a flat human-
readable summary to stdout (one line per event, including assistant
prose + tool calls + tool results in compact form).

The full raw stream is also captured in trace.tool-calls.jsonl by the
runner — this is the easy-to-grep companion.
"""
import json
import sys
from typing import Any


def short(s: Any, n: int = 240) -> str:
    s = "" if s is None else str(s)
    s = s.replace("\n", " ⏎ ").strip()
    return s if len(s) <= n else s[: n - 1] + "…"


def fmt_tool_use(block: dict) -> str:
    name = block.get("name", "?")
    inp = block.get("input", {})
    if name == "Bash":
        cmd = inp.get("command", "")
        return f"  ↪ Bash: {short(cmd, 200)}"
    if name in ("Read", "Write", "Edit"):
        path = inp.get("file_path", "")
        return f"  ↪ {name}: {short(path, 200)}"
    if name == "Glob" or name == "Grep":
        return f"  ↪ {name}: {short(json.dumps(inp), 200)}"
    return f"  ↪ {name}: {short(json.dumps(inp), 200)}"


def main() -> int:
    for raw in sys.stdin:
        raw = raw.strip()
        if not raw:
            continue
        try:
            ev = json.loads(raw)
        except json.JSONDecodeError:
            print(f"(unparsable) {short(raw, 200)}")
            continue
        et = ev.get("type", "?")
        if et == "system":
            sub = ev.get("subtype", "")
            print(f"[system:{sub}]")
        elif et == "assistant":
            msg = ev.get("message", {})
            for c in msg.get("content", []):
                ct = c.get("type", "?")
                if ct == "text":
                    print(f"[asst] {short(c.get('text',''), 400)}")
                elif ct == "tool_use":
                    print(f"[tool_use] {c.get('name','?')}")
                    print(fmt_tool_use(c))
                else:
                    print(f"[asst:{ct}]")
        elif et == "user":
            msg = ev.get("message", {})
            for c in msg.get("content", []):
                if isinstance(c, dict) and c.get("type") == "tool_result":
                    out = c.get("content", "")
                    if isinstance(out, list):
                        out = " ".join(
                            x.get("text", "") if isinstance(x, dict) else str(x)
                            for x in out
                        )
                    is_err = c.get("is_error", False)
                    tag = "tool_err" if is_err else "tool_ok"
                    print(f"[{tag}] {short(out, 400)}")
        elif et == "result":
            print(f"[result] {ev.get('subtype','?')} cost=${ev.get('total_cost_usd','?')} duration_ms={ev.get('duration_ms','?')}")
        else:
            print(f"[{et}]")
    return 0


if __name__ == "__main__":
    sys.exit(main())
