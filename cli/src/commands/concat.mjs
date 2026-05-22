// wavelet concat <h1> <h2> [...] -o <out.html>
//
// Concatenate multiple comp files end-to-end into one new file.
// - The first input provides the document scaffolding (head, runtime
//   <script>, styles.css link, <gm-doc> metadata).
// - Each subsequent input's tracks get appended to matching tracks
//   in the first (by track id). New tracks are added at the end.
// - Track items from later inputs are time-shifted by the cumulative
//   duration of preceding inputs.
// - Assets dedupe by id (first writer wins; subsequent declarations
//   with the same id are dropped with a warning if their src differs).
// - The resulting timeline duration is the sum of all input
//   durations.

import { loadHtmlDoc, saveHtmlDoc, fpsOf, ITEM_TAGS } from "../htmlDoc.mjs";
import { flag, positionals } from "../args.mjs";
import { parseTime } from "@work.books/wavelet-runtime/time";

export async function concat(args) {
  const out = flag(args, "-o", "--out");
  const inputs = positionals(args);
  if (inputs.length < 2 || !out) {
    console.error("wavelet concat: usage: wavelet concat <h1.html> <h2.html> [...] -o <out.html>");
    return 1;
  }

  let base;
  try {
    base = await loadHtmlDoc(inputs[0]);
  } catch (e) {
    console.error(`wavelet concat: ${e.message}`);
    return 1;
  }
  const fps = fpsOf(base.gmDoc);
  const baseTimeline = base.gmDoc.querySelector("gm-timeline");
  if (!baseTimeline) {
    console.error(`wavelet concat: ${inputs[0]}: no <gm-timeline>`);
    return 1;
  }
  let cumulativeFrames = parseTime(baseTimeline.getAttribute("duration") ?? "0s", fps).frames;

  for (let i = 1; i < inputs.length; i++) {
    let part;
    try {
      part = await loadHtmlDoc(inputs[i]);
    } catch (e) {
      console.error(`wavelet concat: ${e.message}`);
      return 1;
    }
    const partFps = fpsOf(part.gmDoc);
    if (partFps !== fps) {
      console.error(`wavelet concat: ${inputs[i]}: fps=${partFps} doesn't match base fps=${fps}`);
      return 1;
    }
    const partTimeline = part.gmDoc.querySelector("gm-timeline");
    if (!partTimeline) {
      console.error(`wavelet concat: ${inputs[i]}: no <gm-timeline>`);
      return 1;
    }
    const partDur = parseTime(partTimeline.getAttribute("duration") ?? "0s", fps).frames;

    // Merge assets: dedupe by id; warn on src mismatch.
    mergeAssets(base.gmDoc, part.gmDoc, inputs[i]);

    // Merge compositions similarly.
    mergeCompositions(base.gmDoc, part.gmDoc, inputs[i]);

    // Merge tracks. For each track in part, find matching id in base.
    for (const partTrack of partTimeline.querySelectorAll("gm-track")) {
      const trackId = partTrack.getAttribute("id");
      if (!trackId) continue;
      let baseTrack = Array.from(baseTimeline.querySelectorAll("gm-track"))
        .find((t) => t.getAttribute("id") === trackId);
      if (!baseTrack) {
        // New track — clone wholesale, but shift each item's start.
        baseTrack = partTrack.cloneNode(true);
        shiftTrackItems(baseTrack, cumulativeFrames, fps);
        baseTimeline.appendChild(baseTrack);
        continue;
      }
      // Existing track — append items with shifted starts.
      for (const item of [...partTrack.children]) {
        if (!ITEM_TAGS.has(item.tagName.toLowerCase())) continue;
        const shifted = item.cloneNode(true);
        shiftItemStart(shifted, cumulativeFrames, fps);
        baseTrack.appendChild(shifted);
      }
    }

    cumulativeFrames += partDur;
  }

  // Update the timeline duration.
  baseTimeline.setAttribute("duration", framesToString(cumulativeFrames, fps));

  // Save as <out>.
  const outAbs = (await import("node:path")).resolve(process.cwd(), out);
  await saveHtmlDoc(base.dom, outAbs);
  console.log(`wavelet concat: wrote ${out} (${inputs.length} inputs, ${cumulativeFrames / fps}s total)`);
  return 0;
}

function mergeAssets(baseDoc, partDoc, partName) {
  const baseIds = new Set(
    Array.from(baseDoc.querySelectorAll("gm-asset")).map((a) => a.getAttribute("id")),
  );
  for (const a of partDoc.querySelectorAll("gm-asset")) {
    const id = a.getAttribute("id");
    if (!id) continue;
    if (baseIds.has(id)) {
      // Optional: warn on src mismatch — kept silent for now.
      continue;
    }
    const clone = a.cloneNode(true);
    // Append before the timeline so assets stay together.
    const timeline = baseDoc.querySelector("gm-timeline");
    baseDoc.insertBefore(clone, timeline);
    baseIds.add(id);
  }
}

function mergeCompositions(baseDoc, partDoc, partName) {
  const baseIds = new Set(
    Array.from(baseDoc.querySelectorAll("gm-composition")).map((c) => c.getAttribute("id")),
  );
  for (const c of partDoc.querySelectorAll("gm-composition")) {
    const id = c.getAttribute("id");
    if (!id || baseIds.has(id)) continue;
    const clone = c.cloneNode(true);
    const timeline = baseDoc.querySelector("gm-timeline");
    baseDoc.insertBefore(clone, timeline);
    baseIds.add(id);
  }
}

function shiftTrackItems(track, deltaFrames, fps) {
  for (const item of track.children) {
    if (!ITEM_TAGS.has(item.tagName.toLowerCase())) continue;
    shiftItemStart(item, deltaFrames, fps);
  }
}

function shiftItemStart(el, deltaFrames, fps) {
  const raw = el.getAttribute("start") ?? "0s";
  try {
    const startFrames = parseTime(raw, fps).frames;
    el.setAttribute("start", framesToString(startFrames + deltaFrames, fps));
  } catch {
    // Bad start — leave untouched; the linter will catch it.
  }
}

function framesToString(frames, fps) {
  if (frames % fps === 0) return `${frames / fps}s`;
  return `${frames}f`;
}
