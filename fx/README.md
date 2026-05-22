# wavelet-fx

Hydra-shaped DSL that compiles to WGSL fragment shaders + a render-graph spec. Designed for video post-effects in the wavelet renderer; renderer-agnostic in principle.

See [SHADY.md](./SHADY.md) for the full design.

## Status

v0 scaffold. Module structure in place, compiler stubs only. Not yet emitting useful WGSL.

## Layout

- `src/builder.rs` — fluent Rust API (`osc(20.0).rotate(0.5).output()`), the v0 entry point
- `src/parse.rs` — text format parser (deferred until builder API is proven)
- `src/ast.rs` — AST types
- `src/ir.rs` — lowered IR (passes, uniforms, buffers)
- `src/emit.rs` — WGSL + render-graph emit
- `src/stdlib/` — primitive registry (sources, transforms, combinators)
