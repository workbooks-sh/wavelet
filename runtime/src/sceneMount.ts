// Mount + unmount a <gm-scene> into the viewport.
//
// Scenes carry either inline HTML (children of <gm-scene>) or a src=
// pointing at an external HTML file. Either way, we create a positioned
// container inside the viewport, drop the scene markup into it, then
// fire hf:ready so the scene's <script> can attach motion. The runtime
// re-executes any <script> tags in the scene markup (HTML parsers
// don't run scripts injected via innerHTML — we recreate them).

import { dispatchReady, dispatchTick, clearRegisteredTimeline } from "./events";
import type { ResolvedScene } from "./types";

export interface SceneMount {
  el: HTMLElement;
  scene: ResolvedScene;
  cleanup(): void;
}

export interface SceneMountContext {
  viewport: HTMLElement;
  fps: number;
  baseUrl: string | null;
  /** Z-index for this track's items. */
  zIndex: number;
}

export async function mountScene(
  scene: ResolvedScene,
  ctx: SceneMountContext,
): Promise<SceneMount> {
  const el = document.createElement("div");
  el.className = "gm-scene-mount";
  el.dataset.sceneId = scene.id;
  el.style.position = "absolute";
  el.style.inset = "0";
  el.style.zIndex = String(ctx.zIndex);
  if (scene.class) el.classList.add(...scene.class.split(/\s+/).filter(Boolean));
  if (scene.style) el.setAttribute("style", el.getAttribute("style") + ";" + scene.style);
  ctx.viewport.appendChild(el);

  // Fill content. Inline HTML wins (it's already in the DOM as the
  // gm-scene's children — but we use the parser-extracted string so
  // the original element stays hidden); src= triggers a fetch.
  if (scene.inlineHtml) {
    el.innerHTML = scene.inlineHtml;
    executeScripts(el, `scene:${scene.id}`);
  } else if (scene.src) {
    try {
      const url = resolveUrl(scene.src, ctx.baseUrl);
      const res = await fetch(url);
      if (res.ok) {
        const html = await res.text();
        el.innerHTML = html;
        executeScripts(el);
      } else {
        el.textContent = `[scene fetch failed: ${url} → ${res.status}]`;
      }
    } catch (e) {
      el.textContent = `[scene fetch error: ${e instanceof Error ? e.message : String(e)}]`;
    }
  }

  const startMs = (scene.startFrame / ctx.fps) * 1000;
  const durationFrames = scene.endFrame - scene.startFrame;
  const durationMs = (durationFrames / ctx.fps) * 1000;
  // Microtask gap so the scene's <script> finishes registering its
  // onReady handlers before we fire the event.
  await Promise.resolve();
  dispatchReady(el, {
    sceneId: scene.id,
    fps: ctx.fps,
    startMs,
    durationMs,
  });

  return {
    el,
    scene,
    cleanup() {
      // Drop any timeline registration first — if the scene re-mounts
      // later, the script will re-register a fresh paused timeline.
      // Leaving a stale registration around would cause the runtime
      // to drive a torn-down timeline.
      clearRegisteredTimeline(scene.id);
      el.remove();
    },
  };
}

/** Fire hf:tick on the mount with the current local frame. */
export function tickScene(mount: SceneMount, globalFrame: number, fps: number): void {
  const local = globalFrame - mount.scene.startFrame;
  if (local < 0) return;
  const durationFrames = mount.scene.endFrame - mount.scene.startFrame;
  dispatchTick(mount.el, {
    sceneId: mount.scene.id,
    fps,
    frame: local,
    durationFrames,
  });
}

/**
 * Re-execute any <script> tags inside an element. Necessary because
 * scripts injected via innerHTML don't run.
 */
function executeScripts(root: Element, debugLabel = "?"): void {
  const scripts = Array.from(root.querySelectorAll("script"));
  if (typeof window !== "undefined") {
    (window as any).__execLog = (window as any).__execLog ?? [];
    (window as any).__execLog.push({
      label: debugLabel,
      scriptCount: scripts.length,
      parentChain: scripts.map((s) => !!s.parentNode),
    });
  }
  for (const old of scripts) {
    const fresh = document.createElement("script");
    // Copy attributes (type, src, async, defer, …)
    for (const attr of Array.from(old.attributes)) {
      fresh.setAttribute(attr.name, attr.value);
    }
    fresh.textContent = old.textContent;
    // Force synchronous execution. Dynamically-inserted scripts
    // default to async=true (per the HTML spec); we need them to run
    // IN ORDER and SYNCHRONOUSLY so that scene scripts can call
    // wavelet.registerTimeline() in the same tick as the mount, before
    // the runtime fires its first hf:tick on the scene. Setting
    // async=false on a no-src script flips this.
    fresh.async = false;
    const parent = old.parentNode;
    if (parent) {
      old.remove();
      parent.appendChild(fresh);
    }
    // Belt + braces: also execute the script body directly via
    // indirect-eval. Inline classic scripts injected via DOM
    // sometimes get skipped by the browser when multiple scenes
    // mount in the same tick (observed: only the first scene's
    // script runs). Indirect eval guarantees execution at this
    // exact point. The DOM-insertion path above stays so the
    // script tag is visible in dev tools.
    const body = old.textContent ?? "";
    if (body.trim() && !old.getAttribute("src")) {
      try {
        // Indirect eval (avoids local scope, runs in global scope).
        // eslint-disable-next-line no-new-func
        (0, eval)(body);
      } catch (e) {
        console.error(`[wavelet sceneMount ${debugLabel}] script error:`, e);
      }
    }
  }
}

function resolveUrl(src: string, base: string | null): string {
  if (!base) return src;
  try {
    return new URL(src, new URL(base, window.location.href)).toString();
  } catch {
    return src;
  }
}
