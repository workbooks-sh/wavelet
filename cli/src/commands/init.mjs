// wavelet init [name] — scaffold a new wavelet composition directory.
//
// Copies templates/default into <cwd>/<name>/, substitutes __NAME__
// placeholders, creates assets/ and scenes/ subdirectories, and
// (when ffmpeg is on PATH) populates assets/ with a placeholder
// test-pattern video and sine-tone audio so the scaffolded
// wavelet.html renders end-to-end with zero extra steps.

import { mkdir, copyFile, readFile, writeFile, readdir, stat } from "node:fs/promises";
import { existsSync } from "node:fs";
import { resolve, join, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import { spawn } from "node:child_process";

const here = dirname(fileURLToPath(import.meta.url));
const TEMPLATE_DIR = resolve(here, "..", "..", "templates", "default");

export async function init(args) {
  const name = sanitiseName(args[0]) ?? "wavelet-composition";
  const target = resolve(process.cwd(), name);
  if (existsSync(target)) {
    console.error(`wavelet init: '${name}' already exists at ${target}`);
    return 1;
  }
  await mkdir(target, { recursive: true });
  await mkdir(join(target, "assets"), { recursive: true });
  await mkdir(join(target, "scenes"), { recursive: true });
  await copyTemplate(TEMPLATE_DIR, target, name);

  const mediaReport = await generatePlaceholderMedia(join(target, "assets"));

  console.log(`wavelet init: created ${name}/`);
  console.log("  - wavelet.html   the composition");
  console.log("  - styles.css   visual identity (yours to edit)");
  console.log(`  - assets/      ${mediaReport.summary}`);
  console.log("  - scenes/      external scene .html files (optional — scenes can stay inline)");
  if (mediaReport.note) {
    console.log("");
    console.log(`  Note: ${mediaReport.note}`);
  }
  console.log("");
  console.log(`Next: cd ${name} && wavelet preview wavelet.html`);
  return 0;
}

/**
 * Spawn ffmpeg to create a tiny test-pattern video + sine-tone audio
 * inside <assetsDir>. Returns a summary string for the init banner.
 * If ffmpeg isn't available, prints a clear hint instead of failing.
 */
async function generatePlaceholderMedia(assetsDir) {
  const ffmpegAvailable = await commandExists("ffmpeg");
  if (!ffmpegAvailable) {
    return {
      summary: "(empty — install ffmpeg and rerun, or drop your own media here)",
      note: "ffmpeg not found on PATH. Without it, the scaffold's gm-asset declarations will lint with missing-asset-file warnings until you drop real media into assets/.",
    };
  }
  const videoPath = join(assetsDir, "demo.mp4");
  const audioPath = join(assetsDir, "vo-tone.mp3");
  try {
    // 6-second calm dark gradient at 1920x1080, 30fps, H.264 +
    // faststart. The `gradients` filter slowly drifts between four
    // muted near-black tones — gentle visible motion (proves the
    // clip is playing) without the SMPTE-color-bar loudness of
    // testsrc2 (which fought every overlay scene in the scaffold
    // and read as a broken-demo rather than a working scaffold).
    // Replace this with your real footage.
    await runFfmpeg([
      "-y", "-loglevel", "error",
      "-f", "lavfi",
      "-i", "gradients=size=1920x1080:rate=30:duration=6:speed=0.02:c0=0x0f1116:c1=0x1d2330:c2=0x141821:c3=0x0f1116:nb_colors=4",
      "-c:v", "libx264",
      "-preset", "veryfast",
      "-pix_fmt", "yuv420p",
      "-movflags", "+faststart",
      videoPath,
    ]);
    // 6-second 440Hz sine tone — placeholder narration slot.
    await runFfmpeg([
      "-y", "-loglevel", "error",
      "-f", "lavfi",
      "-i", "sine=frequency=440:duration=6",
      "-ac", "2",
      "-b:a", "96k",
      audioPath,
    ]);
    return {
      summary: "demo.mp4 (test pattern) + vo-tone.mp3 (placeholder narration)",
      note: null,
    };
  } catch (e) {
    return {
      summary: "(media generation failed — drop your own files in assets/)",
      note: `ffmpeg ran into trouble: ${e.message ?? e}`,
    };
  }
}

function commandExists(cmd) {
  return new Promise((resolveFn) => {
    const proc = spawn("which", [cmd], { stdio: ["ignore", "pipe", "ignore"] });
    proc.on("error", () => resolveFn(false));
    proc.on("exit", (code) => resolveFn(code === 0));
  });
}

function runFfmpeg(args) {
  return new Promise((resolveFn, rejectFn) => {
    const proc = spawn("ffmpeg", args, { stdio: ["ignore", "ignore", "inherit"] });
    proc.on("error", rejectFn);
    proc.on("exit", (code) => {
      if (code === 0) resolveFn();
      else rejectFn(new Error(`ffmpeg exited ${code}`));
    });
  });
}

async function copyTemplate(src, dst, name) {
  const entries = await readdir(src);
  for (const entry of entries) {
    const sPath = join(src, entry);
    const dPath = join(dst, entry);
    const s = await stat(sPath);
    if (s.isDirectory()) {
      await mkdir(dPath, { recursive: true });
      await copyTemplate(sPath, dPath, name);
    } else if (isTextFile(entry)) {
      const text = await readFile(sPath, "utf8");
      await writeFile(dPath, text.replaceAll("__NAME__", name), "utf8");
    } else {
      await copyFile(sPath, dPath);
    }
  }
}

function isTextFile(filename) {
  return /\.(html?|css|js|mjs|cjs|md|json|txt|svg)$/i.test(filename);
}

function sanitiseName(raw) {
  if (!raw) return null;
  // Keep alphanumerics, dashes, underscores. No path separators.
  const cleaned = raw.replace(/[^a-zA-Z0-9_-]/g, "-").replace(/-+/g, "-").replace(/^-|-$/g, "");
  return cleaned.length > 0 ? cleaned : null;
}
