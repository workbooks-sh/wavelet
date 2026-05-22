// Mount + unmount a <gm-include>: load a referenced composition
// (either by ref= to a <gm-composition> decl, or src= to an external
// file path), parse it, and embed it as a nested <gm-doc> at the
// include's z-index. The embedded gm-doc runs its own playhead
// internally but its frame is driven externally — the parent
// orchestrator calls tick() each frame with the include-local frame
// number.

import type {
  CompositionDecl,
  ResolvedInclude,
} from "./types";

export interface IncludeMount {
  el: HTMLElement;
  include: ResolvedInclude;
  /** Drive the embedded comp's playhead off the parent's local frame. */
  tick(localFrame: number, fps: number, playing: boolean): void;
  cleanup(): void;
}

export interface IncludeMountContext {
  viewport: HTMLElement;
  baseUrl: string | null;
  zIndex: number;
  /** Compositions declared at the parent doc root (for ref= lookup). */
  compositionDecls: CompositionDecl[];
  /** Set of comp URLs currently in the include stack — used for cycle detection. */
  ancestry: Set<string>;
}

export async function mountInclude(
  include: ResolvedInclude,
  ctx: IncludeMountContext,
): Promise<IncludeMount> {
  const el = document.createElement("div");
  el.className = "gm-include-mount";
  el.style.position = "absolute";
  el.style.inset = "0";
  el.style.zIndex = String(ctx.zIndex);
  if (include.class) el.classList.add(...include.class.split(/\s+/).filter(Boolean));
  if (include.style) el.setAttribute("style", el.getAttribute("style") + ";" + include.style);
  ctx.viewport.appendChild(el);

  // Resolve the target URL.
  const url = resolveIncludeUrl(include, ctx);
  if (!url) {
    el.textContent = `[gm-include: ref="${include.ref}" not found among <gm-composition> decls]`;
    return { el, include, tick: () => {}, cleanup: () => el.remove() };
  }

  // Cycle detection — refuse to nest a comp inside itself.
  if (ctx.ancestry.has(url)) {
    el.textContent = `[gm-include: cycle detected at ${url} — comp already in the include stack]`;
    return { el, include, tick: () => {}, cleanup: () => el.remove() };
  }

  // Fetch + extract the inner <gm-doc> element.
  let innerDocEl: Element | null = null;
  try {
    const res = await fetch(url);
    if (!res.ok) {
      el.textContent = `[gm-include: ${url} → ${res.status}]`;
      return { el, include, tick: () => {}, cleanup: () => el.remove() };
    }
    const html = await res.text();
    const parser = new DOMParser();
    const dom = parser.parseFromString(html, "text/html");
    const found = dom.querySelector("gm-doc");
    if (!found) {
      el.textContent = `[gm-include: ${url} has no <gm-doc> element]`;
      return { el, include, tick: () => {}, cleanup: () => el.remove() };
    }
    innerDocEl = found;
  } catch (e) {
    el.textContent = `[gm-include error: ${e instanceof Error ? e.message : String(e)}]`;
    return { el, include, tick: () => {}, cleanup: () => el.remove() };
  }

  // Adopt the inner <gm-doc> into our document. Mark as embedded so
  // its connectedCallback skips the chrome bar (the parent owns chrome).
  const adoptedDoc = document.importNode(innerDocEl, true) as HTMLElement;
  adoptedDoc.setAttribute("data-embedded", "");
  // Propagate ancestry as a data attr so the embedded doc's own
  // gm-include mounts can detect cycles.
  adoptedDoc.setAttribute("data-include-ancestry", [...ctx.ancestry, url].join("|"));
  el.appendChild(adoptedDoc);

  return {
    el,
    include,
    tick(localFrame, _fps, _playing) {
      // Pause the embedded doc and drive its playhead from outside.
      // Embedded gm-doc exposes seekFrame() once its connectedCallback
      // finishes; if we tick before then, no-op.
      const doc = adoptedDoc as any;
      if (typeof doc.seekFrame === "function") {
        doc.pause();
        doc.seekFrame(Math.max(0, localFrame));
      }
    },
    cleanup() {
      el.remove();
    },
  };
}

function resolveIncludeUrl(include: ResolvedInclude, ctx: IncludeMountContext): string | null {
  let raw: string | null = null;
  if (include.src) {
    raw = include.src;
  } else if (include.ref) {
    const decl = ctx.compositionDecls.find((c) => c.id === include.ref);
    if (!decl) return null;
    raw = decl.src;
  }
  if (!raw) return null;
  if (!ctx.baseUrl) return raw;
  try {
    return new URL(raw, new URL(ctx.baseUrl, window.location.href)).toString();
  } catch {
    return raw;
  }
}
