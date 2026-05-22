//! Shared GPU dispatch pool (wb-mxrk.7).
//!
//! Holds a single `wgpu::Device + Queue` plus a pipeline cache keyed by
//! `shader_id`. Primitives + assertion shaders that thread the same
//! `GpuContext` can compose by texture handle across calls — previously
//! every dispatch site created its own device, and textures from one
//! device couldn't bind into another (the cross-device issue surfaced
//! by wb-mxrk.5 when sobel() composed with assertion shaders).
//!
//! Test code may pull a global `GpuContext::shared()`; production
//! callers thread their own instance.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use anyhow::{anyhow, Result};

/// Shared device/queue + compiled-pipeline cache.
pub struct GpuContext {
    device: Arc<wgpu::Device>,
    queue: Arc<wgpu::Queue>,
    pipeline_cache: Mutex<HashMap<&'static str, Arc<wgpu::ComputePipeline>>>,
}

impl GpuContext {
    /// Synchronous adapter + device request via pollster, default
    /// `Backends::PRIMARY` + high-perf adapter.
    pub fn new() -> Result<Self> {
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor::default());
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: None,
            force_fallback_adapter: false,
        }))
        .map_err(|e| anyhow!("no wgpu adapter available: {e}"))?;
        let (device, queue) =
            pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
                label: Some("gamut_assert_context"),
                required_features: wgpu::Features::empty(),
                required_limits: wgpu::Limits::default(),
                memory_hints: wgpu::MemoryHints::default(),
                trace: wgpu::Trace::Off,
                experimental_features: wgpu::ExperimentalFeatures::disabled(),
            }))
            .map_err(|e| anyhow!("device request failed: {e}"))?;
        Ok(Self {
            device: Arc::new(device),
            queue: Arc::new(queue),
            pipeline_cache: Mutex::new(HashMap::new()),
        })
    }

    /// Process-wide singleton. Lazy-initialized on first call; panics on
    /// init failure since callers reach this only after a successful
    /// adapter probe elsewhere.
    pub fn shared() -> Arc<Self> {
        static SHARED: OnceLock<Arc<GpuContext>> = OnceLock::new();
        SHARED
            .get_or_init(|| Arc::new(Self::new().expect("shared GpuContext init")))
            .clone()
    }

    /// Borrowed device handle. Identity is stable for the lifetime of
    /// this context — `TextureHandle` uses pointer equality on the
    /// `Arc<Device>` to assert same-context provenance.
    pub fn device(&self) -> &Arc<wgpu::Device> {
        &self.device
    }

    /// Borrowed queue handle.
    pub fn queue(&self) -> &Arc<wgpu::Queue> {
        &self.queue
    }

    /// Return the cached compute pipeline for `shader_id`, compiling
    /// `wgsl` the first time it's seen. The default `assert_main` entry
    /// point matches the ABI in `ABI.md`; primitive shaders pass their
    /// own entry-point name via `get_or_compile_with_entry`.
    pub fn get_or_compile(
        &self,
        shader_id: &'static str,
        wgsl: &str,
    ) -> Result<Arc<wgpu::ComputePipeline>> {
        self.get_or_compile_with(shader_id, wgsl, "assert_main", None)
    }

    /// Same as `get_or_compile` but with explicit entry point + optional
    /// pre-built bind-group-layout (so callers reusing the standard
    /// assertion layout don't recreate it per pipeline). When `layout`
    /// is `None`, wgpu derives the layout from the shader.
    pub fn get_or_compile_with(
        &self,
        shader_id: &'static str,
        wgsl: &str,
        entry_point: &str,
        layout: Option<&wgpu::PipelineLayout>,
    ) -> Result<Arc<wgpu::ComputePipeline>> {
        {
            let cache = self.pipeline_cache.lock().unwrap();
            if let Some(p) = cache.get(shader_id) {
                return Ok(p.clone());
            }
        }
        let module = self
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some(shader_id),
                source: wgpu::ShaderSource::Wgsl(wgsl.into()),
            });
        let pipeline = self
            .device
            .create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some(shader_id),
                layout,
                module: &module,
                entry_point: Some(entry_point),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                cache: None,
            });
        let pipeline = Arc::new(pipeline);
        let mut cache = self.pipeline_cache.lock().unwrap();
        let entry = cache
            .entry(shader_id)
            .or_insert_with(|| pipeline.clone())
            .clone();
        Ok(entry)
    }
}
