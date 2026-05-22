// Frame resolver — walks the IR and produces absolute-frame positions
// for every track item. Missing required time values throw GamutError.
// No fallback defaults; the agent supplies every value or the lint
// catches it before render.

import { parseTime } from "./time";
import {
  GamutError,
  type Adjustment,
  type AudioCue,
  type Clip,
  type GamutDoc,
  type Include,
  type ResolvedAdjustment,
  type ResolvedAudioCue,
  type ResolvedClip,
  type ResolvedGamutDoc,
  type ResolvedInclude,
  type ResolvedScene,
  type ResolvedShader,
  type ResolvedTrack,
  type ResolvedTrackItem,
  type Scene,
  type Shader,
  type Track,
  type TrackItem,
} from "./types";

export function resolveTimeline(doc: GamutDoc): ResolvedGamutDoc {
  const fps = doc.fps;
  if (!Number.isInteger(fps) || fps <= 0) {
    throw new GamutError(`fps must be a positive integer; got ${doc.fps}`);
  }

  const durationFrames = parseTime(doc.timeline.duration, fps).frames;

  const tracks: ResolvedTrack[] = doc.timeline.tracks.map((t) =>
    resolveTrack(t, fps),
  );

  return {
    version: doc.version,
    fps,
    resolution: doc.resolution,
    aspect: doc.aspect,
    durationFrames,
    assets: doc.assets,
    compositions: doc.compositions,
    tracks,
  };
}

function resolveTrack(track: Track, fps: number): ResolvedTrack {
  const items = track.items.map((it) => resolveItem(it, fps, track.id));
  return {
    id: track.id,
    z: track.z,
    class: track.class,
    style: track.style,
    items,
  };
}

function resolveItem(item: TrackItem, fps: number, trackId: string): ResolvedTrackItem {
  const startFrame = parseTime(item.start, fps).frames;
  switch (item.kind) {
    case "clip":       return resolveClip(item, fps, startFrame, trackId);
    case "scene":      return resolveScene(item, fps, startFrame);
    case "audio":      return resolveAudio(item, fps, startFrame);
    case "shader":     return resolveShader(item, fps, startFrame);
    case "adjustment": return resolveAdjustment(item, fps, startFrame);
    case "include":    return resolveInclude(item, fps, startFrame);
  }
}

function resolveClip(clip: Clip, fps: number, startFrame: number, trackId: string): ResolvedClip {
  // A clip's frame extent comes from EITHER explicit duration OR the
  // in/out source range. If neither is present, it's an error — the
  // resolver does not pick a default.
  const sourceIn = clip.in ? parseTime(clip.in, fps).frames : undefined;
  const sourceOut = clip.out ? parseTime(clip.out, fps).frames : undefined;

  let endFrame: number;
  let sourceInFrame: number;
  let sourceOutFrame: number;

  if (clip.duration) {
    const durFrames = parseTime(clip.duration, fps).frames;
    endFrame = startFrame + durFrames;
    sourceInFrame = sourceIn ?? 0;
    sourceOutFrame = sourceOut ?? sourceInFrame + durFrames;
  } else if (sourceIn !== undefined && sourceOut !== undefined) {
    const durFrames = sourceOut - sourceIn;
    if (durFrames <= 0) {
      throw new GamutError(
        `<gm-clip asset="${clip.asset}"> on track '${trackId}': out (${clip.out}) must be after in (${clip.in})`,
      );
    }
    endFrame = startFrame + durFrames;
    sourceInFrame = sourceIn;
    sourceOutFrame = sourceOut;
  } else {
    throw new GamutError(
      `<gm-clip asset="${clip.asset}"> on track '${trackId}' requires either duration= or BOTH in= and out=`,
    );
  }

  return {
    kind: "clip",
    asset: clip.asset,
    startFrame,
    endFrame,
    sourceInFrame,
    sourceOutFrame,
    class: clip.class,
    style: clip.style,
  };
}

function resolveScene(scene: Scene, fps: number, startFrame: number): ResolvedScene {
  const endFrame = startFrame + parseTime(scene.duration, fps).frames;
  return {
    kind: "scene",
    id: scene.id,
    startFrame,
    endFrame,
    src: scene.src,
    inlineHtml: scene.inlineHtml,
    class: scene.class,
    style: scene.style,
  };
}

function resolveAudio(cue: AudioCue, fps: number, startFrame: number): ResolvedAudioCue {
  return {
    kind: "audio",
    asset: cue.asset,
    startFrame,
    endFrame: startFrame + parseTime(cue.duration, fps).frames,
    volume: cue.volume,
    pan: cue.pan,
    duck: cue.duck,
    fadeIn: cue.fadeIn,
    fadeOut: cue.fadeOut,
    loop: cue.loop,
    class: cue.class,
    style: cue.style,
  };
}

function resolveShader(shader: Shader, fps: number, startFrame: number): ResolvedShader {
  return {
    kind: "shader",
    lang: shader.lang,
    startFrame,
    endFrame: startFrame + parseTime(shader.duration, fps).frames,
    src: shader.src,
    inlineSource: shader.inlineSource,
    class: shader.class,
    style: shader.style,
  };
}

function resolveAdjustment(adj: Adjustment, fps: number, startFrame: number): ResolvedAdjustment {
  return {
    kind: "adjustment",
    filter: adj.filter,
    startFrame,
    endFrame: startFrame + parseTime(adj.duration, fps).frames,
    backdrop: adj.backdrop,
    blend: adj.blend,
    class: adj.class,
    style: adj.style,
  };
}

function resolveInclude(inc: Include, fps: number, startFrame: number): ResolvedInclude {
  return {
    kind: "include",
    startFrame,
    endFrame: startFrame + parseTime(inc.duration, fps).frames,
    ref: inc.ref,
    src: inc.src,
    class: inc.class,
    style: inc.style,
  };
}

/**
 * Flat list of every resolved item across all tracks. Useful for
 * "what's active at frame N" lookups without per-track recursion.
 */
export function itemsAtFrame(doc: ResolvedGamutDoc, frame: number): ResolvedTrackItem[] {
  const out: ResolvedTrackItem[] = [];
  for (const track of doc.tracks) {
    for (const item of track.items) {
      if (frame >= item.startFrame && frame < item.endFrame) out.push(item);
    }
  }
  return out;
}
