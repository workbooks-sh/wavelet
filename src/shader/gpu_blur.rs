//! GPU separable Gaussian blur — honors wavelet_fx's `PreEffect::GpuBlur` by
//! running a 2-pass blur on the input texture before it's bound to the
//! main wavelet_fx fragment shader.
//!
//! Architecture:
//! - One `GpuBlurPipeline` per input texture that needs blur. Allocates
//!   two scratch textures (horizontal-intermediate + final-blurred) at
//!   the same size as the input.
//! - Two render pipelines (horizontal + vertical), each running a
//!   fullscreen triangle whose fragment shader is the lifted Bevy-bloom
//!   separable Gaussian from `wavelet_fx::stdlib::blur`.
//! - One per-pass uniform buffer carrying `sigma` and `resolution`.
//!
//! Per dispatch the encoder runs:
//!   input_tex → [blur_h pipeline] → intermediate_tex
//!   intermediate_tex → [blur_v pipeline] → blurred_tex
//!
//! `blurred_tex` is what the caller binds to its main shader (in place
//! of the raw input). For the transition pipeline that means:
//! `tex_a_blur` replaces `tex_a` in the bind group whenever the wavelet_fx
//! emit's `pre_effects` contains a `GpuBlur` for channel 0.

use std::sync::Arc;
use wgpu::util::DeviceExt;

/// Fullscreen-triangle vertex shader shared with the rest of the
/// shader subsystem. Two of three vertices fall outside [-1,1] so we
/// don't waste fragments outside the viewport.
const VERTEX_WGSL: &str = r#"
struct VsOut {
  @builtin(position) pos: vec4<f32>,
  @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) vid: u32) -> VsOut {
  var positions = array<vec2<f32>, 3>(
    vec2<f32>(-1.0, -3.0),
    vec2<f32>(-1.0,  1.0),
    vec2<f32>( 3.0,  1.0),
  );
  var uvs = array<vec2<f32>, 3>(
    vec2<f32>(0.0, 2.0),
    vec2<f32>(0.0, 0.0),
    vec2<f32>(2.0, 0.0),
  );
  let p = positions[vid];
  let u = uvs[vid];
  var out: VsOut;
  out.pos = vec4<f32>(p, 0.0, 1.0);
  out.uv = u;
  return out;
}
"#;

/// Build the fragment shader for one direction by composing wavelet_fx's
/// stdlib helper with a tiny entrypoint that wires it up. We emit the
/// horizontal version when `axis == "h"`, vertical otherwise.
fn build_fragment_wgsl(axis: &str) -> String {
    let helper = if axis == "h" {
        wavelet_fx::stdlib::blur::SEPARABLE_GAUSSIAN_H
    } else {
        wavelet_fx::stdlib::blur::SEPARABLE_GAUSSIAN_V
    };
    let entry = if axis == "h" { "fx_blur_h" } else { "fx_blur_v" };
    format!(
        r#"
struct Uniforms {{
  sigma: f32,
  _pad: f32,
  resolution: vec2<f32>,
}};

@group(0) @binding(0) var<uniform> u: Uniforms;
@group(0) @binding(1) var src: texture_2d<f32>;
@group(0) @binding(2) var smp: sampler;

{helper}

@fragment
fn fs_main(@location(0) uv: vec2<f32>) -> @location(0) vec4<f32> {{
  return {entry}(src, smp, uv, u.sigma, u.resolution);
}}
"#
    )
}

/// CPU mirror of the WGSL Uniforms struct above. std140-ish: scalar f32
/// at offset 0, padding f32 at offset 4, vec2 at offset 8.
#[repr(C)]
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
struct BlurUniforms {
    sigma: f32,
    _pad: f32,
    resolution: [f32; 2],
}

/// GPU-side 2-pass separable Gaussian blur pipeline. Created once per
/// input texture that wavelet_fx's emit marked with `PreEffect::GpuBlur`;
/// records both blur passes into a caller-supplied command encoder via
/// [`encode`](Self::encode). Exposes [`blurred_view`](Self::blurred_view)
/// — the texture the caller's main shader should sample in place of
/// the raw input.
pub struct GpuBlurPipeline {
    width: u32,
    height: u32,
    sigma: f32,
    queue: Arc<wgpu::Queue>,
    /// Horizontal-pass intermediate texture. Read from in vertical pass.
    /// Kept alive so the `intermediate_view` it backs stays valid; never
    /// referenced directly after construction.
    #[allow(dead_code)]
    intermediate: wgpu::Texture,
    intermediate_view: wgpu::TextureView,
    /// Final blurred texture. Caller binds this in place of the raw input.
    /// Kept alive to back `blurred_view`; never referenced directly.
    #[allow(dead_code)]
    blurred: wgpu::Texture,
    /// View into the final blurred texture. Bind this in the consumer's
    /// fragment-shader bind group instead of the original input view.
    pub blurred_view: wgpu::TextureView,
    pipeline_h: wgpu::RenderPipeline,
    pipeline_v: wgpu::RenderPipeline,
    bind_group_h: wgpu::BindGroup,
    bind_group_v: wgpu::BindGroup,
    uniform_buf: wgpu::Buffer,
}

impl GpuBlurPipeline {
    /// Build a blur pipeline targeting a specific input texture. The
    /// pipeline reads from `input_view`, writes the final blurred result
    /// into a texture exposed as `blurred_view`. Callers bind that
    /// blurred view to their main shader's bind group.
    ///
    /// `format` is the input texture's format — typically
    /// `Rgba8Unorm` matching the transition pipeline's tex_a/tex_b.
    pub fn new(
        device: &wgpu::Device,
        queue: Arc<wgpu::Queue>,
        input_view: &wgpu::TextureView,
        width: u32,
        height: u32,
        format: wgpu::TextureFormat,
        sigma: f32,
    ) -> Self {
        let intermediate = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("gpu_blur_intermediate"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let intermediate_view =
            intermediate.create_view(&wgpu::TextureViewDescriptor::default());

        let blurred = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("gpu_blur_output"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let blurred_view = blurred.create_view(&wgpu::TextureViewDescriptor::default());

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("gpu_blur_sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        let uniforms = BlurUniforms {
            sigma,
            _pad: 0.0,
            resolution: [width as f32, height as f32],
        };
        let uniform_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("gpu_blur_uniforms"),
            contents: bytemuck::bytes_of(&uniforms),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        let bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("gpu_blur_bgl"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float {
                                filterable: true,
                            },
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 2,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                        count: None,
                    },
                ],
            });

        let bind_group_h = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("gpu_blur_bg_h"),
            layout: &bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniform_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(input_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
            ],
        });
        let bind_group_v = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("gpu_blur_bg_v"),
            layout: &bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniform_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&intermediate_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
            ],
        });

        let vs = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("gpu_blur_vs"),
            source: wgpu::ShaderSource::Wgsl(VERTEX_WGSL.into()),
        });
        let fs_h_wgsl = build_fragment_wgsl("h");
        let fs_v_wgsl = build_fragment_wgsl("v");
        let fs_h = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("gpu_blur_fs_h"),
            source: wgpu::ShaderSource::Wgsl(fs_h_wgsl.into()),
        });
        let fs_v = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("gpu_blur_fs_v"),
            source: wgpu::ShaderSource::Wgsl(fs_v_wgsl.into()),
        });

        let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("gpu_blur_pl"),
            bind_group_layouts: &[&bind_group_layout],
            immediate_size: 0,
        });

        let make_pipeline = |module: &wgpu::ShaderModule, label: &str| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&pl),
                vertex: wgpu::VertexState {
                    module: &vs,
                    entry_point: Some("vs_main"),
                    buffers: &[],
                    compilation_options: Default::default(),
                },
                fragment: Some(wgpu::FragmentState {
                    module,
                    entry_point: Some("fs_main"),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                    compilation_options: Default::default(),
                }),
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                multiview_mask: None,
                cache: None,
            })
        };

        let pipeline_h = make_pipeline(&fs_h, "gpu_blur_pipeline_h");
        let pipeline_v = make_pipeline(&fs_v, "gpu_blur_pipeline_v");

        Self {
            width,
            height,
            sigma,
            queue,
            intermediate,
            intermediate_view,
            blurred,
            blurred_view,
            pipeline_h,
            pipeline_v,
            bind_group_h,
            bind_group_v,
            uniform_buf,
        }
    }

    /// Update σ if the caller wants to vary blur per frame. Rewrites the
    /// uniform buffer; no pipeline rebuild required.
    pub fn set_sigma(&mut self, sigma: f32) {
        if (self.sigma - sigma).abs() < f32::EPSILON {
            return;
        }
        self.sigma = sigma;
        let uniforms = BlurUniforms {
            sigma,
            _pad: 0.0,
            resolution: [self.width as f32, self.height as f32],
        };
        self.queue
            .write_buffer(&self.uniform_buf, 0, bytemuck::bytes_of(&uniforms));
    }

    /// Record both blur passes into `encoder`. After `queue.submit`,
    /// `self.blurred_view` is what the caller's main shader should read.
    pub fn encode(&self, encoder: &mut wgpu::CommandEncoder) {
        {
            let mut rp = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("gpu_blur_pass_h"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.intermediate_view,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            rp.set_pipeline(&self.pipeline_h);
            rp.set_bind_group(0, &self.bind_group_h, &[]);
            rp.draw(0..3, 0..1);
        }
        {
            let mut rp = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("gpu_blur_pass_v"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.blurred_view,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            rp.set_pipeline(&self.pipeline_v);
            rp.set_bind_group(0, &self.bind_group_v, &[]);
            rp.draw(0..3, 0..1);
        }
    }
}
