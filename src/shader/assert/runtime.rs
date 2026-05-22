//! Higher-level shader-assertion runtime (wb-mxrk.7).
//!
//! `run_assertion` + `run_assertion_batch` execute one or more
//! `ShaderAssertion` against a shared `GpuContext`. Pipelines are cached
//! by `shader_id`; workgroup sizes come from naga reflection (not string
//! search). Batch path queues every dispatch + readback copy into a
//! single `CommandEncoder`, a single `queue.submit`, a single
//! `device.poll(Wait)`.

use std::sync::Arc;

use anyhow::{anyhow, Context, Result};
use bytemuck::Zeroable;
use wgpu::util::DeviceExt;

use super::context::GpuContext;
use super::types::{
    AssertionOutcome, AssertionResult, ReasonCode, EVIDENCE_CAPACITY, PARAMS_MAX_BYTES,
};

const COLOR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
const ID_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R32Uint;
const COVERAGE_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R32Float;

/// Opaque wrapper around a `wgpu::Texture` plus a back-reference to the
/// `GpuContext` it was created from. The runtime checks pointer equality
/// on `Arc<Device>` to refuse cross-context dispatch — that's the bug
/// wb-mxrk.5 hit when primitives created their own devices.
#[derive(Clone)]
pub struct TextureHandle {
    pub(crate) texture: Arc<wgpu::Texture>,
    pub(crate) device: Arc<wgpu::Device>,
}

impl TextureHandle {
    /// Upload tightly-packed RGBA8 pixels into a `Rgba8Unorm` texture
    /// bound to this context.
    pub fn from_rgba8(ctx: &GpuContext, width: u32, height: u32, pixels: &[u8]) -> Self {
        let texture = ctx.device().create_texture(&wgpu::TextureDescriptor {
            label: Some("texhandle_rgba8"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: COLOR_FORMAT,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        ctx.queue().write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            pixels,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(width * 4),
                rows_per_image: Some(height),
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
        Self {
            texture: Arc::new(texture),
            device: ctx.device().clone(),
        }
    }

    /// Decode a PNG from disk + upload as an RGBA8 texture.
    pub fn from_png(ctx: &GpuContext, path: &std::path::Path) -> Result<Self> {
        let file = std::fs::File::open(path)
            .with_context(|| format!("open png {}", path.display()))?;
        let decoder = png::Decoder::new(file);
        let mut reader = decoder.read_info()?;
        let mut buf = vec![0u8; reader.output_buffer_size()];
        let info = reader.next_frame(&mut buf)?;
        let pixels = match info.color_type {
            png::ColorType::Rgba => buf[..info.buffer_size()].to_vec(),
            png::ColorType::Rgb => {
                let bytes = &buf[..info.buffer_size()];
                let mut out = Vec::with_capacity(bytes.len() / 3 * 4);
                for px in bytes.chunks_exact(3) {
                    out.extend_from_slice(px);
                    out.push(255);
                }
                out
            }
            other => return Err(anyhow!("unsupported PNG color type: {other:?}")),
        };
        Ok(Self::from_rgba8(ctx, info.width, info.height, &pixels))
    }

    /// Wrap an externally-created texture that is known to belong to
    /// this context. The caller is responsible for the provenance —
    /// `run_assertion` will still assert device identity at dispatch.
    pub fn from_texture(ctx: &GpuContext, texture: wgpu::Texture) -> Self {
        Self {
            texture: Arc::new(texture),
            device: ctx.device().clone(),
        }
    }

    /// Texture width in pixels.
    pub fn width(&self) -> u32 {
        self.texture.width()
    }

    /// Texture height in pixels.
    pub fn height(&self) -> u32 {
        self.texture.height()
    }
}

/// One assertion-shader invocation. `params` is pre-marshalled into the
/// uniform buffer's wire format — the first 8 bytes are reserved for
/// `frame_width, frame_height` and the runtime writes them itself, so
/// callers only need to encode their shader-specific tail bytes (or pass
/// `Vec::new()` for parameter-less shaders).
pub struct ShaderAssertion {
    /// Stable identifier used as the pipeline-cache key + the debug
    /// label. Convention: shader file path or module name.
    pub shader_id: &'static str,
    /// Verbatim WGSL source.
    pub wgsl: &'static str,
    /// Parameter tail bytes (shader-specific). Combined with the
    /// `frame_width/frame_height` prefix the runtime writes.
    pub params: Vec<u8>,
    /// Color texture (binding 0).
    pub frame: TextureHandle,
    /// Optional id/coverage sidecar (binding 1 + 2). Currently maps to
    /// the id-texture slot; coverage is a 1x1 placeholder.
    pub sidecar: Option<TextureHandle>,
    /// Optional golden reference (binding 1 fallback for shaders that
    /// don't use sidecar — golden_rmse rebinds this in its own
    /// dispatcher; in the generic runtime it's currently unused and
    /// reserved for future ABI expansion).
    pub reference: Option<TextureHandle>,
}

/// Single-assertion dispatch. Internally builds a one-element batch.
pub fn run_assertion(ctx: &GpuContext, a: ShaderAssertion) -> Result<AssertionOutcome> {
    let mut outs = run_assertion_batch(ctx, std::slice::from_ref(&a))?;
    Ok(outs.pop().unwrap())
}

/// Batch dispatch: one encoder, one submit, one poll. Returns outcomes
/// in the declared order.
pub fn run_assertion_batch(
    ctx: &GpuContext,
    batch: &[ShaderAssertion],
) -> Result<Vec<AssertionOutcome>> {
    if batch.is_empty() {
        return Ok(Vec::new());
    }
    let device = ctx.device();
    let queue = ctx.queue();
    let layout = standard_bind_group_layout(device);
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("assert_pl"),
        bind_group_layouts: &[&layout],
        immediate_size: 0,
    });

    let mut per_call: Vec<PerCall> = Vec::with_capacity(batch.len());

    for a in batch {
        if !Arc::ptr_eq(&a.frame.device, device) {
            return Err(anyhow!(
                "shader {} frame texture belongs to a different GpuContext",
                a.shader_id
            ));
        }
        if let Some(s) = &a.sidecar {
            if !Arc::ptr_eq(&s.device, device) {
                return Err(anyhow!(
                    "shader {} sidecar texture belongs to a different GpuContext",
                    a.shader_id
                ));
            }
        }

        let (wg_x, wg_y, _wg_z) = reflect_workgroup_size(a.wgsl, "assert_main")
            .unwrap_or((8, 8, 1));
        let pipeline =
            ctx.get_or_compile_with(a.shader_id, a.wgsl, "assert_main", Some(&pipeline_layout))?;

        let width = a.frame.width();
        let height = a.frame.height();

        let id_texture = match &a.sidecar {
            Some(s) => s.texture.clone(),
            None => Arc::new(placeholder_texture(device, ID_FORMAT, "assert_id_ph")),
        };
        let coverage_texture =
            Arc::new(placeholder_texture(device, COVERAGE_FORMAT, "assert_cov_ph"));

        let color_view = a
            .frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let id_view = id_texture.create_view(&wgpu::TextureViewDescriptor::default());
        let coverage_view = coverage_texture.create_view(&wgpu::TextureViewDescriptor::default());

        let params_buf = build_params_buffer(device, width, height, &a.params);
        let result_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("assert_result"),
            size: AssertionResult::SIZE as u64,
            usage: wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_SRC
                | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        queue.write_buffer(
            &result_buf,
            0,
            bytemuck::bytes_of(&AssertionResult::zeroed()),
        );
        let staging = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("assert_staging"),
            size: AssertionResult::SIZE as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("assert_bg"),
            layout: &layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&color_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&id_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&coverage_view),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: params_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: result_buf.as_entire_binding(),
                },
            ],
        });

        per_call.push(PerCall {
            pipeline,
            bind_group,
            result_buf,
            staging,
            wg_x,
            wg_y,
            width,
            height,
            shader_id: a.shader_id,
        });
    }

    let mut encoder =
        device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("assert_batch") });
    for call in &per_call {
        {
            let mut cpass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some(call.shader_id),
                timestamp_writes: None,
            });
            cpass.set_pipeline(&call.pipeline);
            cpass.set_bind_group(0, &call.bind_group, &[]);
            let groups_x = call.width.div_ceil(call.wg_x).max(1);
            let groups_y = call.height.div_ceil(call.wg_y).max(1);
            cpass.dispatch_workgroups(groups_x, groups_y, 1);
        }
        encoder.copy_buffer_to_buffer(
            &call.result_buf,
            0,
            &call.staging,
            0,
            AssertionResult::SIZE as u64,
        );
    }
    queue.submit(Some(encoder.finish()));

    let mut receivers = Vec::with_capacity(per_call.len());
    for call in &per_call {
        let slice = call.staging.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |v| {
            let _ = tx.send(v);
        });
        receivers.push(rx);
    }
    device
        .poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: None,
        })
        .map_err(|e| anyhow!("device poll failed: {e:?}"))?;

    let mut out = Vec::with_capacity(per_call.len());
    for (call, rx) in per_call.iter().zip(receivers.into_iter()) {
        rx.recv()
            .map_err(|e| anyhow!("staging channel closed: {e}"))?
            .map_err(|e| anyhow!("buffer map failed: {e:?}"))?;
        let slice = call.staging.slice(..);
        let data = slice.get_mapped_range();
        let result: AssertionResult = *bytemuck::from_bytes(&data);
        drop(data);
        call.staging.unmap();

        let evidence_len = (result.evidence_count as usize).min(EVIDENCE_CAPACITY);
        out.push(AssertionOutcome {
            passed: result.passed == 1,
            reason_code: result.reason_code,
            reason: ReasonCode::from_raw(result.reason_code).as_str(),
            evidence: result.evidence[..evidence_len].to_vec(),
        });
    }
    Ok(out)
}

struct PerCall {
    pipeline: Arc<wgpu::ComputePipeline>,
    bind_group: wgpu::BindGroup,
    result_buf: wgpu::Buffer,
    staging: wgpu::Buffer,
    wg_x: u32,
    wg_y: u32,
    width: u32,
    height: u32,
    shader_id: &'static str,
}

fn standard_bind_group_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("assert_bgl"),
        entries: &[
            texture_entry(0, wgpu::TextureSampleType::Float { filterable: false }),
            texture_entry(1, wgpu::TextureSampleType::Uint),
            texture_entry(2, wgpu::TextureSampleType::Float { filterable: false }),
            wgpu::BindGroupLayoutEntry {
                binding: 3,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 4,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: false },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
        ],
    })
}

fn texture_entry(binding: u32, sample_type: wgpu::TextureSampleType) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Texture {
            sample_type,
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }
}

fn placeholder_texture(
    device: &wgpu::Device,
    format: wgpu::TextureFormat,
    label: &'static str,
) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width: 1,
            height: 1,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    })
}

fn build_params_buffer(
    device: &wgpu::Device,
    width: u32,
    height: u32,
    tail: &[u8],
) -> wgpu::Buffer {
    let mut bytes = vec![0u8; PARAMS_MAX_BYTES];
    bytes[0..4].copy_from_slice(&width.to_le_bytes());
    bytes[4..8].copy_from_slice(&height.to_le_bytes());
    let copy_len = tail.len().min(PARAMS_MAX_BYTES - 8);
    bytes[8..8 + copy_len].copy_from_slice(&tail[..copy_len]);
    device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("assert_params"),
        contents: &bytes,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
    })
}

/// Parse the WGSL with naga and read the compute entry-point's
/// `@workgroup_size`. Falls back to `None` when the entry-point isn't
/// found or the WGSL doesn't parse (caller defaults to (8, 8, 1)).
pub(crate) fn reflect_workgroup_size(wgsl: &str, entry: &str) -> Option<(u32, u32, u32)> {
    let module = naga::front::wgsl::parse_str(wgsl).ok()?;
    let ep = module
        .entry_points
        .iter()
        .find(|e| e.name == entry && e.stage == naga::ShaderStage::Compute)?;
    Some((ep.workgroup_size[0], ep.workgroup_size[1], ep.workgroup_size[2]))
}
