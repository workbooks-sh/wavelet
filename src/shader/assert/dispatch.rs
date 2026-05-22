//! Compatibility wrapper around the new shader-assertion runtime
//! (`runtime.rs`). The five `wb-mxrk.5` assertion hosts still call
//! `dispatch_assertion(shader_path, frame, params: serde_json::Value)`;
//! this shim builds a `ShaderAssertion` against the process-shared
//! `GpuContext` and delegates.
//!
//! New code should prefer `run_assertion(&ctx, ShaderAssertion {..})`
//! directly so it can thread its own context and share textures across
//! primitive + assertion dispatches (wb-mxrk.7 NOTES).

use std::fs;
use std::path::Path;
use std::sync::OnceLock;

use anyhow::{anyhow, Context, Result};

use super::context::GpuContext;
use super::runtime::{run_assertion, ShaderAssertion, TextureHandle};
use super::types::{AssertionOutcome, FrameSource, PARAMS_MAX_BYTES};

/// Compile + dispatch a WGSL assertion shader over `frame`. Identical
/// surface to the wb-mxrk.1 entry point; internally now routes through
/// the shared `GpuContext` so textures stay on a single device.
pub fn dispatch_assertion(
    shader_path: &Path,
    frame: FrameSource,
    params: serde_json::Value,
) -> Result<AssertionOutcome> {
    let source = fs::read_to_string(shader_path)
        .with_context(|| format!("read shader {}", shader_path.display()))?;
    let ctx = GpuContext::shared();
    let shader_id: &'static str = intern_path(shader_path);
    let wgsl: &'static str = intern_string(source);

    let frame_handle = frame_into_handle(&ctx, frame)?;
    let assertion = ShaderAssertion {
        shader_id,
        wgsl,
        params: marshal_params(&params)?,
        frame: frame_handle,
        sidecar: None,
        reference: None,
    };
    run_assertion(&ctx, assertion)
}

fn frame_into_handle(ctx: &GpuContext, frame: FrameSource) -> Result<TextureHandle> {
    match frame {
        FrameSource::Texture(t) => Ok(TextureHandle::from_texture(ctx, t)),
        FrameSource::PngPath(path) => TextureHandle::from_png(ctx, &path),
        FrameSource::Rgba8 {
            width,
            height,
            pixels,
        } => Ok(TextureHandle::from_rgba8(ctx, width, height, &pixels)),
    }
}

fn marshal_params(extra: &serde_json::Value) -> Result<Vec<u8>> {
    let mut tail = Vec::new();
    if let Some(arr) = extra.as_array() {
        for v in arr {
            if tail.len() + 4 > PARAMS_MAX_BYTES - 8 {
                break;
            }
            // Integer-first packing: JSON integer literals must reach
            // WGSL as ints. `as_f64()` would otherwise turn `4` into the
            // f32 bit pattern of 4.0 and a `u32` field would read back
            // as 0x40800000.
            pack_scalar(&mut tail, v)?;
        }
    }
    Ok(tail)
}

fn pack_scalar(out: &mut Vec<u8>, v: &serde_json::Value) -> Result<()> {
    if v.is_u64() {
        out.extend_from_slice(&(v.as_u64().unwrap() as u32).to_le_bytes());
    } else if v.is_i64() {
        out.extend_from_slice(&(v.as_i64().unwrap() as i32).to_le_bytes());
    } else if v.is_f64() {
        out.extend_from_slice(&(v.as_f64().unwrap() as f32).to_le_bytes());
    } else {
        return Err(anyhow!(
            "assertion param slot must be a JSON number, got {v}"
        ));
    }
    Ok(())
}

/// `ShaderAssertion::shader_id` is `&'static str` so the pipeline-cache
/// key has a stable lifetime. The compat wrapper loads from disk, so
/// we intern the path string into a process-lifetime leak. The set is
/// small (one entry per distinct shader path actually used) and bounded
/// by the number of assertion shaders in the repo.
fn intern_path(path: &Path) -> &'static str {
    intern_string(path.to_string_lossy().into_owned())
}

fn intern_string(s: String) -> &'static str {
    static POOL: OnceLock<std::sync::Mutex<std::collections::HashMap<String, &'static str>>> =
        OnceLock::new();
    let pool = POOL.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()));
    let mut map = pool.lock().unwrap();
    if let Some(v) = map.get(&s) {
        return v;
    }
    let leaked: &'static str = Box::leak(s.clone().into_boxed_str());
    map.insert(s, leaked);
    leaked
}
