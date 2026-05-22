// wavelet preview <file.html> — open the composition in a local Vite
// dev server. Wraps the shared devServer helper.

import { resolve, dirname, basename } from "node:path";
import { existsSync } from "node:fs";
import { startDevServer } from "../devServer.mjs";

export async function preview(args) {
  const file = args[0];
  if (!file) {
    console.error("wavelet preview: missing file argument (e.g. `wavelet preview wavelet.html`)");
    return 1;
  }
  const abs = resolve(process.cwd(), file);
  if (!existsSync(abs)) {
    console.error(`wavelet preview: file not found: ${abs}`);
    return 1;
  }
  const compDir = dirname(abs);
  const compFile = basename(abs);
  const port = parsePortArg(args) ?? 5174;

  let server;
  try {
    server = await startDevServer({ compDir, port });
  } catch (e) {
    console.error(`wavelet preview: ${e.message}`);
    return 1;
  }
  const url = `http://localhost:${server.config.server.port}/${compFile}`;
  console.log(`wavelet preview: ${url}`);
  console.log(`               (Ctrl-C to stop)`);

  return new Promise((resolveFn) => {
    const shutdown = async () => {
      await server.close();
      resolveFn(0);
    };
    process.on("SIGINT", shutdown);
    process.on("SIGTERM", shutdown);
  });
}

function parsePortArg(args) {
  const idx = args.findIndex((a) => a === "--port" || a === "-p");
  if (idx === -1) return null;
  const v = Number(args[idx + 1]);
  return Number.isFinite(v) ? v : null;
}
