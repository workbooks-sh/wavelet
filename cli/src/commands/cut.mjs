// wavelet cut <html> <track-id> --in <t> --out <t>
//
// Remove a time range across one track. Items wholly inside the
// range are removed. Items straddling the range get shortened to the
// part that survives. The timeline duration is left alone — the cut
// leaves a gap (this is "delete in-place"; if you want a ripple
// delete, follow up with wavelet move on the trailing items, or use
// concat with a fresh comp).

import { loadHtmlDoc, saveHtmlDoc, fpsOf, findTrack, trackItems } from "../htmlDoc.mjs";
import { flag, positionals } from "../args.mjs";
import { parseTime } from "@work.books/wavelet-runtime/time";

export async function cut(args) {
  const [file, trackId] = positionals(args);
  const inT = flag(args, "--in");
  const outT = flag(args, "--out");
  if (!file || !trackId || !inT || !outT) {
    console.error("wavelet cut: usage: wavelet cut <html> <track-id> --in <t> --out <t>");
    return 1;
  }
  let ctx;
  try {
    ctx = await loadHtmlDoc(file);
  } catch (e) {
    console.error(`wavelet cut: ${e.message}`);
    return 1;
  }
  let fps, track, inF, outF;
  try {
    fps = fpsOf(ctx.gmDoc);
    track = findTrack(ctx.gmDoc, trackId);
    inF = parseTime(inT, fps).frames;
    outF = parseTime(outT, fps).frames;
  } catch (e) {
    console.error(`wavelet cut: ${e.message}`);
    return 1;
  }
  if (outF <= inF) {
    console.error(`wavelet cut: --out (${outT}) must be after --in (${inT})`);
    return 1;
  }

  let removed = 0;
  let shortened = 0;
  for (const el of trackItems(track)) {
    const range = resolveItemRange(el, fps);
    if (!range) continue;
    // Outside the cut window entirely — no change.
    if (range.end <= inF || range.start >= outF) continue;
    // Wholly inside the window — remove.
    if (range.start >= inF && range.end <= outF) {
      el.remove();
      removed++;
      continue;
    }
    // Straddles the left edge — keep [start, inF].
    if (range.start < inF && range.end > inF && range.end <= outF) {
      shortenTo(el, range.start, inF - range.start, fps);
      shortened++;
      continue;
    }
    // Straddles the right edge — push start to outF, shorten.
    if (range.start >= inF && range.start < outF && range.end > outF) {
      el.setAttribute("start", framesToString(outF, fps));
      shortenTo(el, outF, range.end - outF, fps);
      shortened++;
      continue;
    }
    // Encloses the cut entirely — left half + right gap (we keep the
    // left half here; the user can run a follow-up split if they
    // want both halves preserved).
    if (range.start < inF && range.end > outF) {
      shortenTo(el, range.start, inF - range.start, fps);
      shortened++;
    }
  }

  await saveHtmlDoc(ctx.dom, ctx.abs);
  console.log(`wavelet cut: ${trackId} [${inT}..${outT}] — removed ${removed}, shortened ${shortened}`);
  return 0;
}

function resolveItemRange(el, fps) {
  const startRaw = el.getAttribute("start");
  if (!startRaw) return null;
  let start;
  try { start = parseTime(startRaw, fps).frames; } catch { return null; }
  const durRaw = el.getAttribute("duration");
  if (durRaw) {
    try { return { start, end: start + parseTime(durRaw, fps).frames }; } catch { return null; }
  }
  if (el.tagName.toLowerCase() === "gm-clip") {
    const inRaw = el.getAttribute("in");
    const outRaw = el.getAttribute("out");
    if (inRaw && outRaw) {
      try {
        return { start, end: start + parseTime(outRaw, fps).frames - parseTime(inRaw, fps).frames };
      } catch { return null; }
    }
  }
  return null;
}

function shortenTo(el, newStartFrame, newDurFrames, fps) {
  el.setAttribute("duration", framesToString(newDurFrames, fps));
  // If the clip used in/out, rebalance the out to match the new duration.
  if (el.tagName.toLowerCase() === "gm-clip") {
    const inRaw = el.getAttribute("in");
    if (inRaw) {
      try {
        const inF = parseTime(inRaw, fps).frames;
        el.setAttribute("out", framesToString(inF + newDurFrames, fps));
      } catch { /* leave as-is */ }
    }
  }
}

function framesToString(frames, fps) {
  if (frames % fps === 0) return `${frames / fps}s`;
  return `${frames}f`;
}
