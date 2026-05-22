// Shared Vite dev-server helper used by preview / verify / render.
//
// Starts an in-process Vite server with workspace aliases so the
// wavelet-runtime + wavelet-hyperframes packages resolve against the
// local src/ trees. Returns the server so callers can read its port
// and shut it down.

import { resolve, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import { existsSync, readFileSync } from "node:fs";

const here = dirname(fileURLToPath(import.meta.url));

export async function startDevServer({ compDir, port = 0 } = {}) {
  if (!compDir) throw new Error("startDevServer: compDir is required");
  const workspaceRoot = findWorkspaceRoot(here) ?? resolve(here, "..", "..", "..", "..");
  const runtimeSrc = resolve(workspaceRoot, "packages", "wavelet", "runtime", "src");
  const hfSrc = resolve(workspaceRoot, "packages", "wavelet", "hyperframes", "src");

  let createServer;
  try {
    ({ createServer } = await import("vite"));
  } catch {
    throw new Error("vite is not installed. Run `bun install` at the workspace root.");
  }

  // Virtual bootstrap module: serves `import "@work.books/wavelet-runtime";`
  // under a URL the template can reference via <script type="module"
  // src="/__gamut_bootstrap__">. This sidesteps Vite's html-proxy
  // pipeline (which 500s with "No matching HTML proxy module" on
  // inline `<script type="module">import ...</script>` blocks that
  // need alias resolution).
  const bootstrapPlugin = {
    name: "wavelet-bootstrap",
    resolveId(id) {
      if (id === "/__gamut_bootstrap__" || id === "__gamut_bootstrap__") {
        return "\0gamut-bootstrap";
      }
      return null;
    },
    load(id) {
      if (id === "\0gamut-bootstrap") {
        return `import "@work.books/wavelet-runtime";`;
      }
      return null;
    },
    configureServer(s) {
      // Allow direct GET /__gamut_bootstrap__ requests by mapping them
      // through Vite's transform pipeline as a fresh module.
      s.middlewares.use("/__gamut_bootstrap__", async (req, res, next) => {
        try {
          const result = await s.transformRequest("/__gamut_bootstrap__");
          if (!result) return next();
          res.setHeader("Content-Type", "application/javascript");
          res.end(result.code);
        } catch (e) {
          next(e);
        }
      });
    },
  };

  const server = await createServer({
    root: compDir,
    configFile: false,
    logLevel: "warn",
    plugins: [bootstrapPlugin],
    resolve: {
      alias: {
        "@work.books/wavelet-runtime": resolve(runtimeSrc, "runtime.ts"),
        "@work.books/wavelet-runtime/events": resolve(runtimeSrc, "events.ts"),
        "@work.books/wavelet-runtime/parser": resolve(runtimeSrc, "parser.ts"),
        "@work.books/wavelet-runtime/timeline": resolve(runtimeSrc, "timeline.ts"),
        "@work.books/wavelet-runtime/lint": resolve(runtimeSrc, "lint.ts"),
        "@work.books/wavelet-runtime/time": resolve(runtimeSrc, "time.ts"),
        "@work.books/wavelet-hyperframes": resolve(hfSrc, "index.ts"),
        "@work.books/wavelet-hyperframes/ready": resolve(hfSrc, "ready.ts"),
        "@work.books/wavelet-hyperframes/canvas": resolve(hfSrc, "canvas.ts"),
      },
    },
    server: {
      port,
      fs: { allow: [workspaceRoot, compDir] },
    },
  });
  await server.listen();
  return server;
}

function findWorkspaceRoot(start) {
  let dir = start;
  for (let i = 0; i < 12; i++) {
    const candidate = resolve(dir, "package.json");
    if (existsSync(candidate)) {
      try {
        const pkg = JSON.parse(readFileSync(candidate, "utf8"));
        if (Array.isArray(pkg.workspaces)) return dir;
      } catch { /* skip */ }
    }
    const parent = resolve(dir, "..");
    if (parent === dir) break;
    dir = parent;
  }
  return null;
}
