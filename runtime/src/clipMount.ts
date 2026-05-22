// Mount + unmount a <gm-clip> into the viewport.
//
// Video clips: a <video> element seeks to the source-in frame plus
// (global - startFrame) of offset. When the transport is playing, the
// video plays at native speed; when paused, we pin currentTime each
// tick. Images: an <img> stays static while the clip is active.

import type { Asset, ResolvedClip } from "./types";

export interface ClipMount {
  el: HTMLElement;
  clip: ResolvedClip;
  asset: Asset;
  /** Pin to the global playhead. */
  tick(globalFrame: number, fps: number, playing: boolean): void;
  cleanup(): void;
}

export interface ClipMountContext {
  viewport: HTMLElement;
  baseUrl: string | null;
  zIndex: number;
}

export function mountClip(
  clip: ResolvedClip,
  asset: Asset,
  ctx: ClipMountContext,
): ClipMount {
  const url = resolveUrl(asset.src, ctx.baseUrl);
  const isVideo = asset.kind === "video";
  const el: HTMLElement = isVideo
    ? document.createElement("video")
    : document.createElement("img");
  el.className = "gm-clip-mount";
  (el as any).src = url;
  el.style.position = "absolute";
  el.style.inset = "0";
  el.style.width = "100%";
  el.style.height = "100%";
  el.style.objectFit = "cover";
  el.style.zIndex = String(ctx.zIndex);
  if (clip.class) el.classList.add(...clip.class.split(/\s+/).filter(Boolean));
  if (clip.style) el.setAttribute("style", el.getAttribute("style") + ";" + clip.style);

  if (isVideo) {
    const video = el as HTMLVideoElement;
    video.playsInline = true;
    video.muted = true; // master audio runs through the mixer
    video.preload = "auto";
  }
  ctx.viewport.appendChild(el);

  let lastSeekAt = -1;

  return {
    el,
    clip,
    asset,
    tick(globalFrame, fps, playing): void {
      if (!isVideo) return;
      const video = el as HTMLVideoElement;
      const local = globalFrame - clip.startFrame;
      if (local < 0) return;
      const target = (clip.sourceInFrame + local) / Math.max(1, fps);
      if (playing) {
        if (video.paused) {
          video.currentTime = target;
          const p = video.play();
          if (p && typeof p.catch === "function") p.catch(() => undefined);
          lastSeekAt = target;
        } else if (Math.abs(video.currentTime - target) > 0.08) {
          video.currentTime = target;
          lastSeekAt = target;
        }
      } else {
        if (!video.paused) video.pause();
        if (Math.abs(target - lastSeekAt) > 1 / fps) {
          video.currentTime = target;
          lastSeekAt = target;
        }
      }
    },
    cleanup(): void {
      if (isVideo) {
        const video = el as HTMLVideoElement;
        if (!video.paused) video.pause();
      }
      el.remove();
    },
  };
}

function resolveUrl(src: string, base: string | null): string {
  if (!base) return src;
  try {
    return new URL(src, new URL(base, window.location.href)).toString();
  } catch {
    return src;
  }
}
