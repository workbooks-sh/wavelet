// wavelet — imperative editing CLI for wavelet compositions.
//
// Verb-dispatch entry. Each command lives in its own module so the
// surface stays grep-friendly and additions are local.

import { init } from "./commands/init.mjs";
import { inspect } from "./commands/inspect.mjs";
import { lint } from "./commands/lint.mjs";
import { preview } from "./commands/preview.mjs";
import { trim } from "./commands/trim.mjs";
import { split } from "./commands/split.mjs";
import { cut } from "./commands/cut.mjs";
import { concat } from "./commands/concat.mjs";
import { move } from "./commands/move.mjs";
import { verify } from "./commands/verify.mjs";
import { render } from "./commands/render.mjs";
import { renderV2 } from "./commands/render-v2.mjs";
import { transcribe } from "./commands/transcribe.mjs";
import { help } from "./commands/help.mjs";

const VERBS = {
  init,
  inspect,
  lint,
  preview,
  trim,
  split,
  cut,
  concat,
  move,
  verify,
  render,
  "render-v2": renderV2,
  transcribe,
  help,
  "--help": help,
  "-h": help,
};

export async function run(argv) {
  const [verb, ...rest] = argv;
  if (!verb) {
    await help([]);
    return 0;
  }
  const fn = VERBS[verb];
  if (!fn) {
    console.error(`wavelet: unknown command '${verb}'. Run \`wavelet help\` for the verb list.`);
    return 1;
  }
  return await fn(rest);
}
