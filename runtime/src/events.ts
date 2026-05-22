// Event helpers a <gm-scene>'s inline <script> uses to hook into the
// playhead.
//
// The runtime fires two CustomEvents on every active scene container:
//
//   hf:ready  — once, when the scene mounts. Detail:
//                 { sceneId, fps, startMs, durationMs, frame: 0 }
//
//   hf:tick   — every rAF tick while the scene is active. Detail:
//                 { sceneId, fps, frame: localFrame, durationFrames }
//
// hf:ready is where the agent attaches GSAP / CSS / WebGL motion.
// hf:tick is for scrub-safe motion that needs explicit playhead
// awareness (rare; most authors just use GSAP timelines which advance
// off their own clock — see the caveat below).
//
// Caveat: GSAP runs on its own wall clock. When the wavelet transport
// pauses, GSAP keeps ticking. Authors that need scrub-perfect rewind
// should drive their timeline manually off hf:tick rather than letting
// GSAP free-run.

export interface ReadyDetail {
  sceneId: string;
  fps: number;
  startMs: number;
  durationMs: number;
}

export interface TickDetail {
  sceneId: string;
  fps: number;
  frame: number;
  durationFrames: number;
}

/**
 * Register a callback that fires once when the scene with the given
 * id mounts. If sceneId is omitted, fires on every scene's hf:ready
 * (useful when an outer <script> handles multiple scenes).
 *
 * Returns an unsubscribe function.
 */
export function onReady(
  callbackOrSceneId: string | ((detail: ReadyDetail, target: Element) => void),
  maybeCallback?: (detail: ReadyDetail, target: Element) => void,
): () => void {
  const sceneId = typeof callbackOrSceneId === "string" ? callbackOrSceneId : null;
  const cb = (typeof callbackOrSceneId === "string"
    ? maybeCallback
    : callbackOrSceneId) as ((detail: ReadyDetail, target: Element) => void) | undefined;
  if (!cb) throw new Error("onReady requires a callback");

  const handler = (e: Event) => {
    const ce = e as CustomEvent<ReadyDetail>;
    if (sceneId && ce.detail.sceneId !== sceneId) return;
    cb(ce.detail, e.target as Element);
  };
  document.addEventListener("hf:ready", handler);
  return () => document.removeEventListener("hf:ready", handler);
}

/**
 * Register a callback that fires every rAF tick while the scene is
 * active. If sceneId is omitted, fires for every active scene.
 *
 * Returns an unsubscribe function.
 */
export function onTick(
  callbackOrSceneId: string | ((detail: TickDetail, target: Element) => void),
  maybeCallback?: (detail: TickDetail, target: Element) => void,
): () => void {
  const sceneId = typeof callbackOrSceneId === "string" ? callbackOrSceneId : null;
  const cb = (typeof callbackOrSceneId === "string"
    ? maybeCallback
    : callbackOrSceneId) as ((detail: TickDetail, target: Element) => void) | undefined;
  if (!cb) throw new Error("onTick requires a callback");

  const handler = (e: Event) => {
    const ce = e as CustomEvent<TickDetail>;
    if (sceneId && ce.detail.sceneId !== sceneId) return;
    cb(ce.detail, e.target as Element);
  };
  document.addEventListener("hf:tick", handler);
  return () => document.removeEventListener("hf:tick", handler);
}

/**
 * Register a single paused GSAP timeline for a scene. The runtime
 * drives it via `tl.progress(localFrame / durationFrames)` every
 * tick. This is the **primary scene-timing pattern**: the timeline
 * lives inside the scene's window from progress 0 to 1, regardless
 * of the timeline's natural length. The scene's `duration=` is
 * authoritative.
 *
 *   <gm-scene id="hero" start="0.4s" duration="3s">
 *     <template>
 *       <h1 class="hero">Hello.</h1>
 *       <script>
 *         const tl = gsap.timeline({ paused: true });
 *         tl.from(".hero", { y: 60, opacity: 0, duration: 0.6 }, 0);
 *         tl.to(".hero", { y: -40, opacity: 0, duration: 0.4 }, ">2");
 *         wavelet.registerTimeline("hero", tl);
 *       </script>
 *     </template>
 *   </gm-scene>
 *
 * Cribbed from HyperFrames' window.__timelines convention — wavelet
 * accepts EITHER `wavelet.registerTimeline(id, tl)` OR direct
 * assignment to `window.__timelines[id] = tl` for HF-pattern
 * compatibility.
 *
 * The timeline must be paused (`{ paused: true }`) — the runtime
 * scrubs it via progress(); a playing timeline would race the
 * playhead.
 *
 * Scenes that don't register a timeline fall back to the
 * onReady/onTick model (still fully supported, useful for non-GSAP
 * motion like data-binding or audio-reactive visuals).
 */
export interface RegisteredTimeline {
  /** GSAP-shaped object — duck-typed to avoid a hard dep on gsap types. */
  progress(value: number): unknown;
  duration?(): number;
}

const TIMELINE_REGISTRY = new Map<string, RegisteredTimeline>();

export function registerTimeline(sceneId: string, tl: RegisteredTimeline): void {
  if (!sceneId || !tl) throw new Error("registerTimeline requires (sceneId, timeline)");
  TIMELINE_REGISTRY.set(sceneId, tl);
  // Also mirror onto window.__timelines so HF-style code works
  // unchanged (window.__timelines["id"] = tl).
  if (typeof window !== "undefined") {
    const w = window as any;
    w.__timelines = w.__timelines ?? {};
    w.__timelines[sceneId] = tl;
  }
}

export function getRegisteredTimeline(sceneId: string): RegisteredTimeline | null {
  // Check the runtime registry first, then the HF-compat window global.
  const direct = TIMELINE_REGISTRY.get(sceneId);
  if (direct) return direct;
  if (typeof window !== "undefined") {
    const w = window as any;
    const fromWindow = w.__timelines?.[sceneId];
    if (fromWindow && typeof fromWindow.progress === "function") {
      return fromWindow as RegisteredTimeline;
    }
  }
  return null;
}

export function clearRegisteredTimeline(sceneId: string): void {
  TIMELINE_REGISTRY.delete(sceneId);
  if (typeof window !== "undefined") {
    const w = window as any;
    if (w.__timelines) delete w.__timelines[sceneId];
  }
}

/** Internal: dispatch hf:ready on a scene container. */
export function dispatchReady(target: Element, detail: ReadyDetail): void {
  target.dispatchEvent(new CustomEvent("hf:ready", { bubbles: true, detail }));
}

/** Internal: dispatch hf:tick on a scene container. Also advances any
 *  registered timeline for the scene via tl.progress(). */
export function dispatchTick(target: Element, detail: TickDetail): void {
  const tl = getRegisteredTimeline(detail.sceneId);
  if (tl) {
    const total = Math.max(1, detail.durationFrames);
    const p = Math.max(0, Math.min(1, detail.frame / total));
    try { tl.progress(p); } catch { /* keep firing the event even if tl errors */ }
  }
  target.dispatchEvent(new CustomEvent("hf:tick", { bubbles: true, detail }));
}
