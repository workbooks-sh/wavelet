// Web Audio mixer for <gm-audio> cues.
//
// Ported from the wb-m6ny pass on packages/workbooks/.../audioMixer.ts,
// adapted to wavelet's flatter model: a `<gm-asset kind="audio">` is the
// source, a `<gm-audio>` cue schedules it. The mixer is playhead-
// driven: callers invoke `seek(frame)` / `play()` / `pause()`. No
// internal timer.
//
// Ducking: while any cue with `duck: dB` is active, every OTHER active
// cue's gain is multiplied by 10^(-dB/20). Reverts when the ducking
// cue ends.

import type { Asset, ResolvedAudioCue } from "./types";

interface CueRuntime {
  cue: ResolvedAudioCue;
  asset: Asset;
  source: AudioBufferSourceNode | null;
  gain: GainNode;
  panner: StereoPannerNode;
  buffer: AudioBuffer | null;
  lastApplied: number;
  active: boolean;
}

export interface MixerOptions {
  fps: number;
  baseUrl?: string | null;
}

export class AudioMixer {
  private ctx: AudioContext | null = null;
  private master: GainNode | null = null;
  private cues: CueRuntime[] = [];
  private fps = 30;
  private baseUrl: string | null = null;
  private destroyed = false;
  private playing = false;
  private bufferCache = new Map<string, Promise<AudioBuffer>>();

  constructor(opts: MixerOptions) {
    this.fps = Math.max(1, opts.fps);
    this.baseUrl = opts.baseUrl ?? null;
  }

  /**
   * Replace the cue set. Cues identified by (asset, startFrame, endFrame).
   * Existing cues that match are kept playing; the rest are torn down.
   */
  async load(cues: ResolvedAudioCue[], assets: Asset[]): Promise<void> {
    this.ensureContext();
    if (!this.ctx || !this.master) return;
    const assetById = new Map(assets.map((a) => [a.id, a]));

    const keyOf = (c: ResolvedAudioCue) => `${c.asset}@${c.startFrame}-${c.endFrame}`;
    const nextKeys = new Set(cues.map(keyOf));
    this.cues = this.cues.filter((r) => {
      if (nextKeys.has(keyOf(r.cue))) return true;
      this.stopCue(r);
      return false;
    });

    const existing = new Set(this.cues.map((r) => keyOf(r.cue)));
    for (const cue of cues) {
      if (existing.has(keyOf(cue))) continue;
      const asset = assetById.get(cue.asset);
      if (!asset) continue;
      const gain = this.ctx.createGain();
      const panner = this.ctx.createStereoPanner();
      gain.gain.value = 0;
      panner.pan.value = cue.pan ?? 0;
      gain.connect(panner).connect(this.master);
      const runtime: CueRuntime = {
        cue,
        asset,
        source: null,
        gain,
        panner,
        buffer: null,
        lastApplied: -1,
        active: false,
      };
      this.cues.push(runtime);
      void this.loadBuffer(asset).then((buf) => {
        if (this.destroyed) return;
        runtime.buffer = buf;
      });
    }
  }

  seek(frame: number): void {
    if (!this.ctx) return;
    const seconds = frame / this.fps;

    let activeDuckDb = 0;
    for (const r of this.cues) {
      const within = frame >= r.cue.startFrame && frame < r.cue.endFrame;
      if (within && (r.cue.duck ?? 0) > activeDuckDb) {
        activeDuckDb = r.cue.duck ?? 0;
      }
    }
    const duckLinear = activeDuckDb > 0 ? Math.pow(10, -activeDuckDb / 20) : 1;

    for (const r of this.cues) {
      const within = frame >= r.cue.startFrame && frame < r.cue.endFrame;
      if (!within) {
        if (r.active) this.stopCue(r);
        continue;
      }
      const cueSeconds = seconds - r.cue.startFrame / this.fps;
      const cueLen = (r.cue.endFrame - r.cue.startFrame) / this.fps;
      const baseVol = r.cue.volume ?? 1;
      const fadeIn = r.cue.fadeIn ?? 0;
      const fadeOut = r.cue.fadeOut ?? 0;
      let envelope = 1;
      if (fadeIn > 0 && cueSeconds < fadeIn) envelope *= cueSeconds / fadeIn;
      if (fadeOut > 0 && cueSeconds > cueLen - fadeOut) {
        envelope *= Math.max(0, (cueLen - cueSeconds) / fadeOut);
      }
      const isDuckSource = (r.cue.duck ?? 0) >= activeDuckDb && activeDuckDb > 0;
      const duckMul = isDuckSource ? 1 : duckLinear;
      r.gain.gain.value = baseVol * envelope * duckMul;
      if (this.playing && !r.active && r.buffer) {
        this.startCueAt(r, cueSeconds);
      } else if (r.active && r.source) {
        if (Math.abs(cueSeconds - r.lastApplied) > 0.1 && !this.playing) {
          this.stopCue(r);
        } else {
          r.lastApplied = cueSeconds;
        }
      }
    }
  }

  play(): void {
    this.ensureContext();
    if (!this.ctx) return;
    if (this.ctx.state === "suspended") void this.ctx.resume();
    this.playing = true;
  }

  pause(): void {
    this.playing = false;
    for (const r of this.cues) {
      if (r.active) this.stopCue(r);
    }
  }

  destroy(): void {
    this.destroyed = true;
    this.pause();
    if (this.ctx && this.ctx.state !== "closed") {
      void this.ctx.close().catch(() => undefined);
    }
    this.cues = [];
    this.ctx = null;
    this.master = null;
  }

  setMasterVolume(value: number, muted: boolean): void {
    if (!this.master) return;
    this.master.gain.value = muted ? 0 : Math.max(0, Math.min(1, value));
  }

  private ensureContext(): void {
    if (this.ctx || typeof window === "undefined") return;
    const Ctor = (window as any).AudioContext ?? (window as any).webkitAudioContext;
    if (!Ctor) return;
    this.ctx = new Ctor();
    this.master = this.ctx!.createGain();
    this.master!.gain.value = 1;
    this.master!.connect(this.ctx!.destination);
  }

  private async loadBuffer(asset: Asset): Promise<AudioBuffer> {
    const url = this.resolveUrl(asset.src);
    const existing = this.bufferCache.get(url);
    if (existing) return existing;
    const p = (async () => {
      const res = await fetch(url);
      if (!res.ok) throw new Error(`audio fetch ${url} → ${res.status}`);
      const arr = await res.arrayBuffer();
      return await new Promise<AudioBuffer>((resolve, reject) => {
        this.ctx!.decodeAudioData(arr, resolve, reject);
      });
    })();
    this.bufferCache.set(url, p);
    return p;
  }

  private resolveUrl(src: string): string {
    if (!this.baseUrl) return src;
    try {
      return new URL(src, new URL(this.baseUrl, window.location.href)).toString();
    } catch {
      return src;
    }
  }

  private startCueAt(r: CueRuntime, offsetSeconds: number): void {
    if (!this.ctx || !r.buffer) return;
    const node = this.ctx.createBufferSource();
    node.buffer = r.buffer;
    // Loop flag isn't part of ResolvedAudioCue today — wire later if needed.
    node.connect(r.gain);
    const safeOffset = Math.max(0, offsetSeconds % r.buffer.duration);
    try {
      node.start(0, safeOffset);
    } catch {
      // start() throws if called twice; safe to swallow.
    }
    r.source = node;
    r.lastApplied = offsetSeconds;
    r.active = true;
  }

  private stopCue(r: CueRuntime): void {
    if (r.source) {
      try { r.source.stop(); } catch { /* already stopped */ }
      try { r.source.disconnect(); } catch { /* disconnected */ }
    }
    r.source = null;
    r.active = false;
    r.lastApplied = -1;
  }
}
