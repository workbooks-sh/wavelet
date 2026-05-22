// Public runtime entry point. Importing this module registers the
// gm-* custom-element family with the browser. Side-effect import:
//
//   <script type="module" src="https://unpkg.com/@work.books/wavelet-runtime"></script>
//
// or, in a bundler:
//
//   import "@work.books/wavelet-runtime";

import { GamutDoc } from "./elements/GamutDoc";
import { GmDataElement } from "./elements/DataElement";
import { injectRuntimeStyle } from "./style";
import { onReady, onTick, registerTimeline } from "./events";

/**
 * Idempotent registration. Calling more than once is safe — the
 * customElements registry rejects duplicate definitions, so we guard.
 *
 * Also exposes `window.wavelet = { onReady, onTick }` so plain
 * <script> blocks inside <gm-scene> can call these without ESM
 * imports — handy for inline authoring and for environments (like
 * Vite's html-proxy) that don't apply workspace aliases to inline
 * module scripts.
 */
export function register(): void {
  if (typeof customElements === "undefined") return;
  injectRuntimeStyle();
  if (typeof window !== "undefined") {
    const existing = (window as any).wavelet;
    (window as any).wavelet = {
      ...(existing ?? {}),
      onReady,
      onTick,
      registerTimeline,
    };
  }
  define("gm-doc", GamutDoc);
  // Each data element needs its own constructor — customElements
  // rejects sharing one class across multiple tag names.
  defineData("gm-asset");
  defineData("gm-composition");
  defineData("gm-timeline");
  defineData("gm-track");
  defineData("gm-clip");
  defineData("gm-scene");
  defineData("gm-audio");
  defineData("gm-shader");
  defineData("gm-adjustment");
  defineData("gm-include");
}

function define(name: string, ctor: CustomElementConstructor): void {
  if (customElements.get(name)) return;
  customElements.define(name, ctor);
}

function defineData(name: string): void {
  if (customElements.get(name)) return;
  // Fresh anonymous subclass so each tag gets a distinct constructor.
  customElements.define(name, class extends GmDataElement {});
}

// Auto-register on import. Authors can also call register() explicitly
// from their bundle entry if they prefer.
register();

export { GamutDoc, GmDataElement };
export { onReady, onTick } from "./events";
