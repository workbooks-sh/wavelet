//! Shader pipeline — runs wavelet_fx-compiled WGSL fragments over rendered
//! scene textures. Phase 7b+ of epic wb-q4a6.
//!
//! WaveletFx (the sibling `packages/wavelet_fx/` crate) compiles its Hydra-shaped
//! DSL down to a complete WGSL fragment shader + a uniform/texture binding
//! schedule. This module owns the wgpu pipeline that consumes that output
//! and produces an RGBA buffer the video encoder can stream into the MP4.
//!
//! Two consumers of the same pipeline:
//! - **Transitions** (this file's `transition.rs`): blend scene-A and
//!   scene-B textures across a window declared in `comp.json`.
//! - **`<gm-shader>` wrapper** (Phase 7c, follow-on): render a single
//!   element subtree to texture, run a wavelet_fx pass, blit back.
//!
//! The wgpu device + queue are constructed once in
//! `render::wgpu_device_handle()` and shared with the GPU Vello renderer
//! from Phase 7a.

pub mod assert;
pub mod blur;
pub mod filter_pass;
pub mod gpu_blur;
pub mod transition;

pub use blur::gaussian_rgba;
pub use filter_pass::FilterPipeline;
pub use gpu_blur::GpuBlurPipeline;
pub use transition::{fx_source, TransitionPipeline, TransitionSource};

use std::sync::Arc;

/// Construct a wgpu device + queue for transition / `<gm-shader>` work.
/// Picks the high-performance adapter, falls back to lower-power, and
/// returns `None` only when no GPU is available.
pub fn create_wgpu() -> Option<(Arc<wgpu::Device>, Arc<wgpu::Queue>)> {
    let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor::default());
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        compatible_surface: None,
        force_fallback_adapter: false,
    }))
    .ok()?;
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("gamut_shader_device"),
        required_features: wgpu::Features::empty(),
        required_limits: wgpu::Limits::default(),
        memory_hints: wgpu::MemoryHints::default(),
        trace: wgpu::Trace::Off,
        experimental_features: wgpu::ExperimentalFeatures::disabled(),
    }))
    .ok()?;
    Some((Arc::new(device), Arc::new(queue)))
}
