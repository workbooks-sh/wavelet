// IR types for the wavelet composition format.
//
// The HTML is the source of truth — these types describe what the
// parser pulls out of the DOM. Every required field is required;
// missing fields surface as parse / lint errors, never as silent
// substitutions.
//
// Naming maps to the gm-* custom element family:
//   <gm-doc>        → GamutDoc
//   <gm-asset>      → Asset
//   <gm-timeline>   → Timeline
//   <gm-track>      → Track
//   <gm-clip>       → Clip
//   <gm-scene>      → Scene
//   <gm-audio>      → AudioCue
//   <gm-shader>     → Shader
//   <gm-adjustment> → Adjustment
//   <gm-include>    → Include

export type Fps = number;

export interface FrameTime {
  readonly frames: number;
}

export interface Resolution {
  readonly width: number;
  readonly height: number;
}

/**
 * Pass-through visual attrs carried by every renderable element.
 * Applied verbatim to the rendered DOM. No interpretation.
 */
export interface VisualAttrs {
  class?: string;
  style?: string;
}

export interface GamutDoc {
  /** Schema version. Currently "1". */
  version: string;
  fps: Fps;
  resolution: Resolution;
  /** "16:9", "9:16", "4:3", etc. Free-form — never enum-validated. */
  aspect: string;
  assets: Asset[];
  /** Document-scoped composition references (resolved via <gm-include ref=>). */
  compositions: CompositionDecl[];
  timeline: Timeline;
}

export interface Asset {
  id: string;
  /**
   * Free-form. Recognised by the runtime: "video", "audio", "image",
   * "transcript". Other values pass through untouched — author + their
   * scenes decide what to do with them.
   */
  kind: string;
  src: string;
}

export interface CompositionDecl {
  id: string;
  src: string;
}

export interface Timeline {
  id: string;
  duration: string;
  tracks: Track[];
}

export interface Track extends VisualAttrs {
  id: string;
  /** Composite order. Higher z renders on top. Required (no default). */
  z: number;
  items: TrackItem[];
}

export type TrackItem = Clip | Scene | AudioCue | Shader | Adjustment | Include;

/** Discriminator on TrackItem for runtime dispatch. */
export type TrackItemKind =
  | "clip"
  | "scene"
  | "audio"
  | "shader"
  | "adjustment"
  | "include";

interface TrackItemBase extends VisualAttrs {
  kind: TrackItemKind;
  start: string;
  /** Required when no other duration source (e.g. clip in/out). */
  duration?: string;
  /**
   * Optional `id=` attribute the author wrote on the source element.
   * Used by the CLI's wavelet move/split commands to find items by
   * stable identity. The linter cross-checks these against every
   * other id in the gm-doc subtree for HTML-spec uniqueness.
   */
  id?: string;
}

export interface Clip extends TrackItemBase {
  kind: "clip";
  asset: string;
  /** Source-side trim points (mutually compatible with duration). */
  in?: string;
  out?: string;
}

export interface Scene extends TrackItemBase {
  kind: "scene";
  duration: string;
  /**
   * Optional external scene source. When absent, the scene's content
   * is the children of the <gm-scene> element itself (inline scene).
   */
  src?: string;
  /**
   * Inline scene content as raw HTML string. Populated when src is
   * absent. The runtime mounts this verbatim into the scene container.
   */
  inlineHtml?: string;
  /** Stable identifier used by hf:ready / hf:tick event detail. */
  id: string;
}

export interface AudioCue extends TrackItemBase {
  kind: "audio";
  duration: string;
  asset: string;
  volume?: number;
  pan?: number;
  duck?: number;
  fadeIn?: number;
  fadeOut?: number;
  loop?: boolean;
}

export interface Shader extends TrackItemBase {
  kind: "shader";
  duration: string;
  /** "wgsl" or "glsl". Free-form; runtime dispatches. */
  lang: string;
  /** External shader file (mutually exclusive with inlineSource). */
  src?: string;
  /** Inline shader source as raw text (CDATA-like). */
  inlineSource?: string;
}

export interface Adjustment extends TrackItemBase {
  kind: "adjustment";
  duration: string;
  /** CSS filter shorthand. Pass-through. */
  filter: string;
  /** Optional CSS backdrop-filter. */
  backdrop?: string;
  /** Optional CSS mix-blend-mode. */
  blend?: string;
}

export interface Include extends TrackItemBase {
  kind: "include";
  duration: string;
  /**
   * Either ref= (pointing at a CompositionDecl id) OR src= (external
   * file path). Required: exactly one.
   */
  ref?: string;
  src?: string;
}

// ─── Resolved forms ──────────────────────────────────────────────────
// Produced by timeline.ts. All time strings are converted to absolute
// frame ranges (startFrame, endFrame). No fallback defaults — if a
// required time is missing, the resolver throws GamutError.

export interface ResolvedGamutDoc {
  version: string;
  fps: Fps;
  resolution: Resolution;
  aspect: string;
  durationFrames: number;
  assets: Asset[];
  compositions: CompositionDecl[];
  tracks: ResolvedTrack[];
}

export interface ResolvedTrack extends VisualAttrs {
  id: string;
  z: number;
  items: ResolvedTrackItem[];
}

export type ResolvedTrackItem =
  | ResolvedClip
  | ResolvedScene
  | ResolvedAudioCue
  | ResolvedShader
  | ResolvedAdjustment
  | ResolvedInclude;

interface ResolvedItemBase extends VisualAttrs {
  kind: TrackItemKind;
  startFrame: number;
  endFrame: number;
}

export interface ResolvedClip extends ResolvedItemBase {
  kind: "clip";
  asset: string;
  /** Source-side in/out in frames (defaults: 0 .. clip duration). */
  sourceInFrame: number;
  sourceOutFrame: number;
}

export interface ResolvedScene extends ResolvedItemBase {
  kind: "scene";
  id: string;
  src?: string;
  inlineHtml?: string;
}

export interface ResolvedAudioCue extends ResolvedItemBase {
  kind: "audio";
  asset: string;
  volume?: number;
  pan?: number;
  duck?: number;
  fadeIn?: number;
  fadeOut?: number;
  loop?: boolean;
}

export interface ResolvedShader extends ResolvedItemBase {
  kind: "shader";
  lang: string;
  src?: string;
  inlineSource?: string;
}

export interface ResolvedAdjustment extends ResolvedItemBase {
  kind: "adjustment";
  filter: string;
  backdrop?: string;
  blend?: string;
}

export interface ResolvedInclude extends ResolvedItemBase {
  kind: "include";
  ref?: string;
  src?: string;
}

// ─── Errors ──────────────────────────────────────────────────────────

export class GamutError extends Error {
  constructor(message: string, public readonly cause?: unknown) {
    super(message);
    this.name = "GamutError";
  }
}

/**
 * Linter finding. Distinct from GamutError — the linter collects every
 * issue it finds rather than throwing on the first one, so the agent
 * gets a complete diagnostic in one pass.
 */
export interface LintFinding {
  /** "error" blocks render; "warning" is informational. */
  severity: "error" | "warning";
  /** Stable code for tooling (e.g. "missing-required-attr"). */
  code: string;
  /** Human-readable message. */
  message: string;
  /** Source XPath-ish locator into the original HTML, when available. */
  at?: string;
}
