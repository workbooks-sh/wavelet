// Structural linter for parsed/resolved wavelet documents.
//
// Collects findings rather than throwing on the first one — the agent
// gets a complete diagnostic in one pass. The linter has no opinions
// about taste; it catches things the runtime physically cannot render
// (dangling refs, bad times, schedule overflow, duplicate ids, missing
// scene src files, etc).

import { parseTime } from "./time";
import {
  GamutError,
  type GamutDoc,
  type LintFinding,
  type ResolvedTrackItem,
  type Track,
  type TrackItem,
} from "./types";

export interface LintOptions {
  /**
   * Async file-existence check. The linter calls this for each
   * `<gm-scene src>`, `<gm-include src>`, `<gm-shader src>` and
   * `<gm-asset src>`. Pass null to skip filesystem checks (useful in
   * browser contexts where fs access is irrelevant).
   */
  fileExists?: (relativePath: string) => Promise<boolean> | boolean;
}

export async function lintDocument(
  doc: GamutDoc,
  opts: LintOptions = {},
): Promise<LintFinding[]> {
  const findings: LintFinding[] = [];

  lintAssets(doc, findings);
  lintCompositions(doc, findings);
  lintTimeline(doc, findings);
  lintDuplicateIds(doc, findings);
  lintTimelineCoverage(doc, findings);
  lintAudioCoverage(doc, findings);

  // File-existence checks are async; do them in a second pass.
  if (opts.fileExists) {
    await lintFileRefs(doc, opts.fileExists, findings);
  }

  return findings;
}

/**
 * Walk every element with an `id=` attribute inside the gm-doc
 * subtree and report duplicates. HTML spec requires unique ids
 * across a document — duplicates make `getElementById` /
 * `querySelector("[id=…]")` return the first match silently,
 * which masks bugs. Concat in particular tends to produce this
 * (each input may have its own scene id="hello").
 *
 * Findings are warnings (not errors) because the browser tolerates
 * duplicates — the lookup just becomes ambiguous. The CLI's
 * wavelet move/split commands rely on unique ids, so the agent will
 * usually want to fix them, but won't break the render.
 */
function lintDuplicateIds(doc: GamutDoc, out: LintFinding[]): void {
  const seen = new Map<string, string[]>();
  const add = (id: string | undefined, where: string) => {
    if (!id) return;
    if (!seen.has(id)) seen.set(id, []);
    seen.get(id)!.push(where);
  };

  for (const a of doc.assets) add(a.id, `gm-asset`);
  for (const c of doc.compositions) add(c.id, `gm-composition`);
  add(doc.timeline.id, `gm-timeline`);
  for (const t of doc.timeline.tracks) {
    add(t.id, `gm-track`);
    for (const item of t.items) {
      add(item.id, `gm-${item.kind}`);
    }
  }

  for (const [id, locations] of seen) {
    if (locations.length <= 1) continue;
    out.push({
      severity: "warning",
      code: "duplicate-element-id",
      message: `id="${id}" appears ${locations.length} times across ${locations.join(", ")} — HTML spec requires unique ids`,
      at: `[id=${id}]`,
    });
  }
}

function lintAssets(doc: GamutDoc, out: LintFinding[]): void {
  const seen = new Set<string>();
  for (const a of doc.assets) {
    if (seen.has(a.id)) {
      out.push({
        severity: "error",
        code: "duplicate-asset-id",
        message: `duplicate <gm-asset id="${a.id}"> — asset ids must be unique`,
        at: `gm-asset[id=${a.id}]`,
      });
    }
    seen.add(a.id);
  }
}

function lintCompositions(doc: GamutDoc, out: LintFinding[]): void {
  const seen = new Set<string>();
  for (const c of doc.compositions) {
    if (seen.has(c.id)) {
      out.push({
        severity: "error",
        code: "duplicate-composition-id",
        message: `duplicate <gm-composition id="${c.id}">`,
        at: `gm-composition[id=${c.id}]`,
      });
    }
    seen.add(c.id);
  }
}

function lintTimeline(doc: GamutDoc, out: LintFinding[]): void {
  const fps = doc.fps;
  if (!Number.isInteger(fps) || fps <= 0) {
    out.push({
      severity: "error",
      code: "bad-fps",
      message: `<gm-doc fps="${doc.fps}"> must be a positive integer`,
    });
    return;
  }

  let durationFrames: number;
  try {
    durationFrames = parseTime(doc.timeline.duration, fps).frames;
  } catch (e) {
    out.push({
      severity: "error",
      code: "bad-time",
      message: `<gm-timeline duration="${doc.timeline.duration}">: ${
        e instanceof Error ? e.message : String(e)
      }`,
      at: "gm-timeline",
    });
    return;
  }

  const trackIds = new Set<string>();
  const trackZ = new Set<number>();
  const assetIds = new Set(doc.assets.map((a) => a.id));
  const compositionIds = new Set(doc.compositions.map((c) => c.id));

  for (const track of doc.timeline.tracks) {
    if (trackIds.has(track.id)) {
      out.push({
        severity: "error",
        code: "duplicate-track-id",
        message: `duplicate <gm-track id="${track.id}">`,
        at: `gm-track[id=${track.id}]`,
      });
    }
    trackIds.add(track.id);

    if (trackZ.has(track.z)) {
      out.push({
        severity: "warning",
        code: "duplicate-track-z",
        message: `<gm-track id="${track.id}" z="${track.z}"> shares z with another track — composite order is ambiguous`,
        at: `gm-track[id=${track.id}]`,
      });
    }
    trackZ.add(track.z);

    lintTrack(track, fps, durationFrames, assetIds, compositionIds, out);
  }
}

function lintTrack(
  track: Track,
  fps: number,
  parentDurationFrames: number,
  assetIds: Set<string>,
  compositionIds: Set<string>,
  out: LintFinding[],
): void {
  for (const item of track.items) {
    lintItem(item, track.id, fps, parentDurationFrames, assetIds, compositionIds, out);
  }
}

function lintItem(
  item: TrackItem,
  trackId: string,
  fps: number,
  parentDurationFrames: number,
  assetIds: Set<string>,
  compositionIds: Set<string>,
  out: LintFinding[],
): void {
  const loc = `gm-track[id=${trackId}] > gm-${item.kind}`;
  let startFrame: number;
  try {
    startFrame = parseTime(item.start, fps).frames;
  } catch (e) {
    out.push({
      severity: "error",
      code: "bad-time",
      message: `${loc}: invalid start="${item.start}" — ${
        e instanceof Error ? e.message : String(e)
      }`,
      at: loc,
    });
    return;
  }

  let endFrame: number | null = null;
  if (item.duration) {
    try {
      endFrame = startFrame + parseTime(item.duration, fps).frames;
    } catch (e) {
      out.push({
        severity: "error",
        code: "bad-time",
        message: `${loc}: invalid duration="${item.duration}" — ${
          e instanceof Error ? e.message : String(e)
        }`,
        at: loc,
      });
      return;
    }
  }

  // Dangling asset refs
  if ((item.kind === "clip" || item.kind === "audio") && !assetIds.has(item.asset)) {
    out.push({
      severity: "error",
      code: "dangling-asset-ref",
      message: `${loc} references asset="${item.asset}" but no <gm-asset id="${item.asset}"> exists`,
      at: loc,
    });
  }

  // Dangling composition ref
  if (item.kind === "include" && item.ref && !compositionIds.has(item.ref)) {
    out.push({
      severity: "error",
      code: "dangling-composition-ref",
      message: `${loc} references composition ref="${item.ref}" but no <gm-composition id="${item.ref}"> exists`,
      at: loc,
    });
  }

  // Schedule overflow
  if (endFrame !== null && endFrame > parentDurationFrames) {
    out.push({
      severity: "error",
      code: "schedule-overflow",
      message: `${loc} ends at frame ${endFrame} but timeline duration is ${parentDurationFrames} frames`,
      at: loc,
    });
  }

  // Clip-specific: must have duration OR (in AND out)
  if (item.kind === "clip" && !item.duration && (!item.in || !item.out)) {
    out.push({
      severity: "error",
      code: "missing-clip-extent",
      message: `${loc} requires either duration= or BOTH in= and out= to define its frame extent`,
      at: loc,
    });
  }
}

async function lintFileRefs(
  doc: GamutDoc,
  fileExists: NonNullable<LintOptions["fileExists"]>,
  out: LintFinding[],
): Promise<void> {
  const check = async (path: string, loc: string, code: string) => {
    try {
      const ok = await fileExists(path);
      if (!ok) {
        out.push({
          severity: "error",
          code,
          message: `${loc}: file '${path}' not found`,
          at: loc,
        });
      }
    } catch (e) {
      out.push({
        severity: "warning",
        code: "file-check-failed",
        message: `${loc}: could not check '${path}' (${e instanceof Error ? e.message : String(e)})`,
        at: loc,
      });
    }
  };

  for (const a of doc.assets) {
    await check(a.src, `gm-asset[id=${a.id}]`, "missing-asset-file");
  }
  for (const c of doc.compositions) {
    await check(c.src, `gm-composition[id=${c.id}]`, "missing-composition-file");
  }
  for (const track of doc.timeline.tracks) {
    for (const item of track.items) {
      if (item.kind === "scene" && item.src) {
        await check(item.src, `gm-track[id=${track.id}] > gm-scene`, "missing-scene-file");
      }
      if (item.kind === "shader" && item.src) {
        await check(item.src, `gm-track[id=${track.id}] > gm-shader`, "missing-shader-file");
      }
      if (item.kind === "include" && item.src) {
        await check(item.src, `gm-track[id=${track.id}] > gm-include`, "missing-include-file");
      }
    }
  }
}

/**
 * Walk the renderable track items (clip / scene / include) and flag
 * places where the visible canvas would be empty:
 *
 *   - leading gap   — no renderable active during frame 0
 *   - trailing gap  — no renderable active during the last frames
 *   - internal gap  — a stretch of > GAP_THRESHOLD_FRAMES (default
 *     half a second) anywhere in the middle where nothing renderable
 *     is active
 *
 * Audio cues and <gm-adjustment> overlays don't count — they're
 * atmospheric layers. A "renderable" item is something that puts
 * pixels on the canvas: clip (video/image), scene (HTML), or include
 * (nested composition).
 *
 * All findings are warnings — the runtime renders the void as black,
 * which may be intentional (dramatic silence before the first beat).
 * The check surfaces the choice rather than blocking it.
 */
function lintTimelineCoverage(doc: GamutDoc, out: LintFinding[]): void {
  const fps = doc.fps;
  if (!Number.isInteger(fps) || fps <= 0) return;

  let durationFrames: number;
  try {
    durationFrames = parseTime(doc.timeline.duration, fps).frames;
  } catch {
    return;
  }
  if (durationFrames <= 0) return;

  const RENDERABLE = new Set<TrackItem["kind"]>(["clip", "scene", "include"]);
  const intervals: Array<{ start: number; end: number }> = [];
  for (const track of doc.timeline.tracks) {
    for (const item of track.items) {
      if (!RENDERABLE.has(item.kind)) continue;
      let start: number;
      try {
        start = parseTime(item.start, fps).frames;
      } catch {
        continue;
      }
      const end = endFrameOf(item, start, fps);
      if (end === null) continue;
      intervals.push({ start: Math.max(0, start), end: Math.min(durationFrames, end) });
    }
  }
  if (intervals.length === 0) {
    out.push({
      severity: "warning",
      code: "timeline-no-renderable",
      message: `timeline has no <gm-clip>, <gm-scene>, or <gm-include> — the viewer sees a black frame for all ${durationFrames} frames`,
      at: "gm-timeline",
    });
    return;
  }

  // Sort + merge overlapping intervals.
  intervals.sort((a, b) => a.start - b.start);
  const merged: Array<{ start: number; end: number }> = [];
  for (const iv of intervals) {
    const last = merged[merged.length - 1];
    if (last && iv.start <= last.end) {
      last.end = Math.max(last.end, iv.end);
    } else {
      merged.push({ ...iv });
    }
  }

  // Leading gap.
  if (merged[0].start > 0) {
    out.push({
      severity: "warning",
      code: "timeline-leading-gap",
      message: `no renderable item active from frame 0 to ${merged[0].start} (${secStr(merged[0].start, fps)}) — the comp opens on a black frame`,
      at: "gm-timeline",
    });
  }

  // Trailing gap.
  const tail = merged[merged.length - 1];
  if (tail.end < durationFrames) {
    out.push({
      severity: "warning",
      code: "timeline-trailing-gap",
      message: `no renderable item active from frame ${tail.end} (${secStr(tail.end, fps)}) to ${durationFrames} (${secStr(durationFrames, fps)}) — the comp ends on a black frame`,
      at: "gm-timeline",
    });
  }

  // Internal gaps > half a second.
  const GAP_THRESHOLD = Math.max(1, Math.floor(fps / 2));
  for (let i = 0; i < merged.length - 1; i++) {
    const gapStart = merged[i].end;
    const gapEnd = merged[i + 1].start;
    const gapLen = gapEnd - gapStart;
    if (gapLen >= GAP_THRESHOLD) {
      out.push({
        severity: "warning",
        code: "timeline-internal-gap",
        message: `no renderable item active from frame ${gapStart} (${secStr(gapStart, fps)}) to ${gapEnd} (${secStr(gapEnd, fps)}) — ${gapLen} frame${gapLen === 1 ? "" : "s"} of black canvas`,
        at: "gm-timeline",
      });
    }
  }
}

/**
 * If the comp declares ANY <gm-audio> cues, check that they cover
 * the start and end of the timeline. A leading silence > half a
 * second often feels like a late start; a trailing silence often
 * feels like the comp "ended early" before the visuals wrap.
 *
 * Skipped when there are zero audio cues — silent video is fine and
 * a deliberate choice, not a bug.
 */
function lintAudioCoverage(doc: GamutDoc, out: LintFinding[]): void {
  const fps = doc.fps;
  if (!Number.isInteger(fps) || fps <= 0) return;

  let durationFrames: number;
  try {
    durationFrames = parseTime(doc.timeline.duration, fps).frames;
  } catch {
    return;
  }
  if (durationFrames <= 0) return;

  const audioIntervals: Array<{ start: number; end: number }> = [];
  for (const track of doc.timeline.tracks) {
    for (const item of track.items) {
      if (item.kind !== "audio") continue;
      let start: number;
      try {
        start = parseTime(item.start, fps).frames;
      } catch {
        continue;
      }
      const end = endFrameOf(item, start, fps);
      if (end === null) continue;
      audioIntervals.push({ start: Math.max(0, start), end: Math.min(durationFrames, end) });
    }
  }
  if (audioIntervals.length === 0) return;

  audioIntervals.sort((a, b) => a.start - b.start);
  const first = audioIntervals[0].start;
  const last = audioIntervals.reduce((acc, iv) => Math.max(acc, iv.end), 0);
  const GAP_THRESHOLD = Math.max(1, Math.floor(fps / 2));

  if (first >= GAP_THRESHOLD) {
    out.push({
      severity: "warning",
      code: "audio-leading-silence",
      message: `audio first enters at frame ${first} (${secStr(first, fps)}) — viewers hear ${secStr(first, fps)} of silence before any sound. If intentional (dramatic pause), ignore.`,
      at: "gm-timeline",
    });
  }
  if (durationFrames - last >= GAP_THRESHOLD) {
    out.push({
      severity: "warning",
      code: "audio-trailing-silence",
      message: `audio ends at frame ${last} (${secStr(last, fps)}) but timeline runs to ${durationFrames} (${secStr(durationFrames, fps)}) — the final ${secStr(durationFrames - last, fps)} plays silently. If intentional, ignore; otherwise extend the audio or the cue's duration.`,
      at: "gm-timeline",
    });
  }
}

/** Resolve an item's end frame from its declared duration OR (for clips) in/out range. */
function endFrameOf(item: TrackItem, startFrame: number, fps: number): number | null {
  if (item.duration) {
    try {
      return startFrame + parseTime(item.duration, fps).frames;
    } catch {
      return null;
    }
  }
  if (item.kind === "clip" && item.in && item.out) {
    try {
      return startFrame + parseTime(item.out, fps).frames - parseTime(item.in, fps).frames;
    } catch {
      return null;
    }
  }
  return null;
}

function secStr(frame: number, fps: number): string {
  return `${(frame / fps).toFixed(2)}s`;
}

/**
 * Convenience: count findings by severity.
 */
export function summariseFindings(findings: LintFinding[]): { errors: number; warnings: number } {
  let errors = 0;
  let warnings = 0;
  for (const f of findings) {
    if (f.severity === "error") errors++;
    else warnings++;
  }
  return { errors, warnings };
}
