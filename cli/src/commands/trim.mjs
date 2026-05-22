// wavelet trim <asset> --in <t> --out <t> [-o <out>]
//
// Slices a media file via ffmpeg. Produces a new file; the source is
// not modified. No .html involvement — this command operates purely
// on the asset on disk.

import { spawn } from "node:child_process";
import { resolve, basename, extname, dirname, join } from "node:path";
import { existsSync } from "node:fs";
import { flag, positionals, timeToSeconds } from "../args.mjs";

export async function trim(args) {
  const [file] = positionals(args);
  const inT = flag(args, "--in");
  const outT = flag(args, "--out");
  const outFile = flag(args, "-o", "--out-file");
  if (!file || !inT || !outT) {
    console.error("wavelet trim: usage: wavelet trim <asset> --in <t> --out <t> [-o <out-file>]");
    console.error("  --in / --out accept seconds notation (e.g. '1.5s' or '00:00:01.500').");
    return 1;
  }
  const abs = resolve(process.cwd(), file);
  if (!existsSync(abs)) {
    console.error(`wavelet trim: file not found: ${abs}`);
    return 1;
  }
  let inSec, outSec;
  try {
    inSec = timeToSeconds(inT, "--in");
    outSec = timeToSeconds(outT, "--out");
  } catch (e) {
    console.error(`wavelet trim: ${e.message}`);
    return 1;
  }
  if (outSec <= inSec) {
    console.error(`wavelet trim: --out (${outSec}s) must be after --in (${inSec}s)`);
    return 1;
  }

  const target = outFile
    ? resolve(process.cwd(), outFile)
    : autoOutPath(abs, inSec, outSec);
  const duration = outSec - inSec;

  console.log(`wavelet trim: ${file} → ${target}`);
  console.log(`            in=${inSec}s out=${outSec}s duration=${duration}s`);

  return new Promise((resolveFn) => {
    const proc = spawn("ffmpeg", [
      "-y",
      "-ss", String(inSec),
      "-i", abs,
      "-t", String(duration),
      "-c", "copy",
      target,
    ], { stdio: ["ignore", "ignore", "inherit"] });
    proc.on("error", (err) => {
      if (err.code === "ENOENT") {
        console.error("wavelet trim: ffmpeg not found on PATH. Install ffmpeg first.");
      } else {
        console.error(`wavelet trim: ${err.message}`);
      }
      resolveFn(1);
    });
    proc.on("exit", (code) => {
      if (code === 0) console.log(`wavelet trim: done`);
      else console.error(`wavelet trim: ffmpeg exited with code ${code}`);
      resolveFn(code ?? 1);
    });
  });
}

function autoOutPath(src, inSec, outSec) {
  const dir = dirname(src);
  const ext = extname(src);
  const base = basename(src, ext);
  return join(dir, `${base}.trim_${inSec}-${outSec}${ext}`);
}
