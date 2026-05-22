// wavelet verify <file.html> — render-query in headless browser.
//
// Replaces enforced templates with structural feedback. Loads the
// composition in a real Chromium, drives the playhead to per-scene
// keyframes, and reports what's actually visible. Catches the class
// of bug that the static linter can't see — animations that end on
// opacity:0, scenes whose subjects don't match a DOM node, broken
// asset 404s, scene scripts that throw.

import { resolve, dirname, basename } from "node:path";
import { existsSync } from "node:fs";
import { startDevServer } from "../devServer.mjs";
import { flag, hasFlag } from "../args.mjs";

export async function verify(args) {
  const file = args[0];
  if (!file) {
    console.error("wavelet verify: missing file argument (e.g. `wavelet verify wavelet.html`)");
    return 1;
  }
  const abs = resolve(process.cwd(), file);
  if (!existsSync(abs)) {
    console.error(`wavelet verify: file not found: ${abs}`);
    return 1;
  }
  const headed = hasFlag(args, "--headed");
  const timeoutMs = Number(flag(args, "--timeout") ?? 30000);

  const compDir = dirname(abs);
  const compFile = basename(abs);

  let server;
  try {
    server = await startDevServer({ compDir, port: 0 });
  } catch (e) {
    console.error(`wavelet verify: ${e.message}`);
    return 1;
  }

  let chromium;
  try {
    ({ chromium } = await import("playwright-core"));
  } catch {
    console.error("wavelet verify: playwright-core is not installed. Run `bun install` at the workspace root.");
    await server.close();
    return 1;
  }

  // Try to launch — playwright-core needs a browser; the user may
  // need to install one. We surface a useful error.
  let browser;
  try {
    browser = await chromium.launch({ headless: !headed });
  } catch (e) {
    console.error(
      `wavelet verify: could not launch Chromium (${e.message}).\n` +
      `  If this is the first time, install the browser binary with:\n` +
      `    bunx playwright install chromium`,
    );
    await server.close();
    return 1;
  }

  const url = `http://localhost:${server.config.server.port}/${compFile}`;
  const findings = [];
  const consoleErrors = [];
  const networkFails = [];
  let meta = { scenes: [] };

  try {
    // Install Playwright's fake clock on a fresh context so wall time
    // advances deterministically as we step through the timeline.
    // This makes verify see what viewers actually see at each frame
    // for BOTH wall-clock GSAP and wavelet.onTick-driven motion. Same
    // technique the render command uses.
    const context = await browser.newContext({ viewport: { width: 1280, height: 720 } });
    await context.clock.install({ time: new Date(0) });
    const page = await context.newPage();
    page.on("console", (msg) => {
      if (msg.type() === "error") consoleErrors.push(msg.text());
    });
    page.on("requestfailed", (req) => {
      const f = req.failure();
      const errText = f?.errorText ?? "?";
      // ERR_ABORTED on media-type requests is expected during
      // scrubbing — Chromium aborts in-flight range requests when
      // a <video> element is removed or remounted across frames.
      // Distinguish these from real failures.
      if (errText === "net::ERR_ABORTED" && (req.resourceType() === "media" || req.resourceType() === "image")) {
        return;
      }
      networkFails.push({ url: req.url(), failure: errText });
    });
    page.on("response", (res) => {
      if (res.status() === 404) {
        networkFails.push({ url: res.url(), failure: "HTTP 404" });
      }
    });

    console.log(`wavelet verify: loading ${url}`);
    await page.goto(url, { timeout: timeoutMs, waitUntil: "load" });

    // Wait for the runtime to register and gm-doc to finish parsing.
    await page.waitForFunction(() => {
      const doc = document.querySelector("gm-doc");
      return doc && typeof doc.seekFrame === "function" && doc.totalFrames > 0;
    }, { timeout: timeoutMs });

    meta = await page.evaluate(() => {
      const doc = document.querySelector("gm-doc");
      return {
        fps: doc.fps,
        totalFrames: doc.totalFrames,
        scenes: Array.from(document.querySelectorAll("gm-scene")).map((s) => ({
          id: s.getAttribute("id"),
          start: s.getAttribute("start"),
          duration: s.getAttribute("duration"),
        })),
      };
    });

    console.log(`               doc fps=${meta.fps} duration=${meta.totalFrames}f scenes=${meta.scenes.length}`);

    // Parse scene windows for sampling.
    const sceneWindows = await page.evaluate(() => {
      const doc = document.querySelector("gm-doc");
      const fps = doc.fps;
      const parse = (raw) => {
        const v = raw.trim();
        if (v.endsWith("f")) return Number(v.slice(0, -1)) || 0;
        if (v.endsWith("s")) return Math.round((Number(v.slice(0, -1)) || 0) * fps);
        const m = v.match(/^(\d{2}):(\d{2}):(\d{2}):(\d{2})$/);
        if (m) return (Number(m[1]) * 3600 + Number(m[2]) * 60 + Number(m[3])) * fps + Number(m[4]);
        return 0;
      };
      return Array.from(document.querySelectorAll("gm-scene")).map((s) => {
        const start = parse(s.getAttribute("start") ?? "0s");
        const dur = parse(s.getAttribute("duration") ?? "0s");
        return {
          id: s.getAttribute("id"),
          startFrame: start,
          endFrame: start + dur,
          midFrame: start + Math.floor(dur / 2),
        };
      });
    });

    // Build chronological sample queue: four points per scene (start,
    // mid, pre-end, end). Walking in time order means the fake clock
    // advances monotonically — wall-clock GSAP scenes get the
    // correct elapsed time at each sample, AND wavelet.onTick-driven
    // scenes get the right per-frame state via the seekFrame call.
    // The pre-end sample (200ms before scene-end) is compared
    // against end to detect static tails (hard-cut bugs).
    const preEndDeltaFrames = Math.max(1, Math.floor(meta.fps * 0.2));
    const sampleQueue = [];
    for (const sw of sceneWindows) {
      const endFrame = Math.max(sw.startFrame, sw.endFrame - 1);
      const preEndFrame = Math.max(sw.startFrame, endFrame - preEndDeltaFrames);
      sampleQueue.push({ sceneId: sw.id, frame: sw.startFrame, label: "start" });
      sampleQueue.push({ sceneId: sw.id, frame: sw.midFrame, label: "mid" });
      // Only add preEnd if it's distinct from start (very short scenes
      // would otherwise produce duplicate samples and a false-positive
      // hard-cut warning).
      if (preEndFrame > sw.midFrame) {
        sampleQueue.push({ sceneId: sw.id, frame: preEndFrame, label: "preEnd" });
      }
      sampleQueue.push({ sceneId: sw.id, frame: endFrame, label: "end" });
    }
    sampleQueue.sort((a, b) => a.frame - b.frame);

    // Each scene gets its three samples filed back into samplesByScene[id].
    const samplesByScene = new Map();
    let lastFrame = 0;
    for (const sp of sampleQueue) {
      const deltaFrames = sp.frame - lastFrame;
      if (deltaFrames > 0) {
        // Advance both wall time and rAF callbacks in lockstep.
        await page.clock.runFor(Math.round((deltaFrames / meta.fps) * 1000));
      }
      lastFrame = sp.frame;
      await page.evaluate((f) => {
        const doc = document.querySelector("gm-doc");
        doc.pause();
        doc.seekFrame(f);
      }, sp.frame);
      // Tiny wait so any synchronous tick handlers + microtasks settle
      // before sampling. 50ms is small enough that wall-clock GSAP
      // doesn't drift meaningfully between seek and sample.
      await page.waitForTimeout(50);
      const snapshot = await sampleSceneSnapshot(page, sp.sceneId);
      let bucket = samplesByScene.get(sp.sceneId);
      if (!bucket) {
        bucket = {};
        samplesByScene.set(sp.sceneId, bucket);
      }
      bucket[sp.label] = snapshot;
    }

    for (const sw of sceneWindows) {
      const bucket = samplesByScene.get(sw.id) ?? {};
      const sampleStart = bucket.start ?? emptySample();
      const sample = bucket.mid ?? emptySample();
      const sampleEnd = bucket.end ?? emptySample();

      if (!sample.mounted) {
        findings.push({
          severity: "error",
          code: "scene-not-mounted",
          message: `scene '${sw.id}': container missing at midpoint (frame ${sw.midFrame})`,
          at: `gm-scene[id=${sw.id}]`,
        });
        continue;
      }
      if (sample.contentLen < 8) {
        findings.push({
          severity: "warning",
          code: "scene-empty",
          message: `scene '${sw.id}': mounted but content length is ${sample.contentLen} (likely empty)`,
          at: `gm-scene[id=${sw.id}]`,
        });
      }
      if (sample.visibleRects === 0) {
        findings.push({
          severity: "error",
          code: "scene-not-visible",
          message: `scene '${sw.id}': zero elements with non-zero bounding-rect at midpoint`,
          at: `gm-scene[id=${sw.id}]`,
        });
      }
      // Compute the "has overlapping follower scene" predicate once;
      // it's reused by scene-ends-invisible (suppress when overlap
      // means the fade was intentional handoff), subject-ends-invisible
      // (same), and scene-hard-cut (warn ONLY when no overlap).
      const tailStart = sw.endFrame - preEndDeltaFrames;
      const hasOverlap = sceneWindows.some((other) => {
        if (other.id === sw.id) return false;
        const activeDuringTail = other.startFrame < sw.endFrame && other.endFrame > tailStart;
        if (!activeDuringTail) return false;
        const isContinuousBackground = other.startFrame <= sw.startFrame && other.endFrame >= sw.endFrame;
        return !isContinuousBackground;
      });

      // Animation actually animates? Compare layouts between start
      // and mid; if identical AND mid is fully opaque, that's fine
      // (static scene); if start equals end AND end is invisible,
      // that's likely an animation ending at opacity 0 — UNLESS
      // there's an overlapping follower scene, in which case fading
      // out is the intentional handoff pattern.
      if (sampleEnd.maxOpacity < 0.05 && !hasOverlap) {
        findings.push({
          severity: "warning",
          code: "scene-ends-invisible",
          message: `scene '${sw.id}': all elements have opacity < 0.05 at end frame (animation likely ends on opacity:0)`,
          at: `gm-scene[id=${sw.id}]`,
        });
      }
      // Per-element fade-out check — catches the case where ONE
      // named element (e.g. `.title`) ends invisible while others
      // stay visible. The agent likely wrote gsap.to(...,{opacity:0})
      // when they meant gsap.from(...). Also suppressed when there's
      // an overlapping follower scene (intentional exit fade).
      if (!hasOverlap) {
        for (const e of sampleEnd.elements) {
          if (e.opacity >= 0.05) continue;
          // Cross-reference against the start sample — only flag if the
          // element started visible (so genuinely-decorative fades-to-0
          // overlays aren't false positives).
          const startMatch = sampleStart.elements.find((s) => s.key === e.key);
          if (!startMatch || startMatch.opacity < 0.05) continue;
          findings.push({
            severity: "warning",
            code: "subject-ends-invisible",
            message: `scene '${sw.id}': element ${e.selector} ends at opacity ${e.opacity.toFixed(2)} (started visible at ${startMatch.opacity.toFixed(2)} — likely a fade-out where a fade-in was intended)`,
            at: `gm-scene[id=${sw.id}] ${e.selector}`,
          });
        }
      }
      // Hard-cut detection: if the scene's last 200ms shows no
      // rect/opacity change (preEnd === end), AND no following scene
      // overlaps this scene's last few frames, AND this scene isn't
      // the terminal scene (one that ends at timeline duration —
      // nothing to cut TO), the viewer sees a hard cut. The agent
      // likely wrote gsap.from() for entrance and forgot the
      // matching gsap.to() exit.
      const samplePreEnd = bucket.preEnd;
      // hasOverlap was computed above and reused by the
      // scene-ends-invisible / subject-ends-invisible suppressions.
      // Find the terminal frame: the latest endFrame across all
      // scenes (the last scene in time order). The terminal scene
      // gets a pass on hard-cut warnings — there's nothing to cut to.
      const terminalFrame = Math.max(
        ...sceneWindows.map((s) => s.endFrame),
        meta.totalFrames,
      );
      const isTerminalScene = sw.endFrame >= terminalFrame - preEndDeltaFrames;
      if (
        samplePreEnd &&
        sampleEnd.mounted &&
        samplePreEnd.rectSig === sampleEnd.rectSig &&
        samplePreEnd.maxOpacity === sampleEnd.maxOpacity &&
        !hasOverlap &&
        !isTerminalScene
      ) {
        findings.push({
          severity: "warning",
          code: "scene-hard-cut",
          message: `scene '${sw.id}': no motion in the final ${(preEndDeltaFrames / meta.fps).toFixed(2)}s and no following scene overlaps the tail — the viewer sees a hard cut. Add an exit tween (gsap.to(.subject, {opacity:0, …})) or extend the next scene to overlap this one's end.`,
          at: `gm-scene[id=${sw.id}]`,
        });
      }
      if (sample.rectSig === sampleStart.rectSig && sample.maxOpacity === sampleStart.maxOpacity) {
        findings.push({
          severity: "warning",
          code: "scene-no-motion",
          message: `scene '${sw.id}': no DOM rect or opacity change between start and midpoint (static scene or animation didn't fire)`,
          at: `gm-scene[id=${sw.id}]`,
        });
      }
    }

    for (const err of consoleErrors) {
      findings.push({
        severity: "error",
        code: "console-error",
        message: `runtime console.error: ${err.slice(0, 200)}`,
      });
    }
    for (const f of networkFails) {
      // Ignore favicon and Vite's dev-only probes.
      if (f.url.includes("/favicon.ico")) continue;
      if (f.url.includes("/@vite/") || f.url.includes("/@id/")) continue;
      findings.push({
        severity: "error",
        code: "asset-load-failed",
        message: `${f.url} → ${f.failure}`,
      });
    }
  } finally {
    await browser.close();
    await server.close();
  }

  if (findings.length === 0) {
    console.log(`${file}: verify clean — ${meta.scenes.length} scene${meta.scenes.length === 1 ? "" : "s"} sampled, no findings`);
    return 0;
  }

  const sorted = findings.sort((a, b) =>
    a.severity === b.severity ? 0 : a.severity === "error" ? -1 : 1,
  );
  for (const f of sorted) {
    const tag = f.severity === "error" ? "error " : "warn  ";
    const at = f.at ? `  (${f.at})` : "";
    console.log(`  ${tag} [${f.code}] ${f.message}${at}`);
  }
  const errors = findings.filter((f) => f.severity === "error").length;
  const warnings = findings.length - errors;
  console.log("");
  console.log(`${file}: ${errors} error${errors === 1 ? "" : "s"}, ${warnings} warning${warnings === 1 ? "" : "s"}`);
  return errors > 0 ? 1 : 0;
}

function emptySample() {
  return { mounted: false, contentLen: 0, visibleRects: 0, maxOpacity: 0, rectSig: "", elements: [] };
}

async function sampleSceneSnapshot(page, sceneId) {
  // Pure DOM-state snapshot. Assumes the caller has already advanced
  // the fake clock + seekFrame'd the runtime to the target frame.
  return await page.evaluate((sid) => {
    const mount = document.querySelector(`.gm-scene-mount[data-scene-id="${sid}"]`);
    if (!mount) {
      return { mounted: false, contentLen: 0, visibleRects: 0, maxOpacity: 0, rectSig: "", elements: [] };
    }
    // Only count visual elements (not <script>, <style>, <link>, <meta>
    // etc. — these always have getComputedStyle.opacity === '1' and
    // confuse the maxOpacity check).
    const NON_VISUAL = new Set(["SCRIPT", "STYLE", "LINK", "META", "TITLE", "TEMPLATE", "NOSCRIPT"]);
    const all = Array.from(mount.querySelectorAll("*")).filter(
      (el) => !NON_VISUAL.has(el.tagName),
    );
    if (all.length === 0) {
      return { mounted: true, contentLen: mount.innerHTML.length, visibleRects: 0, maxOpacity: 0, rectSig: "", elements: [] };
    }
    // Build a stable selector for each element so we can identify
    // per-element fade-out bugs in the findings report. Prefer id,
    // then tag+class, then nth-of-type within the mount.
    const selectorFor = (el) => {
      if (el.id) return `#${el.id}`;
      const cls = el.classList.length > 0 ? "." + Array.from(el.classList).join(".") : "";
      if (cls) return `${el.tagName.toLowerCase()}${cls}`;
      const siblings = Array.from(el.parentElement?.children ?? []).filter((s) => s.tagName === el.tagName);
      const idx = siblings.indexOf(el) + 1;
      return `${el.tagName.toLowerCase()}:nth-of-type(${idx})`;
    };
    let visible = 0;
    let maxOp = 0;
    const sigs = [];
    const elements = [];
    for (const el of all) {
      const r = el.getBoundingClientRect();
      if (r.width > 0 && r.height > 0) visible++;
      const op = Number(getComputedStyle(el).opacity);
      if (op > maxOp) maxOp = op;
      sigs.push(`${Math.round(r.x)},${Math.round(r.y)},${Math.round(r.width)}x${Math.round(r.height)},${op.toFixed(2)}`);
      elements.push({
        selector: selectorFor(el),
        opacity: op,
        width: Math.round(r.width),
        height: Math.round(r.height),
        // Identifying-token used to match the same element across
        // start/mid/end samples. Defaults to selector but falls back
        // to a position-in-tree string when selectors collide.
        key: selectorFor(el),
      });
    }
    return {
      mounted: true,
      contentLen: mount.innerHTML.length,
      visibleRects: visible,
      maxOpacity: maxOp,
      rectSig: sigs.join("|"),
      elements,
    };
  }, sceneId);
}

