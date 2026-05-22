// Unit tests for the rAF playhead. We stub requestAnimationFrame
// + performance.now so the loop is fully deterministic.

import { describe, expect, test, beforeEach, afterEach } from "bun:test";
import { createPlayhead } from "../src/playhead";

interface FakeClock {
  now: number;
  rafQueue: Array<(t: number) => void>;
  raf(cb: (t: number) => void): number;
  cancel(id: number): void;
  tickFrames(frameCount: number, fps: number): void;
  advanceMs(ms: number): void;
  drainRaf(): void;
}

function installFakeClock(): FakeClock {
  const clock: FakeClock = {
    now: 0,
    rafQueue: [],
    raf(cb) {
      clock.rafQueue.push(cb);
      return clock.rafQueue.length;
    },
    cancel(id) {
      const idx = id - 1;
      if (clock.rafQueue[idx]) clock.rafQueue[idx] = () => undefined;
    },
    tickFrames(frameCount, fps) {
      for (let i = 0; i < frameCount; i++) {
        clock.advanceMs(1000 / fps);
        clock.drainRaf();
      }
    },
    advanceMs(ms) {
      clock.now += ms;
    },
    drainRaf() {
      const pending = clock.rafQueue.slice();
      clock.rafQueue = [];
      for (const cb of pending) cb(clock.now);
    },
  };
  return clock;
}

let restoreRaf: (() => void) | null = null;
let restoreNow: (() => void) | null = null;
let clock: FakeClock;

beforeEach(() => {
  clock = installFakeClock();
  const realRaf = globalThis.requestAnimationFrame;
  const realCancel = globalThis.cancelAnimationFrame;
  const realNow = performance.now.bind(performance);
  globalThis.requestAnimationFrame = ((cb: (t: number) => void) => clock.raf(cb)) as any;
  globalThis.cancelAnimationFrame = ((id: number) => clock.cancel(id)) as any;
  (performance as any).now = () => clock.now;
  restoreRaf = () => {
    globalThis.requestAnimationFrame = realRaf;
    globalThis.cancelAnimationFrame = realCancel;
  };
  restoreNow = () => {
    (performance as any).now = realNow;
  };
});

afterEach(() => {
  restoreRaf?.();
  restoreNow?.();
});

describe("createPlayhead", () => {
  test("starts paused at frame 0", () => {
    const ph = createPlayhead({
      fps: 30,
      getDurationFrames: () => 60,
      onTick: () => {},
    });
    expect(ph.playing).toBe(false);
    expect(ph.frame).toBe(0);
  });

  test("play() advances the frame on rAF ticks", () => {
    const ticks: number[] = [];
    const ph = createPlayhead({
      fps: 30,
      getDurationFrames: () => 60,
      onTick: (f) => ticks.push(f),
    });
    ph.play();
    clock.tickFrames(5, 30);
    expect(ticks.length).toBeGreaterThanOrEqual(5);
    expect(ph.frame).toBeGreaterThanOrEqual(4);
    expect(ph.playing).toBe(true);
  });

  test("pause() stops advancing", () => {
    const ticks: number[] = [];
    const ph = createPlayhead({
      fps: 30,
      getDurationFrames: () => 60,
      onTick: (f) => ticks.push(f),
    });
    ph.play();
    clock.tickFrames(3, 30);
    const before = ph.frame;
    ph.pause();
    clock.tickFrames(10, 30);
    expect(ph.frame).toBe(before);
    expect(ph.playing).toBe(false);
  });

  test("seek() snaps and fires onTick", () => {
    const ticks: number[] = [];
    const ph = createPlayhead({
      fps: 30,
      getDurationFrames: () => 100,
      onTick: (f) => ticks.push(f),
    });
    ph.seek(42);
    expect(ph.frame).toBe(42);
    expect(ticks[ticks.length - 1]).toBe(42);
  });

  test("seek() clamps to [0, duration-1]", () => {
    const ph = createPlayhead({
      fps: 30,
      getDurationFrames: () => 60,
      onTick: () => {},
    });
    ph.seek(-10);
    expect(ph.frame).toBe(0);
    ph.seek(999);
    expect(ph.frame).toBe(59);
  });

  test("playback stops at the end and fires onEnd", () => {
    let ended = false;
    const ticks: number[] = [];
    const ph = createPlayhead({
      fps: 30,
      getDurationFrames: () => 10,
      onTick: (f) => ticks.push(f),
      onEnd: () => { ended = true; },
    });
    ph.play();
    clock.tickFrames(20, 30);
    expect(ended).toBe(true);
    expect(ph.playing).toBe(false);
    expect(ph.frame).toBe(9);
  });

  test("toggle() alternates between play and pause", () => {
    const ph = createPlayhead({
      fps: 30,
      getDurationFrames: () => 60,
      onTick: () => {},
    });
    ph.toggle();
    expect(ph.playing).toBe(true);
    ph.toggle();
    expect(ph.playing).toBe(false);
  });

  test("setFps re-anchors so playback continues smoothly", () => {
    const ph = createPlayhead({
      fps: 30,
      getDurationFrames: () => 600,
      onTick: () => {},
    });
    ph.play();
    clock.tickFrames(5, 30);
    const beforeFrame = ph.frame;
    ph.setFps(60);
    expect(ph.fps).toBe(60);
    clock.tickFrames(5, 60);
    expect(ph.frame).toBeGreaterThan(beforeFrame);
  });
});
