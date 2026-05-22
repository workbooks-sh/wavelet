export async function help() {
  console.log(`wavelet — composition CLI

Usage: wavelet <verb> [args]

Read-only:
  init [name]              Scaffold a fresh composition directory.
  inspect <file.html>      Print the resolved timeline (frames + seconds).
  lint <file.html>         Structural validation — dangling refs, bad times,
                           schedule overflow, duplicate ids, missing files.
  preview <file.html>      Open the composition in a local dev server.
  verify <file.html>       Render-query in headless Chromium. Loads the comp,
                           scrubs to per-scene keyframes, samples DOM + opacity,
                           reports what's actually visible. Catches bugs the
                           static linter can't see (animations ending invisible,
                           404 assets, scene scripts that throw, etc).

Editing (mutates the <gm-*> element tree; head/styles/scripts preserved):
  trim <asset> --in <t> --out <t> [-o <out>]
                           Slice a media file via ffmpeg. No .html involved.
  split <html> <track> <time>
                           Split the item on <track> containing <time> into
                           two siblings at that time.
  cut <html> <track> --in <t> --out <t>
                           Remove a time range from <track>. Items wholly
                           inside the range are removed; straddling items
                           are shortened.
  move <html> --id <element-id> --to <time>
                           Re-time one element. Re-sorts siblings so source
                           order matches time order.
  concat <h1> <h2> [...] -o <out.html>
                           End-to-end concatenate multiple comps. Assets +
                           compositions dedup by id. Tracks merge by id.

Output:
  render <file.html> -o <out.mp4> [--scale <n>] [--headed]
                           Render the composition to an MP4 via headless
                           Chromium + ffmpeg. Frame-by-frame deterministic
                           via Playwright's fake clock. Audio is silent in v0
                           (filed under wb-4nlm).
  transcribe <audio> -o <words.json> [--model <size>]
                           Wrap a locally-installed Whisper (openai-whisper
                           or whisper.cpp) and emit the canonical transcript
                           shape wavelet scenes consume.

Each composition is a single .html file with <gm-*> custom elements.
See packages/wavelet/README.md for the format overview.`);
  return 0;
}
