// <gm-doc> — the orchestrator. Parses its own subtree, resolves the
// timeline, mounts a viewport sized to the document's resolution,
// renders the player chrome, drives the playhead, and dispatches
// scene mount/unmount events as the playhead moves.
//
// All other gm-* elements are either data-only (hidden, parsed by
// this) or render lazily when their time becomes active (handled by
// mount helpers in ../sceneMount.ts, ../clipMount.ts, etc).

import { parseFromElement } from "../parser";
import { resolveTimeline } from "../timeline";
import { createPlayhead, type Playhead } from "../playhead";
import { AudioMixer } from "../audioMixer";
import { mountScene, tickScene, type SceneMount } from "../sceneMount";
import { mountClip, type ClipMount } from "../clipMount";
import { mountInclude } from "../includeMount";
import { mountShader } from "../shaderMount";
import { createAdjustmentApplicator, type AdjustmentApplicator } from "../adjustmentMount";
import { injectRuntimeStyle } from "../style";
import type {
  Asset,
  ResolvedAdjustment,
  ResolvedAudioCue,
  ResolvedClip,
  ResolvedGamutDoc,
  ResolvedScene,
  ResolvedTrack,
  ResolvedTrackItem,
} from "../types";

interface ItemMount {
  cleanup(): void;
  /** Tick callback for items that need per-frame updates (clips, scenes). */
  tick?(globalFrame: number, fps: number, playing: boolean): void;
}

export class GamutDoc extends HTMLElement {
  private resolved: ResolvedGamutDoc | null = null;
  private playhead: Playhead | null = null;
  private mixer: AudioMixer | null = null;
  private stage: HTMLElement | null = null;
  private viewport: HTMLElement | null = null;
  private chrome: HTMLElement | null = null;
  private scrubFill: HTMLElement | null = null;
  private timeLabel: HTMLElement | null = null;
  private playBtn: HTMLButtonElement | null = null;
  private adjustments: AdjustmentApplicator | null = null;
  private mounts = new Map<string, ItemMount>();
  /** Keys for which spawnMount is in-flight. Prevents double-mount when
   *  a scene's async fetch hasn't resolved by the next refresh tick. */
  private pending = new Set<string>();
  private assetById = new Map<string, Asset>();
  private resizeObserver: ResizeObserver | null = null;

  connectedCallback(): void {
    injectRuntimeStyle();
    try {
      const parsed = parseFromElement(this);
      this.resolved = resolveTimeline(parsed);
    } catch (e) {
      this.renderError(e instanceof Error ? e.message : String(e));
      return;
    }
    this.assetById = new Map(this.resolved.assets.map((a) => [a.id, a]));
    // Embedded instances (mounted by <gm-include>) skip chrome and
    // skip the rAF playhead — the parent doc drives their playhead
    // via tick() so they stay in sync. The embedded doc still
    // builds its own stage + viewport + adjustments + mixer so its
    // tracks render in isolation.
    const embedded = this.hasAttribute("data-embedded");
    if (!embedded) this.buildChrome();
    this.buildStage();
    this.scaleViewport();
    this.mixer = new AudioMixer({ fps: this.resolved.fps });
    if (!embedded) this.attachPlayhead();
    this.refresh(0);
    this.installResizeObserver();
  }

  disconnectedCallback(): void {
    this.playhead?.destroy();
    this.playhead = null;
    this.mixer?.destroy();
    this.mixer = null;
    for (const m of this.mounts.values()) m.cleanup();
    this.mounts.clear();
    this.resizeObserver?.disconnect();
    this.resizeObserver = null;
    this.adjustments?.reset();
    this.adjustments = null;
  }

  // ─── Public transport surface (useful for cw verify / cw render) ───

  play(): void {
    this.playhead?.play();
    this.mixer?.play();
    this.updatePlayBtn();
  }
  pause(): void {
    this.playhead?.pause();
    this.mixer?.pause();
    this.updatePlayBtn();
  }
  seekFrame(frame: number): void {
    this.playhead?.seek(frame);
  }
  get currentFrame(): number {
    return this.playhead?.frame ?? 0;
  }
  get totalFrames(): number {
    return this.resolved?.durationFrames ?? 0;
  }
  get fps(): number {
    return this.resolved?.fps ?? 30;
  }

  // ─── Internals ────────────────────────────────────────────────────

  private buildStage(): void {
    if (!this.resolved) return;
    const stage = document.createElement("div");
    stage.className = "gm-stage";
    stage.style.setProperty(
      "--gm-aspect",
      `${this.resolved.resolution.width} / ${this.resolved.resolution.height}`,
    );
    const viewport = document.createElement("div");
    viewport.className = "gm-viewport";
    viewport.style.width = `${this.resolved.resolution.width}px`;
    viewport.style.height = `${this.resolved.resolution.height}px`;
    stage.appendChild(viewport);
    this.insertBefore(stage, this.chrome);
    this.stage = stage;
    this.viewport = viewport;
    this.adjustments = createAdjustmentApplicator(viewport);
  }

  private buildChrome(): void {
    const chrome = document.createElement("div");
    chrome.className = "gm-chrome";
    chrome.setAttribute("data-print-hidden", "");

    const btn = document.createElement("button");
    btn.type = "button";
    btn.textContent = "▶";
    btn.setAttribute("aria-label", "Play");
    btn.addEventListener("click", () => this.toggle());
    this.playBtn = btn;

    const scrub = document.createElement("div");
    scrub.className = "gm-scrub";
    scrub.addEventListener("click", (e) => this.onScrubClick(e));
    const fill = document.createElement("div");
    fill.className = "gm-scrub-fill";
    scrub.appendChild(fill);
    this.scrubFill = fill;

    const time = document.createElement("div");
    time.className = "gm-time";
    time.textContent = "0:00 / 0:00";
    this.timeLabel = time;

    chrome.appendChild(btn);
    chrome.appendChild(scrub);
    chrome.appendChild(time);
    this.appendChild(chrome);
    this.chrome = chrome;
  }

  private attachPlayhead(): void {
    if (!this.resolved) return;
    this.playhead = createPlayhead({
      fps: this.resolved.fps,
      getDurationFrames: () => this.resolved?.durationFrames ?? 0,
      onTick: (frame) => this.refresh(frame),
      onEnd: () => this.updatePlayBtn(),
    });
  }

  private toggle(): void {
    if (this.playhead?.playing) this.pause();
    else this.play();
  }

  private updatePlayBtn(): void {
    if (!this.playBtn) return;
    const playing = this.playhead?.playing ?? false;
    this.playBtn.textContent = playing ? "❚❚" : "▶";
    this.playBtn.setAttribute("aria-label", playing ? "Pause" : "Play");
  }

  private onScrubClick(e: MouseEvent): void {
    if (!this.resolved) return;
    const rect = (e.currentTarget as HTMLElement).getBoundingClientRect();
    const ratio = (e.clientX - rect.left) / rect.width;
    const target = Math.round(ratio * (this.resolved.durationFrames - 1));
    this.seekFrame(target);
  }

  /** Drive the timeline forward to `frame`. Mount/unmount items, update audio, recompose adjustments. */
  private refresh(frame: number): void {
    if (!this.resolved || !this.viewport) return;

    // Build the per-track active sets in z order.
    const tracksByZ = [...this.resolved.tracks].sort((a, b) => a.z - b.z);
    const seen = new Set<string>();
    const activeAdjustments: ResolvedAdjustment[] = [];
    const activeAudio: ResolvedAudioCue[] = [];

    for (const track of tracksByZ) {
      const zBase = track.z * 1000;
      let itemIdx = 0;
      for (const item of track.items) {
        const within = frame >= item.startFrame && frame < item.endFrame;
        const key = mountKey(track, item, itemIdx);
        itemIdx++;
        if (item.kind === "adjustment" && within) activeAdjustments.push(item);
        if (item.kind === "audio" && within) activeAudio.push(item);
        if (item.kind !== "scene" && item.kind !== "clip" && item.kind !== "include" && item.kind !== "shader") continue;
        if (within) {
          seen.add(key);
          if (!this.mounts.has(key) && !this.pending.has(key)) {
            this.spawnMount(track, item, zBase + itemIdx, key);
          }
          const m = this.mounts.get(key);
          if (item.kind === "include") {
            // Includes get the LOCAL frame so the embedded gm-doc runs
            // its own timeline from 0..its-own-duration.
            const local = frame - item.startFrame;
            m?.tick?.(local, this.resolved.fps, this.playhead?.playing ?? false);
          } else {
            m?.tick?.(frame, this.resolved.fps, this.playhead?.playing ?? false);
          }
        }
      }
    }

    // Tear down anything no longer active.
    for (const [key, m] of this.mounts) {
      if (!seen.has(key)) {
        m.cleanup();
        this.mounts.delete(key);
      }
    }

    // Adjustments compose in z order.
    this.adjustments?.apply(activeAdjustments);

    // Audio mixer absorbs all active cues at once.
    void this.mixer?.load(activeAudio, this.resolved.assets);
    this.mixer?.seek(frame);

    // Tick every mounted scene so hf:tick fires.
    for (const m of this.mounts.values()) {
      if ((m as any).sceneMount) {
        tickScene((m as any).sceneMount as SceneMount, frame, this.resolved.fps);
      }
    }

    this.updateScrub(frame);
  }

  private spawnMount(
    track: ResolvedTrack,
    item: ResolvedTrackItem,
    zIndex: number,
    key: string,
  ): void {
    if (!this.viewport || !this.resolved) return;
    if (item.kind === "scene") {
      this.pending.add(key);
      void mountScene(item, {
        viewport: this.viewport,
        fps: this.resolved.fps,
        baseUrl: this.baseHrefForRelativeFetches(),
        zIndex,
      }).then((mount) => {
        this.pending.delete(key);
        // The mount may arrive after the playhead has moved past the
        // scene window — guard with a fresh range check.
        const currentFrame = this.playhead?.frame ?? 0;
        if (currentFrame < item.startFrame || currentFrame >= item.endFrame) {
          mount.cleanup();
          return;
        }
        const itemMount: ItemMount = {
          cleanup: () => mount.cleanup(),
        };
        (itemMount as any).sceneMount = mount;
        this.mounts.set(key, itemMount);
        // Fire an immediate hf:tick at the current playhead frame so
        // any registered timeline (wavelet.registerTimeline + the
        // dispatchTick → tl.progress() path) gets its first
        // progress() call right after the scene's inline script
        // registered the timeline. Without this the timeline sits at
        // its default state until the next rAF tick — which never
        // arrives when the playhead is paused (e.g. during wavelet
        // verify sampling).
        if (this.resolved) {
          tickScene(mount, currentFrame, this.resolved.fps);
        }
      }).catch(() => {
        this.pending.delete(key);
      });
    } else if (item.kind === "clip") {
      const asset = this.assetById.get(item.asset);
      if (!asset) return;
      const mount = mountClip(item, asset, {
        viewport: this.viewport,
        baseUrl: this.baseHrefForRelativeFetches(),
        zIndex,
      });
      const itemMount: ItemMount = {
        cleanup: () => mount.cleanup(),
        tick: (frame, fps, playing) => mount.tick(frame, fps, playing),
      };
      this.mounts.set(key, itemMount);
    } else if (item.kind === "shader") {
      this.pending.add(key);
      void mountShader(item, {
        viewport: this.viewport,
        baseUrl: this.baseHrefForRelativeFetches(),
        zIndex,
      }).then((mount) => {
        this.pending.delete(key);
        const currentFrame = this.playhead?.frame ?? 0;
        if (currentFrame < item.startFrame || currentFrame >= item.endFrame) {
          mount.cleanup();
          return;
        }
        const itemMount: ItemMount = {
          cleanup: () => mount.cleanup(),
          tick: (globalFrame, fps, playing) => mount.tick(globalFrame, fps, playing),
        };
        this.mounts.set(key, itemMount);
      }).catch(() => {
        this.pending.delete(key);
      });
    } else if (item.kind === "include") {
      this.pending.add(key);
      void mountInclude(item, {
        viewport: this.viewport,
        baseUrl: this.baseHrefForRelativeFetches(),
        zIndex,
        compositionDecls: this.resolved.compositions,
        ancestry: this.includeAncestry(),
      }).then((mount) => {
        this.pending.delete(key);
        const currentFrame = this.playhead?.frame ?? 0;
        if (currentFrame < item.startFrame || currentFrame >= item.endFrame) {
          mount.cleanup();
          return;
        }
        const itemMount: ItemMount = {
          cleanup: () => mount.cleanup(),
          tick: (localFrame, fps, playing) => mount.tick(localFrame, fps, playing),
        };
        this.mounts.set(key, itemMount);
      }).catch(() => {
        this.pending.delete(key);
      });
    }
  }

  /**
   * Walk up the DOM for any ancestor with data-include-ancestry,
   * extract its URL set. Used to detect include cycles — if a
   * child include's target URL is already in the ancestry stack,
   * the include refuses to mount.
   */
  private includeAncestry(): Set<string> {
    const out = new Set<string>();
    const raw = this.getAttribute("data-include-ancestry");
    if (raw) {
      for (const url of raw.split("|")) {
        if (url) out.add(url);
      }
    }
    return out;
  }

  private baseHrefForRelativeFetches(): string {
    if (typeof window === "undefined") return "";
    return window.location.href;
  }

  private updateScrub(frame: number): void {
    if (!this.resolved || !this.scrubFill || !this.timeLabel) return;
    const total = this.resolved.durationFrames;
    const ratio = total > 0 ? Math.min(1, frame / Math.max(1, total - 1)) : 0;
    this.scrubFill.style.width = `${(ratio * 100).toFixed(3)}%`;
    this.timeLabel.textContent = `${formatTime(frame, this.resolved.fps)} / ${formatTime(total, this.resolved.fps)}`;
  }

  private installResizeObserver(): void {
    if (typeof ResizeObserver === "undefined" || !this.stage) return;
    this.resizeObserver = new ResizeObserver(() => this.scaleViewport());
    this.resizeObserver.observe(this.stage);
  }

  private scaleViewport(): void {
    if (!this.stage || !this.viewport || !this.resolved) return;
    const rect = this.stage.getBoundingClientRect();
    if (rect.width === 0) return;
    const scale = rect.width / this.resolved.resolution.width;
    this.viewport.style.transform = `scale(${scale})`;
  }

  private renderError(message: string): void {
    const pre = document.createElement("pre");
    pre.style.color = "#ff8a8a";
    pre.style.background = "rgba(0,0,0,0.6)";
    pre.style.padding = "12px 16px";
    pre.style.font = "12px ui-monospace, SFMono-Regular, Menlo, monospace";
    pre.style.margin = "0";
    pre.style.whiteSpace = "pre-wrap";
    pre.textContent = `wavelet: ${message}`;
    this.appendChild(pre);
  }
}

function mountKey(track: ResolvedTrack, item: ResolvedTrackItem, idx: number): string {
  return `${track.id}:${item.kind}:${idx}:${item.startFrame}-${item.endFrame}`;
}

function formatTime(frame: number, fps: number): string {
  const totalSeconds = Math.max(0, frame) / fps;
  const m = Math.floor(totalSeconds / 60);
  const s = Math.floor(totalSeconds % 60);
  return `${m}:${s.toString().padStart(2, "0")}`;
}
