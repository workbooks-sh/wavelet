// wavelet move <html> --id <element-id> --to <time>
//
// Update one element's `start=` attribute. The element is found by
// its id= attribute (works on any gm-* element that has one). The
// surrounding HTML is preserved byte-for-byte; only the start= attr
// of the matched element changes.
//
// Re-orders sibling track items so source order matches time order
// after the move (preserves the resolver's left-to-right scan
// invariant).

import { loadHtmlDoc, saveHtmlDoc, GamutCliError, ITEM_TAGS } from "../htmlDoc.mjs";
import { flag, positionals } from "../args.mjs";

export async function move(args) {
  const [file] = positionals(args);
  const id = flag(args, "--id", "-i");
  const to = flag(args, "--to", "-t");
  if (!file || !id || !to) {
    console.error("wavelet move: usage: wavelet move <html> --id <element-id> --to <time>");
    return 1;
  }
  let ctx;
  try {
    ctx = await loadHtmlDoc(file);
  } catch (e) {
    console.error(`wavelet move: ${e.message}`);
    return 1;
  }

  const target = ctx.gmDoc.querySelector(`[id="${cssEscape(id)}"]`);
  if (!target) {
    console.error(`wavelet move: no element with id="${id}" inside <gm-doc>`);
    return 1;
  }
  const tag = target.tagName.toLowerCase();
  if (!ITEM_TAGS.has(tag)) {
    console.error(`wavelet move: <${tag} id="${id}"> is not a track item (clip/scene/audio/shader/adjustment/include)`);
    return 1;
  }
  const old = target.getAttribute("start");
  target.setAttribute("start", to);

  // Re-sort the track's direct item children by start= so the source
  // order matches time order. We compare by raw time string when fps
  // matches; otherwise resolve. Simpler approach: re-resolve fps and
  // sort by parsed frames.
  const track = target.parentElement;
  if (track && track.tagName.toLowerCase() === "gm-track") {
    sortTrackChildren(track, ctx.gmDoc);
  }

  await saveHtmlDoc(ctx.dom, ctx.abs);
  console.log(`wavelet move: ${id} start ${old} → ${to}`);
  return 0;
}

function sortTrackChildren(track, gmDoc) {
  // Pull all item children, sort by start in frames, re-insert.
  const fps = Number(gmDoc.getAttribute("fps")) || 30;
  const items = [...track.children].filter((c) =>
    ITEM_TAGS.has(c.tagName.toLowerCase()),
  );
  items.sort((a, b) => {
    const fa = parseStartFrames(a, fps);
    const fb = parseStartFrames(b, fps);
    return fa - fb;
  });
  // Detach and re-append in order. Non-item children (e.g. comments,
  // whitespace text nodes) get re-appended after items — track
  // semantics don't care about non-item ordering.
  for (const el of items) el.remove();
  for (const el of items) track.appendChild(el);
}

function parseStartFrames(el, fps) {
  const raw = el.getAttribute("start");
  if (!raw) return 0;
  // Minimal inline parser (avoid the await/import of runtime here
  // since we're already inside a hot path).
  const v = raw.trim();
  if (v.endsWith("f")) return Number(v.slice(0, -1)) || 0;
  if (v.endsWith("s")) return (Number(v.slice(0, -1)) || 0) * fps;
  // Timecode HH:MM:SS:FF
  const m = v.match(/^(\d{2}):(\d{2}):(\d{2}):(\d{2})$/);
  if (m) return (Number(m[1]) * 3600 + Number(m[2]) * 60 + Number(m[3])) * fps + Number(m[4]);
  return 0;
}

function cssEscape(s) {
  // Conservative: escape anything that isn't [a-zA-Z0-9_-].
  return s.replace(/[^a-zA-Z0-9_-]/g, "\\$&");
}
