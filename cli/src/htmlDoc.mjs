// Shared helpers for loading + mutating + saving an HTML composition.
//
// The mutation commands (move, split, cut, concat) all follow the
// same shape: parse HTML into a live DOM, mutate the <gm-doc>
// subtree in place, serialize back. We use linkedom rather than
// re-parsing through wavelet-runtime so the surrounding <head>, runtime
// <script> tag, stylesheet links, comments, and whitespace are
// preserved byte-for-byte everywhere outside the mutated subtree.

import "./dom.mjs";
import { readFile, writeFile } from "node:fs/promises";
import { resolve } from "node:path";
import { DOMParser } from "linkedom";

export class GamutCliError extends Error {
  constructor(msg) { super(msg); this.name = "GamutCliError"; }
}

export async function loadHtmlDoc(file) {
  const abs = resolve(process.cwd(), file);
  const html = await readFile(abs, "utf8");
  const dom = new DOMParser().parseFromString(html, "text/html");
  const gmDoc = dom.querySelector("gm-doc");
  if (!gmDoc) {
    throw new GamutCliError(`${file}: no <gm-doc> element found`);
  }
  return { abs, dom, gmDoc };
}

export async function saveHtmlDoc(dom, abs) {
  // linkedom serializes via .toString() at the document level.
  const out = dom.toString();
  await writeFile(abs, out, "utf8");
}

/** Resolve fps from a parsed <gm-doc> Element. Throws if missing/invalid. */
export function fpsOf(gmDoc) {
  const v = gmDoc.getAttribute("fps");
  const n = Number(v);
  if (!Number.isInteger(n) || n <= 0) {
    throw new GamutCliError(`<gm-doc fps="${v}"> must be a positive integer`);
  }
  return n;
}

/** Find a <gm-track id="…"> inside the doc's <gm-timeline>. */
export function findTrack(gmDoc, trackId) {
  const tl = gmDoc.querySelector("gm-timeline");
  if (!tl) throw new GamutCliError("<gm-doc> has no <gm-timeline>");
  const track = Array.from(tl.querySelectorAll("gm-track"))
    .find((t) => t.getAttribute("id") === trackId);
  if (!track) {
    const available = Array.from(tl.querySelectorAll("gm-track"))
      .map((t) => t.getAttribute("id"))
      .filter(Boolean);
    throw new GamutCliError(
      `track '${trackId}' not found. Available: ${available.join(", ") || "(none)"}`,
    );
  }
  return track;
}

/** Track item element types (the children of a <gm-track>). */
export const ITEM_TAGS = new Set([
  "gm-clip", "gm-scene", "gm-audio", "gm-shader", "gm-adjustment", "gm-include",
]);

/** Direct-child track items, in source order. */
export function trackItems(track) {
  return Array.from(track.children).filter((c) =>
    ITEM_TAGS.has(c.tagName.toLowerCase()),
  );
}
