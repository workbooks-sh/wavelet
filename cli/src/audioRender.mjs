// Pre-render the resolved <gm-audio> cues to a single WAV file the
// main render command muxes alongside the video stream. Mirrors the
// browser audioMixer's envelope math (vol × fade-in × fade-out ×
// duck) so the rendered output matches what viewers hear in
// preview.
//
// Implementation strategy:
// 1. Decode each asset once via ffmpeg subprocess to interleaved f32
//    stereo PCM at the project sample rate.
// 2. Walk every output sample, sum each active cue's gain-scaled
//    samples into the master buffer.
// 3. Write WAV header + samples to a temp file.
//
// Duck model: for each sample's frame index, if any OTHER cue has
// duck > 0 active at that frame, attenuate this cue by max-duck dB.
// Equal-power pan would be more correct than linear; we use linear
// here for simplicity. The math matches audioMixer.ts.

import { spawn } from "node:child_process";
import { writeFile, mkdtemp, rm } from "node:fs/promises";
import { join, resolve, dirname } from "node:path";
import { tmpdir } from "node:os";

export const RENDER_SAMPLE_RATE = 48000;
const CHANNELS = 2;

/**
 * Render all audio cues to a WAV file. Returns the path to the WAV,
 * or null if there are no cues to render.
 *
 * @param resolvedDoc - parsed + resolved wavelet doc
 *   (must have .audios cues per track and .assets list)
 * @param compDir - directory of the source .html (for relative asset paths)
 * @param totalFrames - video duration in frames
 * @param fps
 * @param progress(pct) - optional progress callback
 */
export async function renderAudio({ resolvedDoc, compDir, totalFrames, fps, progress }) {
  // Collect every active audio cue across all tracks.
  const cues = [];
  for (const track of resolvedDoc.tracks ?? []) {
    for (const item of track.items ?? []) {
      if (item.kind === "audio") cues.push(item);
    }
  }
  if (cues.length === 0) return null;

  // Decode each referenced asset once.
  const audioAssets = (resolvedDoc.assets ?? []).filter((a) => a.kind === "audio");
  if (audioAssets.length === 0) return null;

  const tmp = await mkdtemp(join(tmpdir(), "wavelet-audio-"));
  const buffers = new Map(); // assetId -> Float32Array (interleaved L/R)
  try {
    for (let i = 0; i < audioAssets.length; i++) {
      const a = audioAssets[i];
      const absPath = resolve(compDir, a.src);
      try {
        const pcm = await decodeToPcm(absPath, RENDER_SAMPLE_RATE);
        buffers.set(a.id, pcm);
      } catch (e) {
        console.warn(`wavelet render: failed to decode audio asset '${a.id}' (${absPath}): ${e.message ?? e}`);
      }
      progress?.(Math.floor((i + 1) / audioAssets.length * 20));
    }
    if (buffers.size === 0) return null;

    // Pre-compute duck windows for cross-cue attenuation.
    const duckWindows = cues
      .filter((c) => (c.duck ?? 0) > 0)
      .map((c) => ({ start: c.startFrame, end: c.endFrame, db: c.duck }));

    const totalSamples = Math.ceil((totalFrames / fps) * RENDER_SAMPLE_RATE);
    const out = new Float32Array(totalSamples * CHANNELS);

    for (let cueIdx = 0; cueIdx < cues.length; cueIdx++) {
      const cue = cues[cueIdx];
      const src = buffers.get(cue.asset);
      if (!src) continue;
      mixCueInto(out, src, cue, duckWindows, fps);
      progress?.(20 + Math.floor((cueIdx + 1) / cues.length * 60));
    }

    // Write the master buffer as a WAV file the main ffmpeg call
    // can read as an input.
    const wavPath = join(tmp, "mix.wav");
    await writeWavFile(wavPath, out, RENDER_SAMPLE_RATE, CHANNELS);
    progress?.(100);
    return { wavPath, cleanupTmp: () => rm(tmp, { recursive: true, force: true }).catch(() => {}) };
  } catch (err) {
    await rm(tmp, { recursive: true, force: true }).catch(() => {});
    throw err;
  }
}

function mixCueInto(out, src, cue, duckWindows, fps) {
  const cueStartSample = Math.round((cue.startFrame / fps) * RENDER_SAMPLE_RATE);
  const cueEndSample = Math.round((cue.endFrame / fps) * RENDER_SAMPLE_RATE);
  const cueLenSamples = cueEndSample - cueStartSample;
  const fadeInSamples = Math.round((cue.fadeIn ?? 0) * RENDER_SAMPLE_RATE);
  const fadeOutSamples = Math.round((cue.fadeOut ?? 0) * RENDER_SAMPLE_RATE);
  const baseVol = cue.volume ?? 1;
  const pan = Math.max(-1, Math.min(1, cue.pan ?? 0));
  const cueDuckDb = cue.duck ?? 0;
  const srcFrames = src.length / CHANNELS;

  // Linear pan (matches Web Audio StereoPannerNode at small enough
  // magnitudes; equal-power pan is slightly different but the
  // perceptual gap is minor for content-level mixing).
  const lGain = pan <= 0 ? 1 : 1 - pan;
  const rGain = pan >= 0 ? 1 : 1 + pan;

  const outEnd = Math.min(cueEndSample, out.length / CHANNELS);

  for (let i = cueStartSample; i < outEnd; i++) {
    const cuePos = i - cueStartSample;
    let env = 1;
    if (fadeInSamples > 0 && cuePos < fadeInSamples) env *= cuePos / fadeInSamples;
    if (fadeOutSamples > 0 && cuePos > cueLenSamples - fadeOutSamples) {
      env *= Math.max(0, (cueLenSamples - cuePos) / fadeOutSamples);
    }

    // Ducking: if some OTHER cue has duck > 0 active at this frame,
    // attenuate this cue. If THIS cue is itself the duck source, it
    // doesn't duck itself.
    const frame = (i / RENDER_SAMPLE_RATE) * fps;
    let activeDuckDb = 0;
    for (const dw of duckWindows) {
      if (frame >= dw.start && frame < dw.end && dw.db > activeDuckDb) {
        activeDuckDb = dw.db;
      }
    }
    const isDuckSource = cueDuckDb >= activeDuckDb && activeDuckDb > 0;
    const duckMul = isDuckSource || activeDuckDb === 0 ? 1 : Math.pow(10, -activeDuckDb / 20);

    const gain = baseVol * env * duckMul;
    if (gain <= 0) continue;

    // Sample from the cue's PCM buffer; loop if cue runs longer
    // than the asset (matches gm-audio loop="true" semantics — for
    // non-loop cues, the cuePos won't exceed srcFrames anyway since
    // the cue's duration is bounded by its declared duration).
    const srcPos = cuePos % srcFrames;
    const sIdx = srcPos * CHANNELS;
    out[i * CHANNELS]     += src[sIdx]     * gain * lGain;
    out[i * CHANNELS + 1] += src[sIdx + 1] * gain * rGain;
  }
}

/**
 * Decode an audio file to interleaved f32le stereo PCM at the given
 * sample rate via ffmpeg subprocess. Returns a Float32Array of
 * length (frames * 2).
 */
async function decodeToPcm(path, sampleRate) {
  return new Promise((resolveFn, reject) => {
    const proc = spawn("ffmpeg", [
      "-v", "error",
      "-i", path,
      "-f", "f32le",
      "-ac", "2",
      "-ar", String(sampleRate),
      "-",
    ], { stdio: ["ignore", "pipe", "inherit"] });

    const chunks = [];
    proc.stdout.on("data", (c) => chunks.push(c));
    proc.on("error", reject);
    proc.on("exit", (code) => {
      if (code !== 0) {
        reject(new Error(`ffmpeg decode failed for ${path} (exit ${code})`));
        return;
      }
      const buf = Buffer.concat(chunks);
      // Float32Array view over the buffer (alignment-safe via slice).
      const aligned = new Float32Array(buf.length / 4);
      // Buffer might not be 4-byte aligned in the underlying ArrayBuffer;
      // copy via DataView to be safe.
      const view = new DataView(buf.buffer, buf.byteOffset, buf.byteLength);
      for (let i = 0; i < aligned.length; i++) {
        aligned[i] = view.getFloat32(i * 4, true);
      }
      resolveFn(aligned);
    });
  });
}

/** Write a 16-bit PCM WAV file from a Float32Array of interleaved samples. */
async function writeWavFile(path, samples, sampleRate, channels) {
  // Convert f32 to int16. Clip soft (-1, 1) range to int16 range.
  const int16 = new Int16Array(samples.length);
  for (let i = 0; i < samples.length; i++) {
    const s = Math.max(-1, Math.min(1, samples[i]));
    int16[i] = Math.round(s * 32767);
  }

  const dataBytes = int16.length * 2;
  const buf = Buffer.alloc(44 + dataBytes);

  // RIFF header
  buf.write("RIFF", 0);
  buf.writeUInt32LE(36 + dataBytes, 4);
  buf.write("WAVE", 8);
  // fmt chunk
  buf.write("fmt ", 12);
  buf.writeUInt32LE(16, 16);            // chunk size
  buf.writeUInt16LE(1, 20);             // PCM format
  buf.writeUInt16LE(channels, 22);
  buf.writeUInt32LE(sampleRate, 24);
  buf.writeUInt32LE(sampleRate * channels * 2, 28); // byte rate
  buf.writeUInt16LE(channels * 2, 32);  // block align
  buf.writeUInt16LE(16, 34);            // bits per sample
  // data chunk
  buf.write("data", 36);
  buf.writeUInt32LE(dataBytes, 40);
  // sample data
  Buffer.from(int16.buffer, int16.byteOffset, int16.byteLength).copy(buf, 44);

  await writeFile(path, buf);
}
