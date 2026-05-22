// rAF-driven playhead. Wall-clock anchored so a paused-then-resumed
// session stays in sync with elapsed seconds rather than accumulating
// rAF drift. The single source of frame truth for every gm-* element
// inside a <gm-doc>.

export interface PlayheadOptions {
  fps: number;
  getDurationFrames(): number;
  onTick(frame: number): void;
  onEnd?(): void;
}

export interface Playhead {
  play(): void;
  pause(): void;
  toggle(): void;
  seek(frame: number): void;
  seekSeconds(seconds: number): void;
  setFps(fps: number): void;
  destroy(): void;
  readonly playing: boolean;
  readonly frame: number;
  readonly fps: number;
}

export function createPlayhead(opts: PlayheadOptions): Playhead {
  let fps = opts.fps;
  let playing = false;
  let frame = 0;
  let anchorWallMs = 0;
  let anchorFrame = 0;
  let rafId: number | null = null;

  function loop(now: number): void {
    if (!playing) return;
    const elapsedSec = (now - anchorWallMs) / 1000;
    const next = Math.floor(anchorFrame + elapsedSec * fps);
    const total = opts.getDurationFrames();
    if (total > 0 && next >= total) {
      frame = total - 1;
      opts.onTick(frame);
      playing = false;
      opts.onEnd?.();
      return;
    }
    if (next !== frame) {
      frame = next;
      opts.onTick(frame);
    }
    rafId = requestAnimationFrame(loop);
  }

  return {
    play(): void {
      if (playing) return;
      const total = opts.getDurationFrames();
      if (total > 0 && frame >= total - 1) {
        frame = 0;
        opts.onTick(frame);
      }
      playing = true;
      anchorWallMs = performance.now();
      anchorFrame = frame;
      rafId = requestAnimationFrame(loop);
    },
    pause(): void {
      if (!playing) return;
      playing = false;
      if (rafId !== null) {
        cancelAnimationFrame(rafId);
        rafId = null;
      }
    },
    toggle(): void {
      if (playing) this.pause();
      else this.play();
    },
    seek(target: number): void {
      const total = opts.getDurationFrames();
      const clamped = total > 0 ? Math.max(0, Math.min(total - 1, target)) : Math.max(0, target);
      frame = clamped;
      anchorWallMs = performance.now();
      anchorFrame = clamped;
      opts.onTick(frame);
    },
    seekSeconds(seconds: number): void {
      this.seek(Math.round(seconds * fps));
    },
    setFps(next: number): void {
      if (next <= 0 || !Number.isFinite(next)) return;
      anchorFrame = frame;
      anchorWallMs = performance.now();
      fps = next;
    },
    destroy(): void {
      playing = false;
      if (rafId !== null) {
        cancelAnimationFrame(rafId);
        rafId = null;
      }
    },
    get playing() { return playing; },
    get frame() { return frame; },
    get fps() { return fps; },
  };
}
