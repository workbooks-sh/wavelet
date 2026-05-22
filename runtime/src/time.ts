// Frame / seconds / SMPTE timecode parser. Ported verbatim (with new
// error type) from packages/workbooks/packages/cw-xml/src/time.ts, which
// mirrored the Rust crate's time.rs. Behaviour is identical so the same
// time string produces the same frame number across implementations.
//
// Accepts:
//   - "12f"           → 12 absolute frames
//   - "4s"            → 4 seconds × fps
//   - "1.5s"          → 1.5 seconds, must resolve to whole frames
//   - "00:04:12:08"   → SMPTE timecode HH:MM:SS:FF

import { GamutError, type FrameTime, type Fps } from "./types";

export function frame(frames: number): FrameTime {
  return { frames };
}

export function framesToSeconds(t: FrameTime, fps: Fps): number {
  return t.frames / fps;
}

export function parseTime(raw: string, fps: Fps): FrameTime {
  const value = raw.trim();
  if (value.length === 0) {
    throw new GamutError(`invalid time '${raw}': time value is empty`);
  }
  if (fps <= 0 || !Number.isFinite(fps)) {
    throw new GamutError(`invalid time '${raw}': fps must be greater than zero`);
  }

  if (value.endsWith("f")) {
    const body = value.slice(0, -1);
    if (!/^\d+$/.test(body)) {
      throw new GamutError(
        `invalid time '${raw}': frame values must be unsigned integers like 12f`,
      );
    }
    return { frames: Number(body) };
  }

  if (value.endsWith("s")) {
    return parseSeconds(value, value.slice(0, -1), fps);
  }

  if ((value.match(/:/g) ?? []).length === 3) {
    return parseTimecode(value, fps);
  }

  throw new GamutError(
    `invalid time '${raw}': expected frames (12f), seconds (4s), or timecode (00:00:04:12)`,
  );
}

function parseSeconds(original: string, body: string, fps: Fps): FrameTime {
  const dot = body.indexOf(".");
  const whole = dot === -1 ? body : body.slice(0, dot);
  const frac = dot === -1 ? "" : body.slice(dot + 1);

  if (whole.length === 0 || !/^\d+$/.test(whole)) {
    throw new GamutError(
      `invalid time '${original}': seconds must be a non-negative decimal number`,
    );
  }
  if (frac.length > 0 && !/^\d+$/.test(frac)) {
    throw new GamutError(
      `invalid time '${original}': seconds must be a non-negative decimal number`,
    );
  }

  const fpsB = BigInt(fps);
  const wholeFrames = BigInt(whole) * fpsB;

  let fracFrames = 0n;
  if (frac.length > 0) {
    const numerator = BigInt(frac);
    const denominator = 10n ** BigInt(frac.length);
    const scaled = numerator * fpsB;
    if (scaled % denominator !== 0n) {
      throw new GamutError(
        `invalid time '${original}': seconds must resolve exactly to whole frames at this fps`,
      );
    }
    fracFrames = scaled / denominator;
  }

  const total = wholeFrames + fracFrames;
  if (total > 0xffff_ffffn) {
    throw new GamutError(`invalid time '${original}': time value overflows frame range`);
  }
  return { frames: Number(total) };
}

function parseTimecode(value: string, fps: Fps): FrameTime {
  const parts = value.split(":");
  if (parts.length !== 4) {
    throw new GamutError(`invalid time '${value}': timecode must have four fields`);
  }
  const [hh, mm, ss, ff] = parts;
  const hours = parseTcPart(value, hh, "hours");
  const minutes = parseTcPart(value, mm, "minutes");
  const seconds = parseTcPart(value, ss, "seconds");
  const frames = parseTcPart(value, ff, "frames");

  if (minutes >= 60 || seconds >= 60) {
    throw new GamutError(
      `invalid time '${value}': timecode minutes and seconds must be less than 60`,
    );
  }
  if (frames >= fps) {
    throw new GamutError(
      `invalid time '${value}': timecode frame component must be less than fps`,
    );
  }

  const totalSeconds = BigInt(hours) * 3600n + BigInt(minutes) * 60n + BigInt(seconds);
  const total = totalSeconds * BigInt(fps) + BigInt(frames);
  if (total > 0xffff_ffffn) {
    throw new GamutError(`invalid time '${value}': timecode overflows frame range`);
  }
  return { frames: Number(total) };
}

function parseTcPart(value: string, part: string, label: string): number {
  if (part.length !== 2 || !/^\d{2}$/.test(part)) {
    throw new GamutError(`invalid time '${value}': timecode ${label} must be two digits`);
  }
  return Number(part);
}
