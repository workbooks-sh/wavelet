// Base class for gm-* elements that carry data only and never render
// in place. They exist in the DOM so <gm-doc> can parse the live tree
// (and so authors can use querySelector / dev tools to inspect the
// composition); their CSS rule (display: none) hides them.

export class GmDataElement extends HTMLElement {
  // Marker. No behavior — the CSS in style.ts handles display:none.
  // We keep the class so future hooks (attribute observers, etc.)
  // can land here without changing the element registration.
}
