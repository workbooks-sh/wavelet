export * from "./types";
export { parseDocument, parseFromElement, __resetAnonCounter } from "./parser";
export { parseTime, framesToSeconds, frame } from "./time";
export { resolveTimeline, itemsAtFrame } from "./timeline";
export { lintDocument, summariseFindings, type LintOptions } from "./lint";

// Runtime — auto-registers gm-* custom elements on import.
export { register, GamutDoc, GmDataElement } from "./runtime";
export {
  onReady,
  onTick,
  registerTimeline,
  type ReadyDetail,
  type TickDetail,
  type RegisteredTimeline,
} from "./events";
export { createPlayhead, type Playhead, type PlayheadOptions } from "./playhead";
export { AudioMixer, type MixerOptions } from "./audioMixer";
