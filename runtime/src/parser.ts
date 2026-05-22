// DOM-walker that pulls a GamutDoc out of an HTML document.
//
// The HTML is the source of truth. The parser doesn't validate
// semantics or fill in defaults — it just translates the gm-* element
// tree into typed IR. Missing required attributes are flagged by the
// linter at the next stage, not silently substituted here.
//
// Works in two modes:
//   - parseDocument(htmlString) — parse a serialised .html file via
//     DOMParser (browser-native) or a linkedom polyfill in Node/Bun.
//   - parseFromElement(rootEl) — walk a live DOM subtree, used by the
//     Web Components runtime when registering itself.

import {
  GamutError,
  type Adjustment,
  type Asset,
  type AudioCue,
  type Clip,
  type CompositionDecl,
  type GamutDoc,
  type Include,
  type Resolution,
  type Scene,
  type Shader,
  type Timeline,
  type Track,
  type TrackItem,
  type VisualAttrs,
} from "./types";

export interface ParseOptions {
  /** When true, unknown gm-* elements throw. Default: false (preserve). */
  strict?: boolean;
}

export function parseDocument(html: string, opts: ParseOptions = {}): GamutDoc {
  const DOMParserCtor = getDOMParser();
  const dom = new DOMParserCtor().parseFromString(html, "text/html");
  const root = findGamutRoot(dom);
  if (!root) {
    throw new GamutError("no <gm-doc> element found in the HTML document");
  }
  return parseFromElement(root, opts);
}

export function parseFromElement(root: Element, opts: ParseOptions = {}): GamutDoc {
  if (root.tagName.toLowerCase() !== "gm-doc") {
    throw new GamutError(
      `expected <gm-doc> root, got <${root.tagName.toLowerCase()}>`,
    );
  }

  const fps = parseInt10(attrRequired(root, "fps"), "<gm-doc fps>");
  const resolution = parseResolution(attrRequired(root, "resolution"));

  const assetEls = queryDirectChildren(root, "gm-asset");
  const compositionEls = queryDirectChildren(root, "gm-composition");
  const timelineEl = queryDirectChildren(root, "gm-timeline")[0];

  if (!timelineEl) {
    throw new GamutError("<gm-doc> must contain a <gm-timeline> child");
  }

  return {
    version: attrOr(root, "version", "1"),
    fps,
    resolution,
    aspect: attrRequired(root, "aspect"),
    assets: assetEls.map(parseAsset),
    compositions: compositionEls.map(parseComposition),
    timeline: parseTimeline(timelineEl, opts),
  };
}

function findGamutRoot(dom: Document): Element | null {
  // Direct querySelector handles both the body-child case and the
  // document-root case (a fragment that *is* <gm-doc>).
  return dom.querySelector("gm-doc");
}

function parseAsset(el: Element): Asset {
  return {
    id: attrRequired(el, "id"),
    kind: attrRequired(el, "kind"),
    src: attrRequired(el, "src"),
  };
}

function parseComposition(el: Element): CompositionDecl {
  return {
    id: attrRequired(el, "id"),
    src: attrRequired(el, "src"),
  };
}

function parseTimeline(el: Element, opts: ParseOptions): Timeline {
  const tracks = queryDirectChildren(el, "gm-track").map((t) => parseTrack(t, opts));
  return {
    id: attrRequired(el, "id"),
    duration: attrRequired(el, "duration"),
    tracks,
  };
}

function parseTrack(el: Element, opts: ParseOptions): Track {
  const z = parseInt10(attrRequired(el, "z"), "<gm-track z>");
  const items: TrackItem[] = [];
  for (const child of elementChildren(el)) {
    const item = parseTrackItem(child, opts);
    if (item) items.push(item);
  }
  return {
    id: attrRequired(el, "id"),
    z,
    items,
    ...visualAttrs(el),
  };
}

function parseTrackItem(el: Element, opts: ParseOptions): TrackItem | null {
  const tag = el.tagName.toLowerCase();
  switch (tag) {
    case "gm-clip":       return parseClip(el);
    case "gm-scene":      return parseScene(el);
    case "gm-audio":      return parseAudio(el);
    case "gm-shader":     return parseShader(el);
    case "gm-adjustment": return parseAdjustment(el);
    case "gm-include":    return parseInclude(el);
    default:
      if (opts.strict) {
        throw new GamutError(
          `unknown track item <${tag}> (strict mode). Allowed: gm-clip, gm-scene, gm-audio, gm-shader, gm-adjustment, gm-include.`,
        );
      }
      return null;
  }
}

function parseClip(el: Element): Clip {
  return {
    kind: "clip",
    id: attrOpt(el, "id"),
    asset: attrRequired(el, "asset"),
    start: attrRequired(el, "start"),
    duration: attrOpt(el, "duration"),
    in: attrOpt(el, "in"),
    out: attrOpt(el, "out"),
    ...visualAttrs(el),
  };
}

function parseScene(el: Element): Scene {
  const src = attrOpt(el, "src");
  // Inline scene content goes inside a <template> child so its
  // <script> blocks stay inert at page-parse time and only run when
  // the runtime clones them at mount. The runtime needs single
  // execution; without <template> wrapping, scripts run twice (once
  // at parse, once at mount). For backward compatibility we still
  // accept raw children if no <template> is present.
  const tmpl = elementChildren(el).find((c) => c.tagName.toLowerCase() === "template");
  const inline = src
    ? undefined
    : tmpl
      ? (tmpl as HTMLTemplateElement).innerHTML.trim() || undefined
      : el.innerHTML.trim() || undefined;
  return {
    kind: "scene",
    id: attrOr(el, "id", `scene-${anonSceneCounter()}`),
    start: attrRequired(el, "start"),
    duration: attrRequired(el, "duration"),
    src,
    inlineHtml: inline,
    ...visualAttrs(el),
  };
}

function parseAudio(el: Element): AudioCue {
  return {
    kind: "audio",
    id: attrOpt(el, "id"),
    asset: attrRequired(el, "asset"),
    start: attrRequired(el, "start"),
    duration: attrRequired(el, "duration"),
    volume: parseFloatAttr(el, "volume"),
    pan: parseFloatAttr(el, "pan"),
    duck: parseFloatAttr(el, "duck"),
    fadeIn: parseFloatAttr(el, "fade-in") ?? parseFloatAttr(el, "fadeIn"),
    fadeOut: parseFloatAttr(el, "fade-out") ?? parseFloatAttr(el, "fadeOut"),
    loop: parseBoolAttr(el, "loop"),
    ...visualAttrs(el),
  };
}

function parseShader(el: Element): Shader {
  const src = attrOpt(el, "src");
  return {
    kind: "shader",
    id: attrOpt(el, "id"),
    lang: attrRequired(el, "lang"),
    start: attrRequired(el, "start"),
    duration: attrRequired(el, "duration"),
    src,
    inlineSource: src ? undefined : el.textContent?.trim() || undefined,
    ...visualAttrs(el),
  };
}

function parseAdjustment(el: Element): Adjustment {
  return {
    kind: "adjustment",
    id: attrOpt(el, "id"),
    filter: attrRequired(el, "filter"),
    start: attrRequired(el, "start"),
    duration: attrRequired(el, "duration"),
    backdrop: attrOpt(el, "backdrop"),
    blend: attrOpt(el, "blend"),
    ...visualAttrs(el),
  };
}

function parseInclude(el: Element): Include {
  const ref = attrOpt(el, "ref");
  const src = attrOpt(el, "src");
  if (!ref && !src) {
    throw new GamutError("<gm-include> must have either ref= or src=");
  }
  if (ref && src) {
    throw new GamutError(
      "<gm-include> cannot have both ref= and src= — pick one (ref= points at a <gm-composition>, src= loads an external file)",
    );
  }
  return {
    kind: "include",
    id: attrOpt(el, "id"),
    start: attrRequired(el, "start"),
    duration: attrRequired(el, "duration"),
    ref,
    src,
    ...visualAttrs(el),
  };
}

// ─── Attribute helpers ───────────────────────────────────────────────

function attrRequired(el: Element, name: string): string {
  const v = el.getAttribute(name);
  if (v === null || v.trim().length === 0) {
    throw new GamutError(
      `<${el.tagName.toLowerCase()}> requires attribute '${name}'`,
    );
  }
  return v.trim();
}

function attrOr(el: Element, name: string, fallback: string): string {
  const v = el.getAttribute(name);
  return v === null || v.trim().length === 0 ? fallback : v.trim();
}

function attrOpt(el: Element, name: string): string | undefined {
  const v = el.getAttribute(name);
  return v === null || v.trim().length === 0 ? undefined : v.trim();
}

function visualAttrs(el: Element): VisualAttrs {
  return {
    class: attrOpt(el, "class"),
    style: attrOpt(el, "style"),
  };
}

function parseFloatAttr(el: Element, name: string): number | undefined {
  const v = el.getAttribute(name);
  if (v === null) return undefined;
  const n = Number(v.trim());
  return Number.isFinite(n) ? n : undefined;
}

function parseBoolAttr(el: Element, name: string): boolean | undefined {
  const v = el.getAttribute(name);
  if (v === null) return undefined;
  const t = v.trim().toLowerCase();
  if (t === "" || t === "true" || t === "1" || t === name) return true;
  if (t === "false" || t === "0") return false;
  return undefined;
}

function parseInt10(value: string, label: string): number {
  if (!/^-?\d+$/.test(value)) {
    throw new GamutError(`${label} must be an integer, got '${value}'`);
  }
  return Number(value);
}

function parseResolution(value: string): Resolution {
  const m = value.match(/^(\d+)x(\d+)$/);
  if (!m) {
    throw new GamutError(
      `<gm-doc resolution='${value}'> must look like '1920x1080'`,
    );
  }
  return { width: Number(m[1]), height: Number(m[2]) };
}

// ─── Tree helpers ────────────────────────────────────────────────────

function elementChildren(el: Element): Element[] {
  const out: Element[] = [];
  for (const child of Array.from(el.childNodes)) {
    if (isElement(child)) out.push(child);
  }
  return out;
}

function queryDirectChildren(parent: Element, tagName: string): Element[] {
  return elementChildren(parent).filter(
    (c) => c.tagName.toLowerCase() === tagName,
  );
}

function isElement(node: Node | { nodeType?: number }): node is Element {
  return (node as Node).nodeType === 1;
}

// ─── DOMParser resolution ────────────────────────────────────────────

function getDOMParser(): typeof DOMParser {
  if (typeof DOMParser !== "undefined") return DOMParser;
  throw new GamutError(
    "DOMParser is not available in this environment. In Node/Bun, install linkedom and inject it: " +
      "import { DOMParser } from 'linkedom'; globalThis.DOMParser = DOMParser;",
  );
}

// ─── Anonymous scene id counter ──────────────────────────────────────

let __anonScene = 0;
function anonSceneCounter(): number {
  return ++__anonScene;
}

/** Test-only: reset the anonymous counter for deterministic test ids. */
export function __resetAnonCounter(): void {
  __anonScene = 0;
}
