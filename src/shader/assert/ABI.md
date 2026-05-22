# Shader-as-validator ABI

The contract every assertion shader follows. Stable surface — the dispatcher
in `dispatch.rs` and every shader under `crates/wavelet/.../shader_lib/` agree
on this layout. Changes here are breaking.

## Entry point

```wgsl
@compute @workgroup_size(WG_X, WG_Y) fn assert_main(
    @builtin(global_invocation_id) gid: vec3<u32>,
) { ... }
```

`WG_X` and `WG_Y` are per-shader. Dispatcher reads the workgroup size from
the compiled module (via naga reflection) and computes dispatch count from
the frame dimensions in `Params`. Shaders that need a single-thread reducer
use `@workgroup_size(1, 1)` and rely on storage barriers within `assert_main`.

## Bindings (group 0)

| binding | resource                         | type (wgsl)                                                | usage      |
|---------|----------------------------------|------------------------------------------------------------|------------|
| 0       | color frame                      | `texture_2d<f32>`                                          | sample     |
| 1       | ID buffer (cryptomatte-style)    | `texture_2d<u32>`                                          | sample     |
| 2       | coverage buffer                  | `texture_2d<f32>`                                          | sample     |
| 3       | validator params                 | `var<uniform> params: Params`                              | read       |
| 4       | result                           | `var<storage, read_write> result: AssertionResult`         | write      |

Bindings 1 + 2 are mandatory in the layout — shaders that don't consume them
declare the variable anyway (or omit, since WGSL allows declaring only the
bindings you reference). The dispatcher always supplies all five slots, using
1x1 placeholder textures for the ID + coverage when the caller doesn't have
a real composite-aware buffer to hand in. wb-mxrk.6 lands the producer side.

## Params (binding 3)

`Params` is opaque to the dispatcher — each validator-kind defines its own
struct. The dispatcher accepts a `serde_json::Value` from the caller and
serializes it to a `Vec<u8>` zero-padded to 256 bytes, the maximum uniform
buffer binding stride for portable WebGPU + every shipping wgpu backend.
Shaders cast the buffer to their concrete `Params` struct.

Every `Params` struct must begin with:

```wgsl
struct Params {
    frame_width: u32,
    frame_height: u32,
    // ... validator-specific fields after
};
```

so the dispatcher can write the frame dimensions without knowing the rest.

## AssertionResult (binding 4)

```wgsl
struct AssertionResult {
    passed: u32,             // 0 = fail, 1 = pass
    reason_code: i32,        // mapped to a string in Rust via ReasonCode
    evidence_count: u32,     // number of valid floats in `evidence`
    evidence: array<f32, 64>,
};
```

Total size: `4 + 4 + 4 + 64 * 4 = 268 bytes`. Storage buffer, `read_write`.

The dispatcher zero-initializes the buffer before each dispatch. Shaders
must use atomic stores or single-thread writes to populate it — there is
intentionally no `atomic<u32>` in the result struct, because every shader
that needs reduction does its reduction into a workgroup-local buffer
first, then a single thread writes the final result. This keeps the
result-buffer contract simple and avoids forcing every consumer to think
about atomics.

`reason_code` semantics:
- `0`  — success (when `passed = 1`) or unspecified failure (when `passed = 0`)
- `1`  — assertion failed: metric out of bounds
- `2`  — assertion failed: region not found / empty mask
- `3`  — assertion failed: insufficient signal (e.g. flat region)
- `4`  — assertion failed: numerical issue (NaN / Inf)
- `5+` — validator-specific; documented in the shader's own header comment

Extend this set as new validators land; never recycle a code.

## Rust side

`dispatch_assertion(shader_path, frame, params) -> AssertionOutcome` is the
single public entry point. The dispatcher:

1. Reads + parses the WGSL source (naga validates it).
2. Allocates the result storage buffer (268 bytes, `STORAGE | COPY_SRC`).
3. Allocates the params uniform buffer (256 bytes, `UNIFORM | COPY_DST`).
4. Allocates the color texture from the `FrameSource` (loads PNGs for the
   path variant via the `png` crate).
5. Allocates 1x1 placeholder textures for the ID + coverage bindings.
6. Builds bind group layout + pipeline.
7. Dispatches with `(ceil(width / WG_X), ceil(height / WG_Y), 1)` work items.
8. Copies the result buffer to a `MAP_READ` staging buffer.
9. Maps + reads, returns `AssertionOutcome { passed, reason_code, reason, evidence }`.

The `evidence` `Vec<f32>` is truncated to the first `evidence_count` floats.

## Reserved keywords

WGSL reserves a long list of identifiers — including `pass`, `loop`,
`if`, `for`, `switch`, `case`, `default`, `return`, `var`, `let`, `fn`,
`struct`, `type`, `as`, and many more (see
[WGSL spec §3.2 Reserved Words](https://www.w3.org/TR/WGSL/#reserved-words)).
A reserved word as a local variable, struct field, or function name is a
compile error.

The one that bites validators most is `pass`: it collides with what you
naturally want to call the in-bounds boolean. Pick `in_band`, `is_within`,
`accept`, or similar instead. The `passed: u32` field on `AssertionResult`
is fine — `passed` is not reserved, only `pass` is.

## Out of scope (other tickets)

- The shader library — wb-mxrk.2..5
- Composite-aware ID + coverage producer — wb-mxrk.6
- `wgsl_to_wgpu` codegen integration — wb-mxrk.7
- `query.shader` Plan validator kind + CLI subcommand — wb-mxrk.8
