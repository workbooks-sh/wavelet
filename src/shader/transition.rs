//! Scene-to-scene shader transitions powered by wavelet_fx. Phase 7b of
//! epic wb-q4a6.
//!
//! `TransitionPipeline` owns:
//! - wgpu device / queue (shared with the GPU Vello renderer)
//! - a `wgpu::RenderPipeline` whose fragment stage is the wavelet_fx-compiled
//!   WGSL, plus a full-screen-triangle vertex stage we own
//! - two input `wgpu::Texture`s (the outgoing + incoming scene frames)
//! - one output texture + a CPU-readable buffer for the blended result
//!
//! Per frame within a transition window:
//!   `render(frame_a, frame_b, progress) -> Vec<u8>`
//!
//! ## What wavelet_fx emits
//!
//! WaveletFx's `EmitOutput.passes[0].wgsl` is a complete fragment shader with
//! its own `Uniforms` struct and texture bindings. Texture order matches
//! `passes[0].inputs` — for a transition that's `[InputChannel(0),
//! InputChannel(1)]` (frame A, frame B). Uniforms include `u_time`,
//! `u_resolution`, plus any `Tween` / `CssProp` / `Audio*` slots the
//! composition references.
//!
//! For v0 we only honor:
//! - `Time` (we pass the frame's seconds-from-comp-start)
//! - `Resolution` (frame width × height)
//! - `CssProp("progress")` (the transition's normalized progress, 0..1)
//! - `Tween` (sampled at the current time)
//!
//! Anything else is bound to zero — we'll wire audio + beat + arbitrary
//! props as the use case shows up.

use wavelet_fx::{compile, parse, EmitOutput, TextureRef, UniformKind};
use std::sync::Arc;
use wgpu::util::DeviceExt;

/// Where the shader source comes from for a transition declaration.
#[derive(Debug, Clone)]
pub enum TransitionSource {
    /// Compiled wavelet_fx output (caller already ran the compile).
    Compiled(EmitOutput),
}

/// Build a `TransitionSource` from a wavelet_fx text source. Returns an error
/// containing wavelet_fx's structured diagnostic on parse / compile failure.
pub fn fx_source(src: &str) -> Result<TransitionSource, String> {
    let comp = parse(src).map_err(|d| format!("wavelet_fx parse: {d:?}"))?;
    let out = compile(&comp).map_err(|d| format!("wavelet_fx compile: {d:?}"))?;
    Ok(TransitionSource::Compiled(out))
}

/// A built shader pipeline ready to consume frame pairs.
pub struct TransitionPipeline {
    width: u32,
    height: u32,
    device: Arc<wgpu::Device>,
    queue: Arc<wgpu::Queue>,
    pipeline: wgpu::RenderPipeline,
    bind_group: wgpu::BindGroup,
    uniform_buf: wgpu::Buffer,
    tex_a: wgpu::Texture,
    tex_b: wgpu::Texture,
    out_tex: wgpu::Texture,
    out_view: wgpu::TextureView,
    /// CPU-readable buffer the GPU copies into at end of each render.
    readback: wgpu::Buffer,
    /// Padded row stride wgpu requires for cross-CPU-GPU copies.
    bytes_per_row: u32,
    /// Layout of the wavelet_fx emit's uniform table — order matches what we
    /// write into `uniform_buf` per frame.
    uniform_kinds: Vec<UniformKind>,
    /// Pre-pass instructions from wavelet_fx's emit. Applied per-frame
    /// before the main shader runs — see `pub fn render`.
    pre_effects: Vec<wavelet_fx::PreEffect>,
    /// GPU blur pipelines for any input channel that wavelet_fx requested
    /// via `PreEffect::GpuBlur`. Keyed by `input_channel`; the bind
    /// group binds the corresponding `blurred_view` in place of the
    /// raw `tex_a`/`tex_b` view. Per-frame `render()` records both
    /// blur passes into the command encoder before the main pass.
    gpu_blurs: Vec<(u32, super::gpu_blur::GpuBlurPipeline)>,
}

/// CPU mirror of the WGSL Uniforms struct emitted by wavelet_fx, with proper
/// std140-ish alignment. Each member is written at its WGSL natural
/// alignment: f32 → 4, vec2 → 8, vec3/vec4 → 16. The shader reads
/// `u_time` at offset 0, then `u_resolution: vec2<f32>` at the next
/// 8-byte boundary, then any further f32 scalars at 4-byte boundaries.
/// Mismatching this alignment silently scrambles every uniform after the
/// first vec — the shader keeps running but reads zeros where it expected
/// our data.
fn pack_uniforms(kinds: &[UniformKind], t_secs: f32, w: u32, h: u32, progress: f32) -> Vec<u8> {
    let mut buf: Vec<u8> = Vec::new();

    fn align_to(buf: &mut Vec<u8>, align: usize) {
        while buf.len() % align != 0 {
            buf.push(0);
        }
    }
    fn push_f32(buf: &mut Vec<u8>, v: f32) {
        buf.extend_from_slice(&v.to_le_bytes());
    }

    for k in kinds {
        match k {
            UniformKind::Time => {
                align_to(&mut buf, 4);
                push_f32(&mut buf, t_secs);
            }
            UniformKind::Resolution => {
                // vec2<f32> requires 8-byte alignment.
                align_to(&mut buf, 8);
                push_f32(&mut buf, w as f32);
                push_f32(&mut buf, h as f32);
            }
            UniformKind::Tween(t) => {
                align_to(&mut buf, 4);
                push_f32(&mut buf, t.value());
            }
            UniformKind::Constant(c) => {
                align_to(&mut buf, 4);
                push_f32(&mut buf, *c);
            }
            UniformKind::CssProp(name) if name == "progress" => {
                align_to(&mut buf, 4);
                push_f32(&mut buf, progress);
            }
            UniformKind::CssProp(_)
            | UniformKind::Seed
            | UniformKind::Beat
            | UniformKind::AudioRms
            | UniformKind::AudioFftBin(_) => {
                align_to(&mut buf, 4);
                push_f32(&mut buf, 0.0);
            }
        }
    }
    // Whole struct rounds up to 16-byte alignment for std140 compatibility.
    align_to(&mut buf, 16);
    buf
}

/// Full-screen vertex shader the transition pipeline always uses. Emits a
/// triangle that covers the viewport in clip space (-1..1) and passes
/// 0..1 UV through to the fragment stage.
const VERTEX_WGSL: &str = r#"
struct VsOut {
  @builtin(position) pos: vec4<f32>,
  @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) vid: u32) -> VsOut {
  // Big triangle covering clip space. Two of three vertices fall outside
  // the [-1,1] box so we don't waste fragments outside the viewport.
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

impl TransitionPipeline {
    /// Build a pipeline at `(width, height)` from a wavelet_fx source. Allocates
    /// all GPU resources once; subsequent `render` calls reuse them.
    pub fn new(
        device: Arc<wgpu::Device>,
        queue: Arc<wgpu::Queue>,
        width: u32,
        height: u32,
        source: &TransitionSource,
    ) -> Result<Self, String> {
        let TransitionSource::Compiled(emit) = source;
        let pass = emit
            .passes
            .first()
            .ok_or_else(|| "wavelet_fx emit has no passes".to_string())?;

        // Determine how many texture bindings the shader expects. We support
        // up to two for transitions (frame A + frame B); anything else fails
        // fast so we don't try to bind nothing useful.
        let n_input_channels = pass
            .inputs
            .iter()
            .filter(|t| matches!(t, TextureRef::InputChannel(_)))
            .count();
        if n_input_channels > 2 {
            return Err(format!(
                "transition shader has {n_input_channels} input channels — only 2 supported"
            ));
        }

        let bytes_per_row = align_up(width * 4, 256);

        // Input textures: scene A, scene B.
        let tex_a = create_input_texture(&device, width, height, "transition_input_a");
        let tex_b = create_input_texture(&device, width, height, "transition_input_b");
        let tex_a_view = tex_a.create_view(&wgpu::TextureViewDescriptor::default());
        let tex_b_view = tex_b.create_view(&wgpu::TextureViewDescriptor::default());

        // For each `PreEffect::GpuBlur`, allocate a 2-pass blur
        // pipeline. The corresponding `blurred_view` replaces the raw
        // tex_a/tex_b view in the bind group below. CpuBlur is handled
        // in `render()` per-frame.
        let mut gpu_blurs: Vec<(u32, super::gpu_blur::GpuBlurPipeline)> = Vec::new();
        for effect in &pass.pre_effects {
            if let wavelet_fx::PreEffect::GpuBlur {
                input_channel,
                radius,
            } = effect
            {
                let input_view = match input_channel {
                    0 => &tex_a_view,
                    1 => &tex_b_view,
                    other => {
                        return Err(format!(
                            "wavelet_fx GpuBlur on input_channel {other} — transition \
                             pipeline only has 2 channels (0, 1)"
                        ));
                    }
                };
                let blur = super::gpu_blur::GpuBlurPipeline::new(
                    &device,
                    queue.clone(),
                    input_view,
                    width,
                    height,
                    wgpu::TextureFormat::Rgba8Unorm,
                    *radius,
                );
                gpu_blurs.push((*input_channel, blur));
            }
        }

        // Output texture (we render into this) + a CPU-readable buffer we
        // copy the result into for the encoder.
        let out_tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("transition_output"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let out_view = out_tex.create_view(&wgpu::TextureViewDescriptor::default());
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("transition_readback"),
            size: (bytes_per_row * height) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("transition_sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        let uniform_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("transition_uniforms"),
            // Allocate enough for any reasonable wavelet_fx emit; pad up.
            contents: &[0u8; 1024],
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        // Bind group layout: uniforms at 0, then iChannel0/sampler at 1/2,
        // then iChannel1/sampler at 3/4 (matching wavelet_fx's emission order).
        let mut layout_entries: Vec<wgpu::BindGroupLayoutEntry> = Vec::new();
        layout_entries.push(wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        });
        let mut next_binding: u32 = 1;
        for _ in 0..n_input_channels {
            layout_entries.push(wgpu::BindGroupLayoutEntry {
                binding: next_binding,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            });
            next_binding += 1;
            layout_entries.push(wgpu::BindGroupLayoutEntry {
                binding: next_binding,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            });
            next_binding += 1;
        }
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("transition_bgl"),
            entries: &layout_entries,
        });

        // Helper: when channel `ch` has an associated GpuBlur, the main
        // shader reads from the blurred view; otherwise it reads from
        // the raw input view.
        let view_for_channel = |ch: u32| -> &wgpu::TextureView {
            for (c, blur) in &gpu_blurs {
                if *c == ch {
                    return &blur.blurred_view;
                }
            }
            if ch == 0 { &tex_a_view } else { &tex_b_view }
        };

        // Bind group entries — order matches the layout above.
        let mut entries: Vec<wgpu::BindGroupEntry> = Vec::new();
        entries.push(wgpu::BindGroupEntry {
            binding: 0,
            resource: uniform_buf.as_entire_binding(),
        });
        let mut next_binding: u32 = 1;
        if n_input_channels >= 1 {
            entries.push(wgpu::BindGroupEntry {
                binding: next_binding,
                resource: wgpu::BindingResource::TextureView(view_for_channel(0)),
            });
            next_binding += 1;
            entries.push(wgpu::BindGroupEntry {
                binding: next_binding,
                resource: wgpu::BindingResource::Sampler(&sampler),
            });
            next_binding += 1;
        }
        if n_input_channels >= 2 {
            entries.push(wgpu::BindGroupEntry {
                binding: next_binding,
                resource: wgpu::BindingResource::TextureView(view_for_channel(1)),
            });
            next_binding += 1;
            entries.push(wgpu::BindGroupEntry {
                binding: next_binding,
                resource: wgpu::BindingResource::Sampler(&sampler),
            });
        }
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("transition_bg"),
            layout: &bind_group_layout,
            entries: &entries,
        });

        // Shader modules — our vertex stage, wavelet_fx's fragment stage.
        let vs = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("transition_vs"),
            source: wgpu::ShaderSource::Wgsl(VERTEX_WGSL.into()),
        });
        let fs = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("transition_fs"),
            source: wgpu::ShaderSource::Wgsl(pass.wgsl.clone().into()),
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("transition_pl"),
            bind_group_layouts: &[&bind_group_layout],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("transition_pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &vs,
                entry_point: Some("vs_main"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &fs,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: wgpu::TextureFormat::Rgba8Unorm,
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
        });

        Ok(Self {
            width,
            height,
            device,
            queue,
            pipeline,
            bind_group,
            uniform_buf,
            tex_a,
            tex_b,
            out_tex,
            out_view,
            readback,
            bytes_per_row,
            uniform_kinds: emit.uniforms.iter().map(|u| u.kind.clone()).collect(),
            pre_effects: pass.pre_effects.clone(),
            gpu_blurs,
        })
    }

    /// Render one transition frame.
    ///
    /// `frame_a` and `frame_b` are RGBA8 buffers `width*height*4` bytes long
    /// (the outgoing and incoming scene). `t_secs` is the absolute frame
    /// time in the composition (for `u_time` and tween sampling).
    /// `progress` is the normalized 0..1 progress through the transition
    /// window (drives `CssProp("progress")`).
    pub fn render(
        &mut self,
        frame_a: &[u8],
        frame_b: &[u8],
        t_secs: f32,
        progress: f32,
    ) -> Vec<u8> {
        // Update uniforms.
        let bytes = pack_uniforms(&self.uniform_kinds, t_secs, self.width, self.height, progress);
        self.queue.write_buffer(&self.uniform_buf, 0, &bytes);

        // Apply CPU pre-effects (PreEffect::CpuBlur fallback). GpuBlur
        // is encoded as a real GPU pass below — no byte-level work.
        // wavelet_fx picks the variant per emit; consumers may not need both
        // simultaneously but the dispatcher handles each correctly.
        let (frame_a_owned, frame_b_owned) = apply_cpu_pre_effects(
            &self.pre_effects,
            frame_a,
            frame_b,
            self.width,
            self.height,
        );
        let frame_a_ref: &[u8] = frame_a_owned.as_deref().unwrap_or(frame_a);
        let frame_b_ref: &[u8] = frame_b_owned.as_deref().unwrap_or(frame_b);

        // Upload (possibly CPU-pre-effected) input frames.
        upload_rgba(&self.queue, &self.tex_a, frame_a_ref, self.width, self.height);
        upload_rgba(&self.queue, &self.tex_b, frame_b_ref, self.width, self.height);

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("transition_encoder"),
            });

        // GpuBlur pre-passes — separable Gaussian on each requested
        // input channel. Each blur pipeline writes its `blurred_view`,
        // which the main bind group already points at; the main pass
        // below samples the blurred output transparently.
        for (_ch, blur) in &self.gpu_blurs {
            blur.encode(&mut encoder);
        }
        {
            let mut rp = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("transition_rp"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.out_view,
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
            rp.set_pipeline(&self.pipeline);
            rp.set_bind_group(0, &self.bind_group, &[]);
            rp.draw(0..3, 0..1);
        }

        // Copy the rendered output texture to a CPU-mappable buffer.
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &self.out_tex,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &self.readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(self.bytes_per_row),
                    rows_per_image: Some(self.height),
                },
            },
            wgpu::Extent3d {
                width: self.width,
                height: self.height,
                depth_or_array_layers: 1,
            },
        );
        self.queue.submit(std::iter::once(encoder.finish()));

        // Map + read. wgpu requires the buffer to be unmapped before we map
        // again next frame; we drop the slice + unmap at the end.
        let slice = self.readback.slice(..);
        let (sender, receiver) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| sender.send(r).unwrap());
        // Block until the GPU finishes our submission so the readback is
        // ready to map.
        let _ = self.device.poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: None,
        });
        receiver
            .recv()
            .expect("readback channel")
            .expect("buffer map");
        let data = slice.get_mapped_range();

        // De-pad rows back to width*4 bytes.
        let row_bytes = (self.width * 4) as usize;
        let mut out = Vec::with_capacity(row_bytes * self.height as usize);
        for y in 0..self.height as usize {
            let base = y * self.bytes_per_row as usize;
            out.extend_from_slice(&data[base..base + row_bytes]);
        }
        drop(data);
        self.readback.unmap();
        out
    }

    /// Output dimensions.
    pub fn dimensions(&self) -> (u32, u32) {
        (self.width, self.height)
    }
}

/// Walk wavelet_fx's PreEffect list and apply CPU-side effects to the input
/// bytes. Returns owned `Vec<u8>` for any channel that had effects
/// applied; `None` if the channel was untouched (callers can keep using
/// the original `&[u8]`).
///
/// Only `PreEffect::CpuBlur` is handled here — `PreEffect::GpuBlur` is
/// encoded into the command buffer in the main render path and runs
/// against the already-uploaded texture, so this function ignores it.
fn apply_cpu_pre_effects(
    effects: &[wavelet_fx::PreEffect],
    frame_a: &[u8],
    frame_b: &[u8],
    width: u32,
    height: u32,
) -> (Option<Vec<u8>>, Option<Vec<u8>>) {
    if effects.is_empty() {
        return (None, None);
    }
    let mut a: Option<Vec<u8>> = None;
    let mut b: Option<Vec<u8>> = None;
    for effect in effects {
        match effect {
            wavelet_fx::PreEffect::CpuBlur {
                input_channel,
                radius,
            } => {
                let target = match input_channel {
                    0 => &mut a,
                    1 => &mut b,
                    other => {
                        eprintln!(
                            "wavelet: PreEffect::CpuBlur on input_channel {} ignored \
                             — transition pipeline only has 2 channels (0,1)",
                            other
                        );
                        continue;
                    }
                };
                if target.is_none() {
                    *target = Some(if *input_channel == 0 {
                        frame_a.to_vec()
                    } else {
                        frame_b.to_vec()
                    });
                }
                if let Some(buf) = target.as_mut() {
                    super::blur::gaussian_rgba(buf, width, height, *radius);
                }
            }
            wavelet_fx::PreEffect::GpuBlur { .. } => {
                // Handled by the GpuBlurPipeline in TransitionPipeline::render.
            }
        }
    }
    (a, b)
}

fn create_input_texture(device: &wgpu::Device, w: u32, h: u32, label: &str) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    })
}

fn upload_rgba(queue: &wgpu::Queue, tex: &wgpu::Texture, bytes: &[u8], w: u32, h: u32) {
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture: tex,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        bytes,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(w * 4),
            rows_per_image: Some(h),
        },
        wgpu::Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: 1,
        },
    );
}

fn align_up(n: u32, a: u32) -> u32 {
    n.div_ceil(a) * a
}
