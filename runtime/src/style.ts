// Base CSS injected by the runtime when it registers the gm-* family.
// Visual identity (palette, type, motion) lives entirely in the
// author's stylesheet — this base only handles layout primitives
// (display: none for data-only elements, viewport positioning, chrome
// bar layout).

export const RUNTIME_CSS = `
  /* Data-only elements never render. */
  gm-asset, gm-composition, gm-timeline, gm-track, gm-clip,
  gm-audio, gm-shader, gm-include {
    display: none;
  }

  /* <gm-scene> stays hidden in the original DOM — its content is
     re-mounted into the viewport when active. */
  gm-scene { display: none; }

  /* <gm-adjustment> is metadata only. */
  gm-adjustment { display: none; }

  /* The doc itself is the player host. */
  gm-doc {
    display: block;
    position: relative;
    width: 100%;
    max-width: 100vw;
    background: var(--gm-doc-bg, #000);
    color: var(--gm-doc-fg, #fff);
    font-family: var(--gm-doc-font, system-ui, -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif);
    overflow: hidden;
  }

  .gm-stage {
    position: relative;
    width: 100%;
    aspect-ratio: var(--gm-aspect, 16 / 9);
    background: #000;
    overflow: hidden;
  }

  .gm-viewport {
    position: absolute;
    top: 0;
    left: 0;
    transform-origin: top left;
    /* width/height set by the runtime to the document's resolution. */
    background: #000;
  }

  .gm-scene-mount, .gm-clip-mount {
    pointer-events: none;
  }
  .gm-scene-mount * { pointer-events: auto; }

  .gm-chrome {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 10px 14px;
    background: var(--gm-chrome-bg, rgba(10, 10, 12, 0.92));
    border-top: 1px solid var(--gm-chrome-border, rgba(255, 255, 255, 0.12));
    user-select: none;
  }

  .gm-chrome button {
    appearance: none;
    background: rgba(255, 255, 255, 0.08);
    color: inherit;
    border: 1px solid rgba(255, 255, 255, 0.18);
    border-radius: 6px;
    padding: 6px 10px;
    font: inherit;
    font-size: 13px;
    cursor: pointer;
  }
  .gm-chrome button:hover { background: rgba(255, 255, 255, 0.14); }
  .gm-chrome button:focus-visible { outline: 2px solid #f59e0b; outline-offset: 2px; }

  .gm-scrub {
    flex: 1;
    height: 6px;
    background: rgba(255, 255, 255, 0.12);
    border-radius: 3px;
    position: relative;
    cursor: pointer;
  }
  .gm-scrub-fill {
    position: absolute;
    inset: 0;
    background: var(--gm-accent, #f59e0b);
    border-radius: 3px;
    width: 0%;
    pointer-events: none;
  }
  .gm-time {
    font-variant-numeric: tabular-nums;
    font-size: 12px;
    color: rgba(255, 255, 255, 0.72);
    min-width: 88px;
    text-align: right;
  }
`;

let injected = false;

/** Inject the base CSS into <head>. Idempotent. */
export function injectRuntimeStyle(): void {
  if (injected || typeof document === "undefined") return;
  const tag = document.createElement("style");
  tag.setAttribute("data-wavelet-runtime", "");
  tag.textContent = RUNTIME_CSS;
  document.head.appendChild(tag);
  injected = true;
}
