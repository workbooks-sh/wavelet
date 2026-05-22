# wavelet-fx

A Hydra-shaped DSL for video post-effects. Compiles to WGSL fragment shaders + a render-graph spec. Renderer-agnostic — wavelet is the first consumer but wavelet-fx knows nothing about wgpu, Vello, or rsmpeg.

## What it is

A small, opinionated authoring language for the ~80% of shader work that's *video post*: per-pixel transforms, blends, modulation, multi-pass effects, feedback loops. The surface is intentionally near-identical to [Hydra](https://hydra.ojack.xyz) so existing sketches port with a global find/replace.

```
// grade.wavelet-fx
let n = noise(4, 0.1)
let s = src(iChannel0)
s.modulate(n, 0.02).contrast(1.1).output
```

That compiles to:
- one WGSL fragment shader string per pass
- a JSON render-graph spec describing pass order, intermediate textures, uniform bindings, and feedback buffers

What the consumer (wavelet) does with those outputs — build a `wgpu::RenderPipeline`, manage textures, run the graph — is entirely outside wavelet-fx's scope.

## What it isn't

- **Not a general WGSL replacement.** No compute shaders, no vertex shaders, no custom geometry. Fragment-pipeline only.
- **Not a renderer.** wavelet-fx emits text and JSON. It doesn't link wgpu, doesn't open windows, doesn't read pixels.
- **Not a 1:1 Hydra clone.** Same function names where they fit; wavelet-specific additions (audio, `prev`, timeline) where Hydra has no equivalent; a `@raw-wgsl { ... }` escape hatch for the parts a CSS-shaped DSL can't reach.

## Surface

Three concepts:

**Sources** produce a `vec4` color from `uv`:
`osc`, `noise`, `voronoi`, `gradient`, `solid`, `shape`, `src(channel)`, `prev` (last frame, auto-managed ping-pong).

**Transforms** chain on a source:
`.rotate`, `.scale`, `.kaleid`, `.pixelate`, `.repeat`, `.scroll`, `.color`, `.brightness`, `.contrast`, `.invert`, `.posterize`, `.thresh`, `.luma`, `.saturate`, `.hue`.

**Combinators** mix two sources:
`.add`, `.mult`, `.blend`, `.diff`, `.mask`, `.modulate`, `.modulateScale`, `.modulatePixelate`, `.modulateRotate`, `.modulateHue`.

Terminate with `.output` (one full-frame chain) or `.output(buffer_name)` (one of N named buffers, for multi-pass).

### Wavelet-specific extensions (not in Hydra)

Authored as builder functions (or the parser equivalents); each returns a
[`Value`](src/value.rs) that slots in anywhere a numeric literal goes.
Multiple references to the same source share one uniform slot — dedup is
done at lowering time.

- `audio_rms()` / `audio_fft(n)` — bound from wavelet's audio mixer
- `time_beat()` — bound from beat detection
- `seed()` — deterministic per-frame seed (`comp_hash ^ frame_index`)
- `prop("--energy")` — read a CSS custom property (Animato-driven uniforms)
- `prev()` — previous frame of the current pass's buffer (consumer
  ping-pongs)
- `@raw-wgsl { ... }` — drop into raw WGSL for the 10% the DSL can't
  express *(planned; not in v0)*

## Timeline / timecode model — same as Animato

wavelet-fx does not invent its own animation primitives. Every numeric parameter
is a [`Value`](src/value.rs) which is either a constant or an `animato::Tween<f32>`:

```rust
use wavelet-fx::{src, noise, Tween, Easing};

let pulse = Tween::new(0.0_f32, 0.1)
    .duration(2.0)
    .easing(Easing::EaseInOutSine)
    .build();

let comp = src(0)
    .modulate(noise(4.0, 0.0), pulse)   // tween drives modulation depth
    .contrast(1.1)
    .output();
```

`Tween`, `Easing`, and `Timeline` are re-exported from `wavelet-fx::` so callers
only import from one place — under the hood, they are *the exact same types*
wavelet uses for DOM/CSS animation. The integration contract is one line:

> A `Tween<f32>` in any parameter slot becomes a `UniformKind::Tween` entry in
> the emit output. At render time the consumer calls `tween.seek(frame_secs)`
> followed by `tween.value()` and writes the result to the corresponding
> uniform buffer slot — the same call pattern wavelet already uses everywhere
> else.

Constants are inlined as WGSL literals; tweens become uniform-table entries;
the master clock (`UniformKind::Time`) is the same `t` Animato samples at, so
shader-uniform animation and DOM animation stay frame-locked by construction.

Color and vector tweens (`Tween<[f32; 4]>` for animated `.color(r, g, b, a)`)
arrive in v1 as `ValueColor` / `ValueVec2`. Specializing `Value` to `f32` for
v0 keeps the generic trait bounds out of the AST.

## Compile pipeline

```
.wavelet-fx text
  → AST            (ast.rs)
  → IR             (ir.rs)        — DAG of passes, uniform table, buffer plan
  → WGSL + spec    (emit.rs)
```

The IR step is where the compiler:
- detects separable kernels (`.blur(8)` → two passes)
- allocates ping-pong buffers for any chain that references `prev` or feeds into itself
- resolves uniform bindings (numeric literals → inline constants, CSS props / audio / time → uniform table entries)
- inlines stdlib primitive bodies

## Integration contract

wavelet-fx's compile output:

```rust
pub struct EmitOutput {
    pub passes: Vec<EmittedPass>,
    pub uniforms: Vec<UniformBinding>,
    pub buffers: Vec<BufferSpec>,
}

pub struct EmittedPass {
    pub name: String,
    pub wgsl: String,                       // a complete fragment shader
    pub inputs: Vec<TextureRef>,            // named textures this pass reads
    pub output: TextureRef,                 // where this pass writes
    pub bindings: PassBindings,             // wgpu @group/@binding layout
}

pub struct PassBindings {
    pub uniforms: BindingSlot,              // the Uniforms struct's slot
    pub textures: Vec<TextureBinding>,      // each (TextureRef, tex slot, sampler slot)
}
```

That's the entire surface. Consumers iterate `bindings.textures`, resolve
each `source` to their own `wgpu::Texture`, build a `BindGroup`
mechanically, pack the `Uniforms` buffer using
[`UniformBinding::sample_at`](src/ir.rs) for scalar values
(tweens + constants) plus their own per-frame data for the rest
(`u_time`, `u_resolution`, `u_audio_rms`, `u_beat`, `u_seed`,
`u_prop_<name>`, ...). Nothing requires grepping the WGSL string.

## v0 scope — implemented

The crate is feature-complete for the initial wavelet integration:

1. Builder API + Hydra-shaped text parser, both produce the same AST.
2. Stdlib for the common authoring surface: `osc`, `noise`, `solid`,
   `src`, `from_buffer`, `prev`, `voronoi`, `gradient`, `shape`;
   transforms `rotate`, `scale`, `color`, `brightness`, `contrast`,
   `invert`, `scroll`, `pixelate`, `repeat`; combinators `add`, `mult`,
   `blend`, `modulate` (Hydra resampling), `modulate_scale`,
   `modulate_rotate`, `diff`, `mask`; terminators `output`, `output_to`.
3. Multi-pass with backwards-only buffer dependencies and ping-pong
   feedback (`prev()` marks the host buffer `feedback: true`).
4. uv threading through the IR — `rotate`, `scale`, `scroll`,
   `pixelate`, `repeat` rewrite uv; `modulate` evaluates rhs first and
   resamples lhs at the displaced uv.
5. Dynamic uniforms beyond tweens — `audio_rms`, `audio_fft(n)`,
   `time_beat`, `seed`, `prop("--name")` — deduped to single slots.
6. Structured `PassBindings` so consumers build wgpu BindGroups without
   reading the WGSL.

Deferred to v1:

- Color/vector tweens (`Tween<[f32;4]>` for animated `.color`).
- `@raw-wgsl { ... }` escape hatch.
- Separable-kernel auto-decomposition (`.blur(8)` → two passes).
- Non-Viewport buffer dimensions (half-res bloom, fixed sizes).

## Open questions

These shape v1 and are worth pinning before serious code lands:

1. **Function-name fidelity to Hydra.** Default: exact names for the ~30 that have a direct WGSL analog (free port for existing users). Diverge only where a Hydra concept has no clean WGSL meaning.
2. **File format.** `.wavelet-fx` files for serious work; inline strings (`filter: wavelet-fx('noise(4).rotate(0.1).output')`) for one-liners. Both supported.
3. **Relationship to raw `<gm-shader src='*.wgsl'>`** (wavelet's `wb-fxqv`): coexist. wavelet-fx compiles *to* WGSL; raw WGSL stays the escape hatch.
4. **Naming on crates.io.** `wavelet-fx` is taken (audio tool). If we publish, rename to `wavelet-wavelet-fx` or `wb-wavelet-fx`. Not blocking for in-tree work.

## Non-goals (firm)

- No live editor / REPL inside this crate. (wavelet owns the dev loop.)
- No GPU code. wavelet-fx never imports `wgpu`.
- No DOM / CSS parsing. CSS integration happens in wavelet via Blitz hooks; wavelet-fx just receives strings.
- No 3D, mesh, vertex effects. Fragment-only.

## Layout

```
packages/wavelet-fx/
  Cargo.toml
  SHADY.md                    this file
  README.md
  src/
    lib.rs                    public API + module roots
    ast.rs                    AST node types
    parse.rs                  text → AST (stub in v0)
    builder.rs                Rust fluent API → AST (the v0 entry point)
    ir.rs                     AST → IR (passes, uniforms, buffers)
    emit.rs                   IR → WGSL string + render-graph spec
    diagnostics.rs            error types
    stdlib/
      mod.rs                  primitive registry
      sources.rs              osc, noise, voronoi, gradient, solid, shape, src
      transforms.rs           rotate, scale, color, brightness, ...
      combinators.rs          add, mult, blend, modulate, ...
```
