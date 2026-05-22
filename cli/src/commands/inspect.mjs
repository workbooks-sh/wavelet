// wavelet inspect <file.html> — pretty-print the resolved timeline.

import "../dom.mjs";
import { readFile } from "node:fs/promises";
import { resolve } from "node:path";
import { parseDocument } from "@work.books/wavelet-runtime/parser";
import { resolveTimeline } from "@work.books/wavelet-runtime/timeline";

export async function inspect(args) {
  const file = args[0];
  if (!file) {
    console.error("wavelet inspect: missing file argument");
    return 1;
  }
  const abs = resolve(process.cwd(), file);
  let html;
  try {
    html = await readFile(abs, "utf8");
  } catch (e) {
    console.error(`wavelet inspect: cannot read ${abs}: ${e.message}`);
    return 1;
  }

  let doc;
  try {
    doc = parseDocument(html);
  } catch (e) {
    console.error(`wavelet inspect: parse failed: ${e.message}`);
    return 1;
  }

  let resolved;
  try {
    resolved = resolveTimeline(doc);
  } catch (e) {
    console.error(`wavelet inspect: resolve failed: ${e.message}`);
    return 1;
  }

  const fps = resolved.fps;
  const totalSec = (resolved.durationFrames / fps).toFixed(2);
  console.log(`${file}`);
  console.log(`  version    ${resolved.version}`);
  console.log(`  fps        ${fps}`);
  console.log(`  resolution ${resolved.resolution.width}x${resolved.resolution.height}`);
  console.log(`  aspect     ${resolved.aspect}`);
  console.log(`  duration   ${resolved.durationFrames}f / ${totalSec}s`);
  console.log("");
  console.log("Assets:");
  if (resolved.assets.length === 0) console.log("  (none)");
  for (const a of resolved.assets) {
    console.log(`  ${pad(a.id, 18)} ${pad(a.kind, 10)} ${a.src}`);
  }
  if (resolved.compositions.length > 0) {
    console.log("");
    console.log("Compositions:");
    for (const c of resolved.compositions) {
      console.log(`  ${pad(c.id, 18)} ${c.src}`);
    }
  }
  console.log("");
  console.log("Timeline:");
  // Tracks already in source order; group items by track.
  const tracksByZ = [...resolved.tracks].sort((a, b) => a.z - b.z);
  for (const track of tracksByZ) {
    console.log(`  ${track.id}  (z=${track.z})`);
    if (track.items.length === 0) {
      console.log(`    (empty)`);
      continue;
    }
    for (const item of track.items) {
      const span = `${fmt(item.startFrame, fps)} → ${fmt(item.endFrame, fps)}`;
      const summary = summariseItem(item);
      console.log(`    [${span}] ${pad(item.kind, 10)} ${summary}`);
    }
  }
  return 0;
}

function summariseItem(item) {
  switch (item.kind) {
    case "clip":       return `asset=${item.asset} src[${item.sourceInFrame}f..${item.sourceOutFrame}f]`;
    case "scene":      return `id=${item.id}${item.src ? ` src=${item.src}` : " (inline)"}`;
    case "audio":      return `asset=${item.asset}${typeof item.volume === "number" ? ` vol=${item.volume}` : ""}${typeof item.duck === "number" ? ` duck=${item.duck}dB` : ""}`;
    case "shader":     return `lang=${item.lang}${item.src ? ` src=${item.src}` : " (inline)"}`;
    case "adjustment": return `filter="${item.filter}"`;
    case "include":    return item.ref ? `ref=${item.ref}` : `src=${item.src}`;
    default:           return "";
  }
}

function fmt(frame, fps) {
  const secs = (frame / fps).toFixed(2).padStart(6);
  return `${pad(String(frame), 4)}f ${secs}s`;
}

function pad(s, n) {
  s = String(s);
  return s.length >= n ? s : s + " ".repeat(n - s.length);
}
