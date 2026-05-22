// wavelet render <file.html> -o <out.mp4>
//
// v0 render path: headless Chromium drives the comp frame-by-frame
// via Playwright's fake clock, screenshots each frame, pipes the
// PNG byte stream into ffmpeg for H.264 encoding.
//
// Audio is not rendered in v0 — gm-audio cues stay silent in the
// output. Filed as a follow-up; the native Rust render path
// (wb-lsw0) handles audio properly via symphonia + a custom mixer.

import { resolve, dirname, basename } from "node:path";
import { existsSync } from "node:fs";
import { spawn } from "node:child_process";
import { startDevServer } from "../devServer.mjs";
import { flag, hasFlag } from "../args.mjs";
import { renderAudio } from "../audioRender.mjs";

export async function render(args) {
  const file = args[0];
  const outArg = flag(args, "-o", "--out");
  if (!file || !outArg) {
    console.error("wavelet render: usage: wavelet render <file.html> -o <out.mp4> [--scale <n>] [--headed]");
    return 1;
  }
  const abs = resolve(process.cwd(), file);
  if (!existsSync(abs)) {
    console.error(`wavelet render: file not found: ${abs}`);
    return 1;
  }
  const outPath = resolve(process.cwd(), outArg);
  const compDir = dirname(abs);
  const compFile = basename(abs);
  const scale = Number(flag(args, "--scale") ?? 1);
  const headed = hasFlag(args, "--headed");

  let server;
  try {
    server = await startDevServer({ compDir, port: 0 });
  } catch (e) {
    console.error(`wavelet render: ${e.message}`);
    return 1;
  }

  let chromium;
  try {
    ({ chromium } = await import("playwright-core"));
  } catch {
    console.error("wavelet render: playwright-core is not installed. Run `bun install`.");
    await server.close();
    return 1;
  }

  let browser;
  try {
    browser = await chromium.launch({ headless: !headed });
  } catch (e) {
    console.error(
      `wavelet render: could not launch Chromium (${e.message}).\n` +
      `  Install with: bunx playwright install chromium`,
    );
    await server.close();
    return 1;
  }

  let code = 0;
  let audioOut = null;
  try {
    const url = `http://localhost:${server.config.server.port}/${compFile}`;
    console.log(`wavelet render: loading ${url}`);

    // Discover comp dimensions before opening the page so the viewport
    // matches the document's resolution. We open a probe page first
    // to read the gm-doc metadata, then close it and reopen at the
    // correct size.
    const probe = await browser.newPage({ viewport: { width: 1280, height: 720 } });
    await probe.goto(url, { waitUntil: "load", timeout: 60000 });
    await probe.waitForFunction(() => {
      const d = document.querySelector("gm-doc");
      return d && typeof d.seekFrame === "function" && d.totalFrames > 0;
    }, { timeout: 60000 });
    const meta = await probe.evaluate(() => {
      const d = document.querySelector("gm-doc");
      return {
        fps: d.fps,
        totalFrames: d.totalFrames,
        // Read declared resolution from <gm-doc resolution="WxH">.
        resolution: (() => {
          const r = d.getAttribute("resolution") ?? "1920x1080";
          const m = r.match(/^(\d+)x(\d+)$/);
          return m ? { width: Number(m[1]), height: Number(m[2]) } : { width: 1920, height: 1080 };
        })(),
      };
    });
    // Pull the resolved IR out of the live runtime so we can mix audio
    // in Node using the same envelope math the browser mixer uses.
    const resolvedDoc = await probe.evaluate(() => {
      const d = document.querySelector("gm-doc");
      return d.resolved ?? null;
    });
    await probe.close();

    const renderW = Math.round(meta.resolution.width * scale);
    const renderH = Math.round(meta.resolution.height * scale);
    console.log(`              fps=${meta.fps} frames=${meta.totalFrames} → ${renderW}×${renderH} → ${outPath}`);

    // Render audio in parallel with the video setup. The WAV is read
    // by ffmpeg below as a second input alongside the PNG stream.
    if (resolvedDoc) {
      try {
        audioOut = await renderAudio({
          resolvedDoc,
          compDir,
          totalFrames: meta.totalFrames,
          fps: meta.fps,
        });
        if (audioOut) {
          console.log(`              audio mixed → ${audioOut.wavPath}`);
        } else {
          console.log(`              (no audio cues — silent render)`);
        }
      } catch (e) {
        console.warn(`wavelet render: audio mix failed (${e.message ?? e}); proceeding with silent video.`);
      }
    }

    // Use Playwright's fake clock so wall time advances deterministically.
    const context = await browser.newContext({ viewport: { width: renderW, height: renderH } });
    await context.clock.install({ time: new Date(0) });
    const page = await context.newPage();
    await page.goto(url, { waitUntil: "load", timeout: 60000 });
    await page.waitForFunction(() => {
      const d = document.querySelector("gm-doc");
      return d && typeof d.seekFrame === "function" && d.totalFrames > 0;
    }, { timeout: 60000 });

    // Locate the gm-stage element so we screenshot just the canvas, not the chrome bar.
    const stage = await page.locator(".gm-stage").first();
    await stage.waitFor({ state: "attached", timeout: 30000 });

    // Spawn ffmpeg: read PNG byte stream from stdin, encode to MP4.
    // When audio is available, add the pre-mixed WAV as a second
    // input and encode it as AAC alongside the video stream.
    const ffArgs = [
      "-y",
      "-loglevel", "warning",
      "-f", "image2pipe",
      "-framerate", String(meta.fps),
      "-i", "-",
    ];
    if (audioOut) {
      ffArgs.push("-i", audioOut.wavPath);
    }
    ffArgs.push(
      "-c:v", "libx264",
      "-pix_fmt", "yuv420p",
      "-vf", `scale=${renderW}:${renderH}:flags=lanczos`,
    );
    if (audioOut) {
      ffArgs.push(
        "-c:a", "aac",
        "-b:a", "192k",
        "-shortest",
      );
    }
    ffArgs.push(
      "-movflags", "+faststart",
      outPath,
    );
    const ff = spawn("ffmpeg", ffArgs, { stdio: ["pipe", "inherit", "inherit"] });
    ff.on("error", (err) => {
      if (err.code === "ENOENT") {
        console.error("wavelet render: ffmpeg not found on PATH. Install ffmpeg first.");
      } else {
        console.error(`wavelet render: ffmpeg error: ${err.message}`);
      }
    });

    const msPerFrame = 1000 / meta.fps;
    const startedAt = Date.now();
    let lastPct = -1;

    for (let f = 0; f < meta.totalFrames; f++) {
      // Advance the page's fake clock by one frame's worth so GSAP +
      // CSS animations + audio scheduling all see consistent wall time.
      await page.clock.runFor(msPerFrame);
      await page.evaluate((frame) => {
        const d = document.querySelector("gm-doc");
        d.pause();
        d.seekFrame(frame);
      }, f);

      const buf = await stage.screenshot({ type: "png", omitBackground: false });
      if (!ff.stdin.destroyed) {
        const ok = ff.stdin.write(buf);
        if (!ok) {
          await new Promise((r) => ff.stdin.once("drain", r));
        }
      }

      const pct = Math.floor((f * 100) / meta.totalFrames);
      if (pct !== lastPct && pct % 5 === 0) {
        const elapsed = (Date.now() - startedAt) / 1000;
        const rate = (f + 1) / elapsed;
        process.stdout.write(`\r              ${pct}%  (${(f + 1).toString().padStart(4)}/${meta.totalFrames}f, ${rate.toFixed(1)} fps)  `);
        lastPct = pct;
      }
    }
    process.stdout.write(`\r              100%  (${meta.totalFrames}/${meta.totalFrames}f)                        \n`);

    ff.stdin.end();
    code = await new Promise((r) => ff.on("exit", (c) => r(c ?? 1)));
    if (code === 0) {
      const elapsed = (Date.now() - startedAt) / 1000;
      console.log(`wavelet render: ${outArg}  (${elapsed.toFixed(1)}s)`);
    } else {
      console.error(`wavelet render: ffmpeg exited with code ${code}`);
    }
  } finally {
    await browser.close();
    await server.close();
    if (audioOut) await audioOut.cleanupTmp();
  }

  return code;
}
