# wavelet-render-core — WASM render-core spike findings

**Question:** can the wavelet motion-graphics engine assemble video frames
*inside the nexus* (a WebAssembly / wasmtime sandbox) — composition parse +
CSS timeline + layout + paint → pixel frames — with **no native exec, no
GPU, no rsmpeg**?

**Verdict: PROVEN for frame rendering.** The carved render core compiles to
`wasm32-wasip1`, `wasm32-wasip2`, and `wasm32-unknown-unknown`, and a frame
of a CSS-animated composition was rendered **inside wasmtime** to a PNG and
visually verified. Encode (frames → mp4) stays host-side — that's the one
remaining wall, and it's a known bedrock (native ffmpeg), not a surprise.

Proof image: `proof/nexus-frame-wasip1.png` (rendered by the wasip1 module
under wasmtime, no GPU, no native code).

---

## Stage-by-stage

### Stage 1 — baseline (native)
The **parent `wavelet` crate does not build on this checkout**: its
`Cargo.toml` `[patch.crates-io]` redirects `blitz-paint` + the entire
`stylo` family to vendored forks at `../../vendor/{blitz-paint,stylo}`,
**and that `vendor/` directory is absent here**. So the monolith's native
baseline could not be run directly.

Instead, the baseline was established *through the carve* (Stage 3): the
render slice built natively and rendered a frame to PNG, visually correct
(dark bg, green gradient rounded box slid to its t=1.5s position). This
proves the engine + a composition render works on this box via the CPU
path. (rsmpeg/ffmpeg never enters the render-core path, so no ffmpeg was
needed.)

### Stage 2 — scope of the render core
The "parse → style/layout → paint frame → bytes" path needs only:

| Need | Crate(s) | Notes |
|---|---|---|
| HTML parse + DOM | `blitz-html`, `blitz-dom`, `html5ever`, `selectors` | default-features off |
| CSS engine | `stylo` family 0.17 (`stylo_traits/dom/atoms/...`), `servo_arc` | **crates.io, NOT the vendored fork** |
| Layout | `taffy` / `stylo_taffy` | |
| Text shaping | `parley`, `fontique`, `harfrust`, `skrifa` | system-font discovery OFF under wasm |
| Paint (CPU) | `anyrender`, `anyrender_vello_cpu`, `vello_cpu`, `vello_common`, `glifo` | pure CPU, no wgpu |
| Drawing model | `peniko`, `kurbo` | |
| Image decode | `image` (png feature) | for `<img>` |
| PNG out | `png` | |
| Timeline | **none — it's Stylo's CSS-animation clock** via `BaseDocument::resolve(t)` | |

**Cut entirely** (host-side or out of scope): `rsmpeg` (native ffmpeg),
`wgpu` + `anyrender_vello` + `naga` (GPU), `symphonia`/`rubato`/`rustfft`
(audio), `src/backends/` (network services), `ocr`/`c2pa`/`depth`/`agent`/
`director`/`screenplay`/`shader`(wgpu), `css_filter` post-process,
`video_bg` decode, `inline_video`, `clap`, `wavelet-fx`.

Key insight confirmed: the **timeline is free** — there is no separate
timeline crate to port. CSS `@keyframes` + `animation:` are driven entirely
by Stylo when you call `resolve(t_seconds)`. A frame at t≠0 differs from t=0
purely because Stylo recomputes animated styles. (Note: GSAP/JS-driven
timelines — e.g. `cli/templates/default/gamut.html` — are NOT animated by
this path; they need a JS engine. The offline/in-nexus path is CSS-only.)

### Stage 3 — carve (native)
Created `crates/wavelet-render-core` — a standalone crate (matches the repo's
one-crate-per-dir convention; the monolith is not a cargo workspace).
Public API:

```rust
pub fn render_frame_rgba(html: &str, frame: u32, fps: u32, w: u32, h: u32) -> Vec<u8> // RGBA8
pub fn render_frame(html: &str, frame: u32, fps: u32, w: u32, h: u32) -> Vec<u8>      // PNG bytes
pub fn rgba_to_png(rgba: &[u8], w: u32, h: u32) -> Vec<u8>
pub fn load_html(html: &str, w: u32, h: u32) -> HtmlDocument
```

Builds natively; `cargo test` passes (`renders_nonblank_and_animates`:
frame 0 ≠ frame 45, and not all-white). Native PNG visually verified.

### Stage 4 — wasm compile
- `cargo build --target wasm32-wasip1 --release` → **clean, first try.**
- `cargo build --target wasm32-wasip2 --release` → **clean.**
- `cargo build --lib --target wasm32-unknown-unknown --release` → **clean.**

**Stylo under wasm was the headline risk and it is NOT a wall** — the full
servo CSS engine (crates.io 0.17) compiled to all three wasm targets without
a single source patch.

### Stage 5 — run in the nexus (wasmtime)
A WASI command (`src/bin/render_one.rs`) renders a built-in CSS-animated
composition and writes the PNG via a preopened dir:

```
wasmtime run --dir=/tmp::/out target/wasm32-wasip1/release/render_one.wasm 36 /out/frame.png
# -> rendered frame 36 -> /out/frame.png (132645 bytes)
```

The PNG was **read back and visually inspected**: heading text ("Assembled
in the nexus") shaped with the bundled Geist font at full opacity (CSS fade
keyframe complete at t=1.2s, translateY applied), green subtitle, gradient
rounded box mid-slide, growing progress bar. Two independent CSS animations
at the correct timeline positions, text shaping, gradients, border-radius —
all rasterized **inside the wasm sandbox, no GPU, no native exec**. The
wasip2 build produces a byte-identical PNG (deterministic).

---

## Walls hit + how they were cleared

1. **Dependency version collision (the real fight).** A fresh resolve pulls
   `glifo 0.1.1` (→ `vello_common 0.0.9`) alongside `vello_cpu 0.0.8`
   (→ `vello_common 0.0.8`). The two `vello_common` versions collide inside
   `vello_cpu`'s glyph-atlas `text.rs` (`AtlasPaint`/`Pixmap` API drift) →
   7 compile errors. **Fix:** pin `glifo = "=0.1.0"`, `vello_cpu = "=0.0.8"`,
   `vello_common = "=0.0.8"` — replicating the parent monolith's lock. This
   is the one dep-graph hazard to remember.

2. **Fonts under WASI.** `system_fonts` cannot link under wasm, so text
   rendered as nothing at first (boxes/gradients/layout were fine without a
   font — that's a useful diagnostic). **Fix:** disable `system_fonts`
   (drop it from `blitz-dom` features) and feed a bundled font via
   `blitz_dom::build_single_font_ctx(bytes)` into `DocumentConfig.font_ctx`
   — the exact path blitz-dom documents as "the standard setup for WASM."
   Bundled Geist (OFL, 126 KB) as `assets/font.ttf`. A production nexus can
   swap in a host-fetched, content-addressed font instead of the embed.

3. **Threading.** No wall. `vello_cpu` multithreading (rayon) is an opt-in
   feature that `anyrender_vello_cpu 0.12.1` does **not** enable, so the CPU
   rasterizer is single-threaded by default — exactly what plain wasip1
   wants. No `-C target-feature=+atomics`, no threads shim needed.

4. **`wasm32-unknown-unknown` fs/args.** The lib compiles for it, but
   `render_one` (the proof bin) needs WASI for `std::fs::write` + args, so
   the *run* proof is on wasip1/wasip2. On bare unknown-unknown the host
   would call exported functions and pull the `Vec<u8>` out of linear memory
   (no fs needed) — the render itself is identical.

Deps cut/stubbed vs the monolith: rsmpeg, wgpu, anyrender_vello, naga,
pollster, symphonia, rubato, rustfft, bytemuck(shader), clap, wavelet-fx,
euclid, chrono, ulid, serde_yaml, and the whole `src/backends|ocr|c2pa|
depth|agent|director|screenplay|shader|css_filter|inline_video|video`
surface. None are needed to paint a frame.

---

## "Assemble video in an isolated workbook" — honest end-to-end picture

| Stage | Maturity | Evidence |
|---|---|---|
| Composition parse (HTML/CSS) | **PROVEN in-nexus** | renders correctly in wasmtime |
| CSS timeline (`resolve(t)`) | **PROVEN in-nexus** | two animations at correct t; frames differ by t |
| Layout + text shaping + paint → frame | **PROVEN in-nexus** | PNG read back, visually correct |
| Fonts | **PROVEN (bundled)** | Geist embedded; host-fetch swap is trivial |
| JS-driven timelines (GSAP) | **OUT of this path** | needs a JS engine; CSS-animation path only |
| `<img>` / asset fetch | **partial** | decode works in-wasm; fetching bytes is a host cap (no net provider in-nexus by design — pass data: URIs or host-injected bytes) |
| video_bg / inline `<video>` | **host-side** | decode is rsmpeg (native) |
| CSS `filter:` post-process | **deferred** | cut from carve; CPU impl exists in monolith, portable later |
| **Encode: frames → mp4** | **WALLED (by design)** | rsmpeg = native ffmpeg = bedrock. Needs a **host broker** (frames out of the sandbox → host ffmpeg) **or** a Forge ffmpeg→wasi lane (not built). Note: not chosen here. |
| Audio mux | **host-side** | symphonia mix + container mux, host |
| Playback | **tier-1, separate** | `@work.books/wavelet-runtime` web-components |

**Bottom line:** the hard, doubted part — a full HTML/CSS/Stylo/Parley/Vello
render core painting motion-graphics frames inside a WASM sandbox with no
GPU and no native code — **works, proven empirically in wasmtime.** The
tenant model is: nexus emits frames (RGBA/PNG), the **host** encodes them to
mp4 (ffmpeg broker) and muxes audio. The only thing that can't move into the
sandbox today is encode, and that was always expected (native ffmpeg).

## Remaining work to make this production-in-nexus
1. Frame-loop API: stream N frames out of one loaded document (reuse the
   `Renderer`/document across `resolve(t)` calls; the monolith already does
   this in `render_offline::render` — port the loop, sans encoder).
2. Host frame→mp4 broker (or Forge ffmpeg→wasi) — pick one; this is the
   encode story, not a render-core problem.
3. Asset injection seam: host hands image/font bytes into the sandbox
   (data: URIs already work; a host-brokered fetch is the richer option).
4. Optional: port the CPU `css_filter` pass for filter/blur effects.
5. Decide font policy in-nexus (embed a default + host-fetch override per
   the wavelet-fonts "fetch not bundle" canon).
