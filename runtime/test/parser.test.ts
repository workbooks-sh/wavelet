// End-to-end smoke test against a canonical wavelet HTML fixture.
// Verifies parse → resolve → lint produces a valid timeline with no
// errors, then exercises the headline failure modes.

import { describe, expect, test, beforeAll } from "bun:test";
import { parseDocument } from "../src/parser";
import { resolveTimeline, itemsAtFrame } from "../src/timeline";
import { lintDocument, summariseFindings } from "../src/lint";
import { parseTime } from "../src/time";
import { GamutError } from "../src/types";

// Bun's happy-dom integration provides DOMParser, but to be portable
// across Node-too runs we fall back to linkedom.
beforeAll(async () => {
  if (typeof DOMParser === "undefined") {
    const { DOMParser: LDP } = await import("linkedom");
    (globalThis as any).DOMParser = LDP;
  }
});

/** Wrap a `<gm-doc>...</gm-doc>` fragment in a proper HTML document so
 *  the parser doesn't misplace children. The HTML5 parser is finicky
 *  about unknown elements appearing before `<body>` — when the input
 *  is "<gm-doc><gm-timeline>...</gm-timeline></gm-doc>" with no
 *  surrounding shell, some parsers route the inner elements into
 *  `<head>`. Real authored wavelet files always live inside an HTML
 *  document, so this matches reality. */
function wrap(body: string): string {
  return `<!doctype html><html><body>${body}</body></html>`;
}

// IMPORTANT: HTML5 doesn't support self-closing custom elements.
// `<gm-asset … />` is parsed as `<gm-asset …>` with the slash ignored,
// and every following sibling becomes a child of <gm-asset>. The
// runtime contract is "every gm-* element gets an explicit closing
// tag." Authoring docs will say the same.
const CANONICAL_HTML = `<!doctype html>
<html>
  <head>
    <title>Demo</title>
  </head>
  <body>
    <gm-doc fps="30" resolution="1920x1080" aspect="16:9">

      <gm-asset id="hero-vid" kind="video" src="footage/hero.mp4"></gm-asset>
      <gm-asset id="vo"       kind="audio" src="audio/vo.mp3"></gm-asset>
      <gm-asset id="vo-words" kind="transcript" src="audio/vo.words.json"></gm-asset>

      <gm-composition id="body" src="comps/body.html"></gm-composition>

      <gm-timeline id="main" duration="12s">

        <gm-track id="base" z="0">
          <gm-clip asset="hero-vid" start="0s" in="2s" out="9s"></gm-clip>
        </gm-track>

        <gm-track id="overlays" z="10">
          <gm-scene id="title" start="0.5s" duration="3s">
            <h1 class="title">Cut from evidence.</h1>
            <script type="module">
              gsap.from(".title", { y: 60, opacity: 0 });
            </script>
          </gm-scene>
          <gm-scene id="payoff" start="3.5s" duration="2s" src="scenes/payoff.html"></gm-scene>
        </gm-track>

        <gm-track id="grading" z="20">
          <gm-adjustment start="0s" duration="12s" filter="contrast(1.05) saturate(1.08)"></gm-adjustment>
        </gm-track>

        <gm-track id="vo" z="0">
          <gm-audio asset="vo" start="0.5s" duration="11s" volume="1.0"></gm-audio>
        </gm-track>

      </gm-timeline>
    </gm-doc>
  </body>
</html>`;

describe("parseDocument", () => {
  test("parses the canonical fixture", () => {
    const doc = parseDocument(CANONICAL_HTML);
    expect(doc.fps).toBe(30);
    expect(doc.resolution).toEqual({ width: 1920, height: 1080 });
    expect(doc.aspect).toBe("16:9");
    expect(doc.assets.map((a) => a.id)).toEqual(["hero-vid", "vo", "vo-words"]);
    expect(doc.compositions.map((c) => c.id)).toEqual(["body"]);
    expect(doc.timeline.tracks.map((t) => t.id)).toEqual(["base", "overlays", "grading", "vo"]);
  });

  test("inline scene HTML is preserved", () => {
    const doc = parseDocument(CANONICAL_HTML);
    const overlays = doc.timeline.tracks.find((t) => t.id === "overlays")!;
    const titleScene = overlays.items[0];
    expect(titleScene.kind).toBe("scene");
    if (titleScene.kind === "scene") {
      expect(titleScene.src).toBeUndefined();
      expect(titleScene.inlineHtml).toContain("Cut from evidence");
      expect(titleScene.inlineHtml).toContain("gsap.from");
    }
  });

  test("scene content inside <template> is preserved", () => {
    const html = wrap(`<gm-doc fps="30" resolution="1920x1080" aspect="16:9">
      <gm-timeline id="t" duration="2s">
        <gm-track id="a" z="0">
          <gm-scene id="tmpl-scene" start="0s" duration="2s">
            <template>
              <h1 class="x">Inside template</h1>
              <script>console.log("only-runs-on-mount");</script>
            </template>
          </gm-scene>
        </gm-track>
      </gm-timeline>
    </gm-doc>`);
    const doc = parseDocument(html);
    const scene = doc.timeline.tracks[0].items[0];
    expect(scene.kind).toBe("scene");
    if (scene.kind === "scene") {
      expect(scene.inlineHtml).toContain("Inside template");
      expect(scene.inlineHtml).toContain("only-runs-on-mount");
    }
  });

  test("external scene preserves src", () => {
    const doc = parseDocument(CANONICAL_HTML);
    const overlays = doc.timeline.tracks.find((t) => t.id === "overlays")!;
    const payoffScene = overlays.items[1];
    expect(payoffScene.kind).toBe("scene");
    if (payoffScene.kind === "scene") {
      expect(payoffScene.src).toBe("scenes/payoff.html");
      expect(payoffScene.inlineHtml).toBeUndefined();
    }
  });

  test("missing <gm-doc> throws", () => {
    expect(() =>
      parseDocument("<html><body><p>hi</p></body></html>"),
    ).toThrow(GamutError);
  });

  test("missing required attribute throws", () => {
    expect(() =>
      parseDocument(wrap(`<gm-doc fps="30"><gm-timeline id="t" duration="1s"></gm-timeline></gm-doc>`)),
    ).toThrow(/requires attribute 'resolution'/);
  });

  test("bad resolution format throws", () => {
    expect(() =>
      parseDocument(wrap(`<gm-doc fps="30" resolution="big" aspect="16:9"><gm-timeline id="t" duration="1s"></gm-timeline></gm-doc>`)),
    ).toThrow(/must look like '1920x1080'/);
  });

  test("<gm-include> with both ref and src throws", () => {
    const html = wrap(`<gm-doc fps="30" resolution="1920x1080" aspect="16:9">
      <gm-composition id="x" src="x.html"></gm-composition>
      <gm-timeline id="t" duration="1s">
        <gm-track id="a" z="0">
          <gm-include ref="x" src="other.html" start="0s" duration="1s"></gm-include>
        </gm-track>
      </gm-timeline>
    </gm-doc>`);
    expect(() => parseDocument(html)).toThrow(/cannot have both ref= and src=/);
  });
});

describe("resolveTimeline", () => {
  test("computes frame ranges for every item", () => {
    const doc = parseDocument(CANONICAL_HTML);
    const resolved = resolveTimeline(doc);
    expect(resolved.durationFrames).toBe(12 * 30);

    const base = resolved.tracks.find((t) => t.id === "base")!;
    const clip = base.items[0];
    expect(clip.kind).toBe("clip");
    expect(clip.startFrame).toBe(0);
    // in="2s" out="9s" → 7-second clip = 210 frames at 30fps
    expect(clip.endFrame).toBe(7 * 30);

    const overlays = resolved.tracks.find((t) => t.id === "overlays")!;
    const title = overlays.items[0];
    expect(title.startFrame).toBe(15); // 0.5s = 15 frames
    expect(title.endFrame).toBe(15 + 90); // +3s = +90 frames

    const grading = resolved.tracks.find((t) => t.id === "grading")!;
    expect(grading.items[0]?.startFrame).toBe(0);
    expect(grading.items[0]?.endFrame).toBe(12 * 30);
  });

  test("itemsAtFrame returns active items only", () => {
    const doc = parseDocument(CANONICAL_HTML);
    const resolved = resolveTimeline(doc);

    // At frame 0: base clip + grading adjustment + vo audio (started at 0.5s)?
    const active0 = itemsAtFrame(resolved, 0);
    expect(active0.map((i) => i.kind).sort()).toEqual(["adjustment", "clip"]);

    // At frame 30 (1s): base clip + overlays title + grading + vo
    const active30 = itemsAtFrame(resolved, 30);
    const kinds = active30.map((i) => i.kind).sort();
    expect(kinds).toContain("clip");
    expect(kinds).toContain("scene");
    expect(kinds).toContain("adjustment");
    expect(kinds).toContain("audio");
  });

  test("clip without duration AND without in/out throws", () => {
    const html = wrap(`<gm-doc fps="30" resolution="1920x1080" aspect="16:9">
      <gm-asset id="x" kind="video" src="x.mp4"></gm-asset>
      <gm-timeline id="t" duration="5s">
        <gm-track id="a" z="0">
          <gm-clip asset="x" start="0s"></gm-clip>
        </gm-track>
      </gm-timeline>
    </gm-doc>`);
    const doc = parseDocument(html);
    expect(() => resolveTimeline(doc)).toThrow(/requires either duration= or BOTH in= and out=/);
  });
});

describe("lintDocument", () => {
  test("canonical fixture has no errors", async () => {
    const doc = parseDocument(CANONICAL_HTML);
    const findings = await lintDocument(doc);
    const { errors } = summariseFindings(findings);
    expect(errors).toBe(0);
  });

  test("flags dangling asset ref", async () => {
    const html = wrap(`<gm-doc fps="30" resolution="1920x1080" aspect="16:9">
      <gm-timeline id="t" duration="5s">
        <gm-track id="a" z="0">
          <gm-clip asset="nope" start="0s" duration="1s"></gm-clip>
        </gm-track>
      </gm-timeline>
    </gm-doc>`);
    const doc = parseDocument(html);
    const findings = await lintDocument(doc);
    expect(findings.some((f) => f.code === "dangling-asset-ref")).toBe(true);
  });

  test("flags duplicate track ids", async () => {
    const html = wrap(`<gm-doc fps="30" resolution="1920x1080" aspect="16:9">
      <gm-timeline id="t" duration="5s">
        <gm-track id="a" z="0"></gm-track>
        <gm-track id="a" z="1"></gm-track>
      </gm-timeline>
    </gm-doc>`);
    const doc = parseDocument(html);
    const findings = await lintDocument(doc);
    expect(findings.some((f) => f.code === "duplicate-track-id")).toBe(true);
  });

  test("flags schedule overflow", async () => {
    const html = wrap(`<gm-doc fps="30" resolution="1920x1080" aspect="16:9">
      <gm-asset id="x" kind="video" src="x.mp4"></gm-asset>
      <gm-timeline id="t" duration="3s">
        <gm-track id="a" z="0">
          <gm-clip asset="x" start="2s" duration="5s"></gm-clip>
        </gm-track>
      </gm-timeline>
    </gm-doc>`);
    const doc = parseDocument(html);
    const findings = await lintDocument(doc);
    expect(findings.some((f) => f.code === "schedule-overflow")).toBe(true);
  });

  test("flags dangling composition include", async () => {
    const html = wrap(`<gm-doc fps="30" resolution="1920x1080" aspect="16:9">
      <gm-timeline id="t" duration="3s">
        <gm-track id="a" z="0">
          <gm-include ref="missing" start="0s" duration="1s"></gm-include>
        </gm-track>
      </gm-timeline>
    </gm-doc>`);
    const doc = parseDocument(html);
    const findings = await lintDocument(doc);
    expect(findings.some((f) => f.code === "dangling-composition-ref")).toBe(true);
  });

  test("flags duplicate ids across the gm-doc subtree", async () => {
    const html = wrap(`<gm-doc fps="30" resolution="1920x1080" aspect="16:9">
      <gm-asset id="hero" kind="video" src="hero.mp4"></gm-asset>
      <gm-timeline id="t" duration="5s">
        <gm-track id="track-a" z="0">
          <gm-clip id="hero" asset="hero" start="0s" duration="2s"></gm-clip>
          <gm-clip id="hero" asset="hero" start="2s" duration="2s"></gm-clip>
        </gm-track>
        <gm-track id="track-a" z="1"></gm-track>
      </gm-timeline>
    </gm-doc>`);
    const doc = parseDocument(html);
    const findings = await lintDocument(doc);
    // 'hero' is duplicated across gm-asset and two gm-clip items (3 total).
    const dupHero = findings.find((f) => f.code === "duplicate-element-id" && f.message.includes('id="hero"'));
    expect(dupHero).toBeDefined();
    expect(dupHero?.message).toContain("3 times");
    // track-a is duplicated; duplicate-track-id error already fires, but the
    // duplicate-element-id warning should also flag it.
    const dupTrack = findings.find((f) => f.code === "duplicate-element-id" && f.message.includes('id="track-a"'));
    expect(dupTrack).toBeDefined();
  });

  test("file-existence check is wired", async () => {
    const html = wrap(`<gm-doc fps="30" resolution="1920x1080" aspect="16:9">
      <gm-asset id="x" kind="video" src="missing.mp4"></gm-asset>
      <gm-timeline id="t" duration="3s">
        <gm-track id="a" z="0">
          <gm-clip asset="x" start="0s" duration="1s"></gm-clip>
        </gm-track>
      </gm-timeline>
    </gm-doc>`);
    const doc = parseDocument(html);
    const findings = await lintDocument(doc, {
      fileExists: async () => false,
    });
    expect(findings.some((f) => f.code === "missing-asset-file")).toBe(true);
  });
});

describe("parseTime (sanity)", () => {
  test("frames", () => {
    expect(parseTime("12f", 30).frames).toBe(12);
  });
  test("seconds", () => {
    expect(parseTime("4s", 30).frames).toBe(120);
  });
  test("timecode", () => {
    expect(parseTime("00:00:04:12", 30).frames).toBe(132);
  });
  test("rejects sub-frame precision", () => {
    expect(() => parseTime("0.01s", 30)).toThrow();
  });
});
