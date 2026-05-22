#!/usr/bin/env bun
import { run } from "../src/index.mjs";

run(process.argv.slice(2)).then(
  (code) => process.exit(code ?? 0),
  (err) => {
    console.error(err?.stack ?? err);
    process.exit(1);
  },
);
