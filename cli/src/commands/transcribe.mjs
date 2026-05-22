// wavelet transcribe <audio> -o <words.json>
//
// Shells out to a locally-installed Whisper implementation to
// transcribe an audio file. Emits the canonical transcript shape
// wavelet runtime + scenes consume:
//
//   [
//     { "text": "Hello", "start_ms": 0,    "end_ms": 320 },
//     { "text": "world", "start_ms": 340,  "end_ms": 620 },
//     ...
//   ]
//
// Supports the OpenAI Whisper Python CLI (`whisper`) and whisper.cpp's
// CLI (`whisper-cpp` or `whisper-cli`). v0 prefers OpenAI Whisper
// because its --word_timestamps output is the highest fidelity.

import { resolve, basename, extname, dirname, join } from "node:path";
import { existsSync, mkdtempSync, readFileSync, writeFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { spawn } from "node:child_process";
import { flag } from "../args.mjs";

export async function transcribe(args) {
  const audio = args[0];
  const outArg = flag(args, "-o", "--out");
  const model = flag(args, "--model") ?? "base";
  if (!audio || !outArg) {
    console.error("wavelet transcribe: usage: wavelet transcribe <audio> -o <words.json> [--model base|small|medium|large]");
    return 1;
  }
  const absAudio = resolve(process.cwd(), audio);
  if (!existsSync(absAudio)) {
    console.error(`wavelet transcribe: file not found: ${absAudio}`);
    return 1;
  }
  const outPath = resolve(process.cwd(), outArg);

  // Probe for an available Whisper.
  const which = await firstAvailable(["whisper", "whisper-cpp", "whisper-cli"]);
  if (!which) {
    console.error(
      "wavelet transcribe: no Whisper binary found on PATH.\n" +
      "  Install one of:\n" +
      "    pip install openai-whisper        # provides `whisper`\n" +
      "    brew install whisper-cpp           # provides `whisper-cpp` / `whisper-cli`",
    );
    return 1;
  }

  console.log(`wavelet transcribe: using ${which} (model=${model})`);

  if (which === "whisper") {
    return await runOpenAIWhisper(absAudio, outPath, model);
  }
  // whisper.cpp variants — minimal support; words come out as cue
  // groups rather than per-word, which downstream still consumes.
  return await runWhisperCpp(which, absAudio, outPath, model);
}

async function runOpenAIWhisper(audio, outPath, model) {
  const tmp = mkdtempSync(join(tmpdir(), "wavelet-transcribe-"));
  try {
    const code = await new Promise((r) => {
      const proc = spawn("whisper", [
        audio,
        "--model", model,
        "--output_format", "json",
        "--word_timestamps", "True",
        "--output_dir", tmp,
      ], { stdio: ["ignore", "inherit", "inherit"] });
      proc.on("error", (err) => {
        console.error(`wavelet transcribe: ${err.message}`);
        r(1);
      });
      proc.on("exit", (c) => r(c ?? 1));
    });
    if (code !== 0) {
      console.error(`wavelet transcribe: whisper exited with code ${code}`);
      return code;
    }
    const base = basename(audio, extname(audio));
    const jsonPath = join(tmp, `${base}.json`);
    if (!existsSync(jsonPath)) {
      console.error(`wavelet transcribe: whisper output not found at ${jsonPath}`);
      return 1;
    }
    const data = JSON.parse(readFileSync(jsonPath, "utf8"));
    const words = openAIWhisperToCanonical(data);
    writeFileSync(outPath, JSON.stringify(words, null, 2), "utf8");
    console.log(`wavelet transcribe: wrote ${outPath} (${words.length} words)`);
    return 0;
  } finally {
    try { rmSync(tmp, { recursive: true, force: true }); } catch { /* skip */ }
  }
}

/** OpenAI Whisper JSON → canonical {text, start_ms, end_ms}[] */
function openAIWhisperToCanonical(data) {
  const out = [];
  for (const seg of data.segments ?? []) {
    for (const w of seg.words ?? []) {
      out.push({
        text: String(w.word ?? "").trim(),
        start_ms: Math.round((w.start ?? 0) * 1000),
        end_ms: Math.round((w.end ?? 0) * 1000),
      });
    }
  }
  return out;
}

async function runWhisperCpp(bin, audio, outPath, model) {
  // whisper.cpp expects a model FILE path, not a model name. Try common
  // locations if the user passed a name like "base".
  const modelPath = resolveWhisperCppModel(model);
  if (!modelPath) {
    console.error(
      `wavelet transcribe: whisper-cpp needs a model FILE path. Tried common locations for '${model}'.\n` +
      `  Download a model and pass --model <path>, e.g.:\n` +
      `    bash -c 'cd ~/.cache && curl -L -o ggml-base.bin https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-base.bin'\n` +
      `    wavelet transcribe ${basename(audio)} --model ~/.cache/ggml-base.bin -o words.json`,
    );
    return 1;
  }
  console.log(`                  model=${modelPath}`);
  // whisper.cpp emits JSON with --output-json into <basename>.json
  const code = await new Promise((r) => {
    const proc = spawn(bin, [
      "-f", audio,
      "-m", modelPath,
      "--output-json",
      "--output-file", outPath.replace(/\.json$/i, ""),
    ], { stdio: ["ignore", "inherit", "inherit"] });
    proc.on("error", (err) => {
      console.error(`wavelet transcribe: ${err.message}`);
      r(1);
    });
    proc.on("exit", (c) => r(c ?? 1));
  });
  if (code !== 0) {
    console.error(`wavelet transcribe: ${bin} exited with code ${code}`);
    return code;
  }
  // Convert whisper.cpp's segment-level JSON to our canonical shape.
  const path = outPath; // whisper.cpp writes <output-file>.json
  if (!existsSync(path)) {
    console.error(`wavelet transcribe: ${bin} output not found at ${path}`);
    return 1;
  }
  const raw = JSON.parse(readFileSync(path, "utf8"));
  const words = whisperCppToCanonical(raw);
  writeFileSync(outPath, JSON.stringify(words, null, 2), "utf8");
  console.log(`wavelet transcribe: wrote ${outPath} (${words.length} entries)`);
  return 0;
}

function whisperCppToCanonical(data) {
  // whisper.cpp JSON: { transcription: [ { timestamps: { from, to }, text }, ... ] }
  const out = [];
  for (const seg of data.transcription ?? []) {
    out.push({
      text: String(seg.text ?? "").trim(),
      start_ms: parseTimestamp(seg.timestamps?.from),
      end_ms: parseTimestamp(seg.timestamps?.to),
    });
  }
  return out;
}

function parseTimestamp(s) {
  // "HH:MM:SS,mmm" or "HH:MM:SS.mmm" → ms
  if (!s) return 0;
  const m = s.match(/^(\d{2}):(\d{2}):(\d{2})[.,](\d{3})$/);
  if (!m) return 0;
  return (Number(m[1]) * 3600 + Number(m[2]) * 60 + Number(m[3])) * 1000 + Number(m[4]);
}

function resolveWhisperCppModel(modelArg) {
  // Absolute or relative path that exists? Use it.
  const direct = resolve(process.cwd(), modelArg);
  if (existsSync(direct)) return direct;
  if (existsSync(modelArg)) return modelArg;
  // Look in standard caches for ggml-<name>.bin
  const candidates = [
    `${process.env.HOME}/.cache/whisper/ggml-${modelArg}.bin`,
    `${process.env.HOME}/.cache/ggml-${modelArg}.bin`,
    `/opt/homebrew/share/whisper-cpp/ggml-${modelArg}.bin`,
    `/usr/local/share/whisper-cpp/ggml-${modelArg}.bin`,
    `./models/ggml-${modelArg}.bin`,
  ];
  for (const c of candidates) {
    if (existsSync(c)) return c;
  }
  return null;
}

async function firstAvailable(bins) {
  for (const b of bins) {
    const ok = await new Promise((r) => {
      const p = spawn("which", [b], { stdio: ["ignore", "pipe", "ignore"] });
      p.on("error", () => r(false));
      p.on("exit", (code) => r(code === 0));
    });
    if (ok) return b;
  }
  return null;
}
