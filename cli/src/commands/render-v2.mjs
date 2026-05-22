// wavelet render-v2 <ir.json|comp.wavelet.xml> -o <out.mp4>
//
// Phase 6: the v2 render path. Shells out to the Rust `wavelet-render` binary
// built from packages/wavelet-rust/wavelet-render-offline. The Rust binary owns
// the heavy lifting — RVST render kernel + Animato motion + rsmpeg encode +
// wgpu compositor + symphonia audio mix.
//
// This module is just argument parsing + binary location + child-process
// management. ~150 LOC of glue.
//
// Coexistence:
// - `wavelet render` (v1) still runs the Playwright + ffmpeg path. Existing.
// - `wavelet render-v2` (this) runs the Rust path. New.
// After Phase 8 ships and vsmoke migrates, v1 will be deprecated and
// `wavelet render` will route here.
//
// Input formats:
// - .json   — wavelet-ir JSON; pass through directly
// - .wavelet.xml — TODO Phase 8: compile to IR via the existing XML parser
//   then pass the temp JSON to the binary. For now: error with a clear hint.

import { resolve, dirname, basename } from "node:path";
import { existsSync } from "node:fs";
import { spawn } from "node:child_process";
import { fileURLToPath } from "node:url";
import { flag } from "../args.mjs";

const __dirname = dirname(fileURLToPath(import.meta.url));

export async function renderV2(args) {
  const file = args[0];
  const outArg = flag(args, "-o", "--out");
  if (!file || !outArg) {
    console.error("wavelet render-v2: usage: wavelet render-v2 <ir.json> -o <out.mp4>");
    console.error("");
    console.error("  Renders a wavelet-ir JSON composition to an MP4 via the");
    console.error("  Rust pipeline (RVST + Animato + rsmpeg + wgpu).");
    console.error("");
    console.error("  XML compositions are not yet supported on this path —");
    console.error("  use `wavelet inspect` to produce IR JSON, or wait for Phase 8.");
    return 1;
  }
  const abs = resolve(process.cwd(), file);
  if (!existsSync(abs)) {
    console.error(`wavelet render-v2: file not found: ${abs}`);
    return 1;
  }
  const outPath = resolve(process.cwd(), outArg);

  if (!abs.endsWith(".json")) {
    console.error(`wavelet render-v2: only .json IR input supported on this path.`);
    console.error(`  Got: ${abs}`);
    console.error(``);
    console.error(`  XML→IR compilation will land in Phase 8 (vsmoke migration).`);
    console.error(`  In the meantime, hand-write a wavelet-ir JSON or convert manually.`);
    return 1;
  }

  // Find the wavelet-render binary. Search order:
  //   1. GAMUT_RENDER_BIN env var (developer override)
  //   2. Relative to this module — monorepo dev layout
  //   3. $PATH lookup (installed via cargo install)
  let binary = process.env.GAMUT_RENDER_BIN;
  if (!binary) {
    const monorepoBin = resolve(
      __dirname,
      "../../../../packages/wavelet-rust/target/release/wavelet-render",
    );
    if (existsSync(monorepoBin)) {
      binary = monorepoBin;
    } else {
      // Fall through — let $PATH resolution happen at spawn time.
      binary = "wavelet-render";
    }
  }

  console.log(`wavelet render-v2: ${basename(abs)} → ${basename(outPath)}`);
  console.log(`  binary: ${binary}`);
  const startMs = Date.now();

  return new Promise((resolveProm) => {
    const child = spawn(binary, [abs, outPath], {
      stdio: "inherit",
    });
    child.on("error", (err) => {
      if (err.code === "ENOENT") {
        console.error(`wavelet render-v2: binary not found: ${binary}`);
        console.error(`  Build with: cd packages/wavelet-rust && cargo build --release --bin wavelet-render`);
        console.error(`  Or set GAMUT_RENDER_BIN to override the binary path.`);
        resolveProm(1);
      } else {
        console.error(`wavelet render-v2: spawn error: ${err.message}`);
        resolveProm(1);
      }
    });
    child.on("exit", (code) => {
      const dur = ((Date.now() - startMs) / 1000).toFixed(1);
      if (code === 0 && existsSync(outPath)) {
        console.log(`wavelet render-v2: done in ${dur}s — ${outPath}`);
      } else if (code !== 0) {
        console.error(`wavelet render-v2: binary exited with code ${code}`);
      }
      resolveProm(code ?? 1);
    });
  });
}
