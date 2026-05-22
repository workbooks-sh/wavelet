// Shared CLI argument helpers.

import { parseTime } from "@work.books/wavelet-runtime/time";

/** Read a flag value: --flag <val> or -f <val>. Returns null if absent. */
export function flag(args, ...names) {
  for (const name of names) {
    const idx = args.indexOf(name);
    if (idx !== -1 && idx + 1 < args.length) return args[idx + 1];
  }
  return null;
}

/** True if the flag is present anywhere in args. */
export function hasFlag(args, ...names) {
  return names.some((n) => args.includes(n));
}

/** Positional args (anything not starting with `-` and not consumed by a flag). */
export function positionals(args) {
  const out = [];
  for (let i = 0; i < args.length; i++) {
    const a = args[i];
    if (a.startsWith("--") || a.startsWith("-")) {
      // Skip the next token as flag value unless this flag is a known boolean.
      if (!BOOLEAN_FLAGS.has(a)) i++;
      continue;
    }
    out.push(a);
  }
  return out;
}

const BOOLEAN_FLAGS = new Set(["--help", "-h", "--version", "-v", "--dry-run"]);

/**
 * Parse a time string into frames. Requires fps. Throws if the time
 * is missing or unparseable.
 */
export function timeToFrames(value, fps, label) {
  if (!value) throw new Error(`${label}: missing time value`);
  return parseTime(value, fps).frames;
}

/**
 * Parse a time string into SECONDS without needing fps. Accepts
 * "1.5s", "00:00:01:00" (treated as h:m:s.cs where the last group is
 * centi-seconds when fps is unknown — but for fps-free use we expect
 * "Ns" / "N.Ns" notation). Throws on "Nf" (frames require fps).
 */
export function timeToSeconds(value, label) {
  if (!value) throw new Error(`${label}: missing time value`);
  const v = value.trim();
  if (v.endsWith("f")) {
    throw new Error(`${label}: frame notation '${v}' requires fps — use seconds instead (e.g. '1.5s')`);
  }
  if (v.endsWith("s")) {
    const n = Number(v.slice(0, -1));
    if (!Number.isFinite(n)) throw new Error(`${label}: bad seconds '${v}'`);
    return n;
  }
  // Plain number → assume seconds.
  const n = Number(v);
  if (Number.isFinite(n)) return n;
  throw new Error(`${label}: unparseable time '${v}'`);
}
