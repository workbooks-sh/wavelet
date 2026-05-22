import { defineConfig } from "vite";
import { fileURLToPath } from "node:url";
import { dirname, resolve } from "node:path";

const here = dirname(fileURLToPath(import.meta.url));

export default defineConfig({
  root: here,
  // Serve TypeScript modules from the parent runtime/src tree.
  resolve: {
    alias: {
      "/src": resolve(here, "..", "src"),
      "@work.books/wavelet-runtime/events": resolve(here, "..", "src", "events.ts"),
      "@work.books/wavelet-hyperframes/ready": resolve(here, "..", "..", "hyperframes", "src", "ready.ts"),
      "@work.books/wavelet-hyperframes/canvas": resolve(here, "..", "..", "hyperframes", "src", "canvas.ts"),
    },
  },
  server: {
    fs: {
      // Allow Vite to read files outside demo/ — specifically ../src/*
      // and ../../hyperframes/src/*.
      allow: [resolve(here, "..", ".."), here],
    },
  },
});
