// Mount + unmount a <gm-shader>: compile inline GLSL ES 3.00 fragment
// shader source into a WebGL2 program, render it as a full-screen
// overlay quad during the shader's active window. Uniforms exposed:
//
//   uniform float uTime;        // 0..1 progress through the shader window
//   uniform float uFrame;       // current local frame (matches gm-tick)
//   uniform float uDuration;    // total frames in the shader window
//   uniform vec2  uResolution;  // canvas pixel size
//
// WGSL is intentionally NOT supported here — WebGPU coverage is still
// rolling out across browsers. WGSL authoring is the native Rust
// render path's job (wb-lsw0); the browser path is GLSL-only.

import type { ResolvedShader } from "./types";

export interface ShaderMount {
  el: HTMLElement;
  shader: ResolvedShader;
  /** Drive the shader's uTime / uFrame uniforms off the parent playhead. */
  tick(globalFrame: number, fps: number, playing: boolean): void;
  cleanup(): void;
}

export interface ShaderMountContext {
  viewport: HTMLElement;
  baseUrl: string | null;
  zIndex: number;
}

const VERTEX_SOURCE = `#version 300 es
precision highp float;
out vec2 vUV;
void main() {
  // Full-screen quad from gl_VertexID — no buffers needed.
  vec2 pos = vec2(
    float((gl_VertexID & 1) << 1) - 1.0,
    float((gl_VertexID & 2)) - 1.0
  );
  vUV = (pos + 1.0) * 0.5;
  gl_Position = vec4(pos, 0.0, 1.0);
}
`;

export async function mountShader(
  shader: ResolvedShader,
  ctx: ShaderMountContext,
): Promise<ShaderMount> {
  const el = document.createElement("div");
  el.className = "gm-shader-mount";
  el.style.position = "absolute";
  el.style.inset = "0";
  el.style.zIndex = String(ctx.zIndex);
  el.style.pointerEvents = "none";
  if (shader.class) el.classList.add(...shader.class.split(/\s+/).filter(Boolean));
  if (shader.style) el.setAttribute("style", el.getAttribute("style") + ";" + shader.style);
  ctx.viewport.appendChild(el);

  // WGSL is out-of-scope for the browser path.
  if (shader.lang.toLowerCase() === "wgsl") {
    el.textContent = `[gm-shader lang=\"wgsl\"]: WGSL is not supported in the browser path. Use the native Rust render path for WGSL, or rewrite as GLSL ES 3.00.`;
    el.style.color = "#ff8a8a";
    el.style.fontFamily = "ui-monospace, monospace";
    el.style.padding = "12px";
    return { el, shader, tick: () => {}, cleanup: () => el.remove() };
  }

  // Resolve fragment source: inline takes priority over src=.
  let fragSource = shader.inlineSource;
  if (!fragSource && shader.src) {
    try {
      const url = resolveUrl(shader.src, ctx.baseUrl);
      const res = await fetch(url);
      if (res.ok) fragSource = await res.text();
    } catch {
      // fall through to error below
    }
  }
  if (!fragSource || fragSource.trim().length === 0) {
    el.textContent = `[gm-shader: no source — provide inline GLSL or src=\"…\"]`;
    el.style.color = "#ff8a8a";
    return { el, shader, tick: () => {}, cleanup: () => el.remove() };
  }

  // Create a canvas sized to the viewport.
  const canvas = document.createElement("canvas");
  canvas.style.width = "100%";
  canvas.style.height = "100%";
  canvas.style.display = "block";
  el.appendChild(canvas);
  const gl = canvas.getContext("webgl2", { premultipliedAlpha: true, alpha: true });
  if (!gl) {
    el.textContent = `[gm-shader: WebGL2 not available in this browser]`;
    el.style.color = "#ff8a8a";
    return { el, shader, tick: () => {}, cleanup: () => el.remove() };
  }

  // Resize canvas to match its CSS-rendered size for crisp output.
  const resizeCanvas = () => {
    const dpr = window.devicePixelRatio || 1;
    const w = Math.max(1, Math.floor(canvas.clientWidth * dpr));
    const h = Math.max(1, Math.floor(canvas.clientHeight * dpr));
    if (canvas.width !== w || canvas.height !== h) {
      canvas.width = w;
      canvas.height = h;
      gl.viewport(0, 0, w, h);
    }
  };

  // Compile shaders.
  const vert = compileShader(gl, gl.VERTEX_SHADER, VERTEX_SOURCE);
  const frag = compileShader(gl, gl.FRAGMENT_SHADER, fragSource);
  if (!vert || !frag) {
    el.textContent = `[gm-shader: compile failed — check the console for shader log]`;
    el.style.color = "#ff8a8a";
    return { el, shader, tick: () => {}, cleanup: () => el.remove() };
  }
  const program = gl.createProgram();
  if (!program) {
    return { el, shader, tick: () => {}, cleanup: () => el.remove() };
  }
  gl.attachShader(program, vert);
  gl.attachShader(program, frag);
  gl.linkProgram(program);
  if (!gl.getProgramParameter(program, gl.LINK_STATUS)) {
    const log = gl.getProgramInfoLog(program) ?? "(no log)";
    console.error(`[gm-shader] link failed: ${log}`);
    el.textContent = `[gm-shader: link failed — see console]`;
    el.style.color = "#ff8a8a";
    return { el, shader, tick: () => {}, cleanup: () => el.remove() };
  }
  gl.useProgram(program);

  // Cache uniform locations once.
  const uTime = gl.getUniformLocation(program, "uTime");
  const uFrame = gl.getUniformLocation(program, "uFrame");
  const uDuration = gl.getUniformLocation(program, "uDuration");
  const uResolution = gl.getUniformLocation(program, "uResolution");

  // Pre-build a 4-vert triangle strip (matches the gl_VertexID quad
  // in the vertex shader — no actual buffer data needed; VAO is empty).
  const vao = gl.createVertexArray();
  gl.bindVertexArray(vao);

  // Enable alpha blending so the overlay composites with the
  // scenes/clips underneath.
  gl.enable(gl.BLEND);
  gl.blendFunc(gl.SRC_ALPHA, gl.ONE_MINUS_SRC_ALPHA);

  const durationFrames = shader.endFrame - shader.startFrame;

  return {
    el,
    shader,
    tick(globalFrame, _fps, _playing) {
      resizeCanvas();
      const localFrame = Math.max(0, globalFrame - shader.startFrame);
      const t = durationFrames > 0 ? localFrame / durationFrames : 0;
      if (uTime) gl.uniform1f(uTime, t);
      if (uFrame) gl.uniform1f(uFrame, localFrame);
      if (uDuration) gl.uniform1f(uDuration, durationFrames);
      if (uResolution) gl.uniform2f(uResolution, canvas.width, canvas.height);
      gl.clearColor(0, 0, 0, 0);
      gl.clear(gl.COLOR_BUFFER_BIT);
      gl.drawArrays(gl.TRIANGLE_STRIP, 0, 4);
    },
    cleanup() {
      gl.deleteProgram(program);
      gl.deleteShader(vert);
      gl.deleteShader(frag);
      gl.deleteVertexArray(vao);
      el.remove();
    },
  };
}

function compileShader(gl: WebGL2RenderingContext, type: number, source: string): WebGLShader | null {
  const shader = gl.createShader(type);
  if (!shader) return null;
  gl.shaderSource(shader, source);
  gl.compileShader(shader);
  if (!gl.getShaderParameter(shader, gl.COMPILE_STATUS)) {
    const log = gl.getShaderInfoLog(shader) ?? "(no log)";
    const kind = type === gl.VERTEX_SHADER ? "vertex" : "fragment";
    console.error(`[gm-shader] ${kind} shader compile failed: ${log}`);
    gl.deleteShader(shader);
    return null;
  }
  return shader;
}

function resolveUrl(src: string, base: string | null): string {
  if (!base) return src;
  try {
    return new URL(src, new URL(base, window.location.href)).toString();
  } catch {
    return src;
  }
}
