// wavelet split <html> <track-id> <time>
//
// Find the track item whose window contains <time> on track
// <track-id> and split it into two siblings at that time. The left
// half keeps the original element (preserves inline scene content
// and id); the right half is a clone with adjusted start + duration.
//
// For clips with source-side in/out, the split also re-balances the
// in/out points so the source media stays continuous across the cut.

import { loadHtmlDoc, saveHtmlDoc, GamutCliError, fpsOf, findTrack, trackItems } from "../htmlDoc.mjs";
import { positionals } from "../args.mjs";
import { parseTime } from "@work.books/wavelet-runtime/time";

export async function split(args) {
  const [file, trackId, timeArg] = positionals(args);
  if (!file || !trackId || !timeArg) {
    console.error("wavelet split: usage: wavelet split <html> <track-id> <time>");
    return 1;
  }
  let ctx;
  try {
    ctx = await loadHtmlDoc(file);
  } catch (e) {
    console.error(`wavelet split: ${e.message}`);
    return 1;
  }
  let fps, track, atFrame;
  try {
    fps = fpsOf(ctx.gmDoc);
    track = findTrack(ctx.gmDoc, trackId);
    atFrame = parseTime(timeArg, fps).frames;
  } catch (e) {
    console.error(`wavelet split: ${e.message}`);
    return 1;
  }

  const items = trackItems(track);
  let target = null;
  for (const el of items) {
    const range = resolveItemRange(el, fps);
    if (!range) continue;
    if (atFrame > range.start && atFrame < range.end) {
      target = { el, range };
      break;
    }
  }
  if (!target) {
    console.error(`wavelet split: no item on track '${trackId}' spans frame ${atFrame} (${timeArg})`);
    return 1;
  }

  const { el: left, range } = target;
  const right = left.cloneNode(true);
  const tag = left.tagName.toLowerCase();
  const leftDurFrames = atFrame - range.start;
  const rightDurFrames = range.end - atFrame;

  // Left half: keep start, set duration to (atFrame - start).
  setDurationFrames(left, leftDurFrames, fps);

  // Right half: new start at atFrame, duration to (end - atFrame).
  right.setAttribute("start", framesToString(atFrame, fps));
  setDurationFrames(right, rightDurFrames, fps);

  // If the source element used in/out (clips), re-balance source ranges.
  if (tag === "gm-clip") {
    rebalanceClipSourceRange(left, right, leftDurFrames, fps);
  }

  // If the source element had an id, the clone gets a derived id.
  const id = left.getAttribute("id");
  if (id) {
    right.setAttribute("id", `${id}-b`);
  }

  left.parentNode.insertBefore(right, left.nextSibling);
  await saveHtmlDoc(ctx.dom, ctx.abs);

  console.log(`wavelet split: ${trackId} @ ${timeArg} — '${tag}' split into two siblings`);
  return 0;
}

/** Returns {start, end} frames for an item element, or null if unparseable. */
function resolveItemRange(el, fps) {
  const startRaw = el.getAttribute("start");
  if (!startRaw) return null;
  let start;
  try { start = parseTime(startRaw, fps).frames; } catch { return null; }

  const durRaw = el.getAttribute("duration");
  if (durRaw) {
    try {
      const dur = parseTime(durRaw, fps).frames;
      return { start, end: start + dur };
    } catch { return null; }
  }
  // Clips can derive duration from in/out.
  if (el.tagName.toLowerCase() === "gm-clip") {
    const inRaw = el.getAttribute("in");
    const outRaw = el.getAttribute("out");
    if (inRaw && outRaw) {
      try {
        const dur = parseTime(outRaw, fps).frames - parseTime(inRaw, fps).frames;
        return { start, end: start + dur };
      } catch { return null; }
    }
  }
  return null;
}

function setDurationFrames(el, frames, fps) {
  // Prefer human-readable seconds when it lands on whole frames.
  if (frames % fps === 0) {
    el.setAttribute("duration", `${frames / fps}s`);
  } else {
    el.setAttribute("duration", `${frames}f`);
  }
}

function framesToString(frames, fps) {
  if (frames % fps === 0) return `${frames / fps}s`;
  return `${frames}f`;
}

function rebalanceClipSourceRange(left, right, leftDurFrames, fps) {
  const inRaw = left.getAttribute("in");
  const outRaw = left.getAttribute("out");
  if (!inRaw || !outRaw) return; // duration-only clip; nothing to balance
  let inF, outF;
  try {
    inF = parseTime(inRaw, fps).frames;
    outF = parseTime(outRaw, fps).frames;
  } catch { return; }
  const midF = inF + leftDurFrames;
  left.setAttribute("out", framesToString(midF, fps));
  right.setAttribute("in", framesToString(midF, fps));
  right.setAttribute("out", framesToString(outF, fps));
}
