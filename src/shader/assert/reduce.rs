//! WGSL reduction primitive: sum / max / mean / variance over a 2D
//! region of a `FrameSource`. wb-mxrk.2.
//!
//! Two-pass strategy: pass 1 (`reduce_tile`) reduces each 16x16 tile of
//! the region into one partial result via workgroup-shared memory; pass 2
//! (`reduce_pass`) collapses partials with the same shared-memory pattern
//! and iterates until a single value remains. This handles full 1080p
//! frames (~2M pixels, ~8100 partials, two extra passes) without ever
//! relying on subgroup ops — the manual barrier path is portable to every
//! wgpu backend including the OpenGL fallback. The scalar variant
//! averages the RGBA channels of the reduced vector; `reduce_rgba`
//! returns the per-channel vector directly.

use std::borrow::Cow;

use anyhow::{anyhow, Context, Result};
use bytemuck::{Pod, Zeroable};
use wgpu::util::DeviceExt;

use super::types::FrameSource;

/// 2D pixel rectangle, exclusive on the high side.
#[derive(Clone, Copy, Debug)]
pub struct Rect {
    /// Left edge, in pixels.
    pub x: u32,
    /// Top edge, in pixels.
    pub y: u32,
    /// Region width, in pixels.
    pub width: u32,
    /// Region height, in pixels.
    pub height: u32,
}

impl Rect {
    /// Pixel count inside the rectangle (`width * height`).
    pub fn area(self) -> u32 {
        self.width * self.height
    }
}

/// Reduction operation. Variance is computed as a two-pass mean +
/// sum-of-squared-deviations, divided by pixel count.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReduceOp {
    /// Per-channel sum of pixel values (normalized 0..1 for Rgba8Unorm).
    Sum,
    /// Per-channel maximum pixel value.
    Max,
    /// Per-channel arithmetic mean: `Sum / area`.
    Mean,
    /// Per-channel population variance: `E[(x - mean)^2]`.
    Variance,
}

const WG: u32 = 16;
const TILE: u32 = WG * WG;
const OP_SUM: u32 = 0;
const OP_MAX: u32 = 1;
const OP_SOS: u32 = 3;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct ReduceParams {
    region_x: u32,
    region_y: u32,
    region_w: u32,
    region_h: u32,
    op: u32,
    pass_count: u32,
    mean_r: f32,
    mean_g: f32,
    mean_b: f32,
    mean_a: f32,
    _pad: [u32; 2],
}

/// Scalar reduction over a region. RGBA inputs are collapsed to a single
/// channel by averaging R+G+B (alpha dropped) — callers needing per-channel
/// results should use `reduce_rgba`.
pub fn reduce(frame: &FrameSource, region: Rect, op: ReduceOp) -> Result<f32> {
    let v = reduce_rgba(frame, region, op)?;
    Ok((v[0] + v[1] + v[2]) / 3.0)
}

/// Per-channel reduction over a region. For `Mean`/`Variance` the divisor
/// is `region.area()`; for `Sum`/`Max` the channels are returned as-is.
pub fn reduce_rgba(frame: &FrameSource, region: Rect, op: ReduceOp) -> Result<[f32; 4]> {
    if region.width == 0 || region.height == 0 {
        return Ok([0.0; 4]);
    }
    let ctx = GpuCtx::new()?;
    let texture = upload_frame(&ctx, frame)?;
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    run_reduce(&ctx, &view, region, op)
}

fn run_reduce(
    ctx: &GpuCtx,
    view: &wgpu::TextureView,
    region: Rect,
    op: ReduceOp,
) -> Result<[f32; 4]> {
    match op {
        ReduceOp::Sum => reduce_op(ctx, view, region, OP_SUM, [0.0; 4]),
        ReduceOp::Max => reduce_op(ctx, view, region, OP_MAX, [0.0; 4]),
        ReduceOp::Mean => {
            let sum = reduce_op(ctx, view, region, OP_SUM, [0.0; 4])?;
            let n = region.area() as f32;
            Ok([sum[0] / n, sum[1] / n, sum[2] / n, sum[3] / n])
        }
        ReduceOp::Variance => {
            let sum = reduce_op(ctx, view, region, OP_SUM, [0.0; 4])?;
            let n = region.area() as f32;
            let mean = [sum[0] / n, sum[1] / n, sum[2] / n, sum[3] / n];
            let sos = reduce_op(ctx, view, region, OP_SOS, mean)?;
            Ok([sos[0] / n, sos[1] / n, sos[2] / n, sos[3] / n])
        }
    }
}

fn reduce_op(
    ctx: &GpuCtx,
    view: &wgpu::TextureView,
    region: Rect,
    op: u32,
    mean: [f32; 4],
) -> Result<[f32; 4]> {
    let groups_x = region.width.div_ceil(WG);
    let groups_y = region.height.div_ceil(WG);
    let tile_count = groups_x * groups_y;

    let tile_buf = ctx.partials_buffer(tile_count);
    let zero_partials = ctx.partials_buffer(1);
    let params = ReduceParams {
        region_x: region.x,
        region_y: region.y,
        region_w: region.width,
        region_h: region.height,
        op,
        pass_count: 0,
        mean_r: mean[0],
        mean_g: mean[1],
        mean_b: mean[2],
        mean_a: mean[3],
        _pad: [0; 2],
    };
    let params_buf = ctx.params_buffer(&params);

    let bg = ctx.bind_group(view, &params_buf, &zero_partials, &tile_buf);

    let mut encoder =
        ctx.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
    {
        let mut cpass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("reduce_tile"),
            timestamp_writes: None,
        });
        cpass.set_pipeline(&ctx.tile_pipeline);
        cpass.set_bind_group(0, &bg, &[]);
        cpass.dispatch_workgroups(groups_x, groups_y, 1);
    }
    ctx.queue.submit(Some(encoder.finish()));

    let mut current = tile_buf;
    let mut current_count = tile_count;
    while current_count > 1 {
        let next_count = current_count.div_ceil(TILE);
        let next = ctx.partials_buffer(next_count);

        let pass_params = ReduceParams {
            pass_count: current_count,
            op,
            ..params
        };
        let pp_buf = ctx.params_buffer(&pass_params);

        let bg2 = ctx.bind_group(view, &pp_buf, &current, &next);

        let mut enc =
            ctx.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
        {
            let mut cpass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("reduce_pass"),
                timestamp_writes: None,
            });
            cpass.set_pipeline(&ctx.pass_pipeline);
            cpass.set_bind_group(0, &bg2, &[]);
            cpass.dispatch_workgroups(next_count, 1, 1);
        }
        ctx.queue.submit(Some(enc.finish()));

        current = next;
        current_count = next_count;
    }

    readback_first(ctx, &current)
}

fn readback_first(ctx: &GpuCtx, buf: &wgpu::Buffer) -> Result<[f32; 4]> {
    let staging = ctx.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("reduce_staging"),
        size: 16,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut enc =
        ctx.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
    enc.copy_buffer_to_buffer(buf, 0, &staging, 0, 16);
    ctx.queue.submit(Some(enc.finish()));

    let slice = staging.slice(..);
    let (tx, rx) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |v| {
        let _ = tx.send(v);
    });
    ctx.device
        .poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: None,
        })
        .map_err(|e| anyhow!("device poll failed: {e:?}"))?;
    rx.recv()
        .map_err(|e| anyhow!("staging channel closed: {e}"))?
        .map_err(|e| anyhow!("buffer map failed: {e:?}"))?;
    let data = slice.get_mapped_range();
    let floats: [f32; 4] = *bytemuck::from_bytes(&data[..16]);
    drop(data);
    staging.unmap();
    Ok(floats)
}

struct GpuCtx {
    device: wgpu::Device,
    queue: wgpu::Queue,
    layout: wgpu::BindGroupLayout,
    tile_pipeline: wgpu::ComputePipeline,
    pass_pipeline: wgpu::ComputePipeline,
}

impl GpuCtx {
    fn new() -> Result<Self> {
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor::default());
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: None,
            force_fallback_adapter: false,
        }))
        .map_err(|e| anyhow!("no wgpu adapter: {e}"))?;
        let (device, queue) =
            pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
                label: Some("gamut_reduce_device"),
                required_features: wgpu::Features::empty(),
                required_limits: wgpu::Limits::default(),
                memory_hints: wgpu::MemoryHints::default(),
                trace: wgpu::Trace::Off,
                experimental_features: wgpu::ExperimentalFeatures::disabled(),
            }))
            .map_err(|e| anyhow!("device request failed: {e}"))?;

        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("reduce.wgsl"),
            source: wgpu::ShaderSource::Wgsl(Cow::Borrowed(include_str!("reduce.wgsl"))),
        });

        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("reduce_bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: false },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("reduce_pl"),
            bind_group_layouts: &[&layout],
            immediate_size: 0,
        });

        let tile_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("reduce_tile_pipeline"),
            layout: Some(&pipeline_layout),
            module: &module,
            entry_point: Some("reduce_tile"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            cache: None,
        });
        let pass_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("reduce_pass_pipeline"),
            layout: Some(&pipeline_layout),
            module: &module,
            entry_point: Some("reduce_pass"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            cache: None,
        });

        Ok(Self {
            device,
            queue,
            layout,
            tile_pipeline,
            pass_pipeline,
        })
    }

    fn params_buffer(&self, p: &ReduceParams) -> wgpu::Buffer {
        self.device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("reduce_params"),
                contents: bytemuck::bytes_of(p),
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            })
    }

    fn partials_buffer(&self, count: u32) -> wgpu::Buffer {
        let size = (count.max(1) as u64) * 16;
        self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("reduce_partials"),
            size,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        })
    }

    fn bind_group(
        &self,
        view: &wgpu::TextureView,
        params: &wgpu::Buffer,
        partials_in: &wgpu::Buffer,
        partials_out: &wgpu::Buffer,
    ) -> wgpu::BindGroup {
        self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("reduce_bg"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: params.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: partials_in.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: partials_out.as_entire_binding(),
                },
            ],
        })
    }
}

fn upload_frame(ctx: &GpuCtx, frame: &FrameSource) -> Result<wgpu::Texture> {
    match frame {
        FrameSource::Texture(t) => Ok(t.clone()),
        FrameSource::Rgba8 {
            width,
            height,
            pixels,
        } => Ok(upload_rgba8(ctx, *width, *height, pixels)),
        FrameSource::PngPath(path) => {
            let file = std::fs::File::open(path)
                .with_context(|| format!("open png {}", path.display()))?;
            let decoder = png::Decoder::new(file);
            let mut reader = decoder.read_info()?;
            let mut buf = vec![0u8; reader.output_buffer_size()];
            let info = reader.next_frame(&mut buf)?;
            let pixels = match info.color_type {
                png::ColorType::Rgba => buf[..info.buffer_size()].to_vec(),
                png::ColorType::Rgb => {
                    let mut out = Vec::with_capacity(info.buffer_size() / 3 * 4);
                    for px in buf[..info.buffer_size()].chunks_exact(3) {
                        out.extend_from_slice(px);
                        out.push(255);
                    }
                    out
                }
                other => return Err(anyhow!("unsupported PNG color type: {other:?}")),
            };
            Ok(upload_rgba8(ctx, info.width, info.height, &pixels))
        }
    }
}

fn upload_rgba8(ctx: &GpuCtx, width: u32, height: u32, pixels: &[u8]) -> wgpu::Texture {
    let texture = ctx.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("reduce_input"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    ctx.queue.write_texture(
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
    texture
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shader::assert::FrameSource;

    fn solid_rgba(width: u32, height: u32, r: u8, g: u8, b: u8, a: u8) -> FrameSource {
        let mut pixels = Vec::with_capacity((width * height * 4) as usize);
        for _ in 0..(width * height) {
            pixels.extend_from_slice(&[r, g, b, a]);
        }
        FrameSource::Rgba8 {
            width,
            height,
            pixels,
        }
    }

    fn gradient_rgba(width: u32, height: u32) -> FrameSource {
        let mut pixels = Vec::with_capacity((width * height * 4) as usize);
        for y in 0..height {
            for x in 0..width {
                let v = ((x + y * width) % 256) as u8;
                pixels.extend_from_slice(&[v, v, v, 255]);
            }
        }
        FrameSource::Rgba8 {
            width,
            height,
            pixels,
        }
    }

    fn region_16() -> Rect {
        Rect { x: 0, y: 0, width: 16, height: 16 }
    }

    #[test]
    fn sum_solid_region() {
        // 16x16 region, value 0.5 per channel → sum = 256 * 0.5 = 128
        let frame = solid_rgba(16, 16, 128, 128, 128, 255);
        let v = reduce_rgba(&frame, region_16(), ReduceOp::Sum).expect("sum");
        let expected = 256.0 * (128.0 / 255.0);
        for ch in 0..3 {
            assert!(
                (v[ch] - expected).abs() < 0.5,
                "channel {ch}: got {} expected {}",
                v[ch],
                expected,
            );
        }
    }

    #[test]
    fn mean_solid_region() {
        let frame = solid_rgba(16, 16, 64, 128, 192, 255);
        let v = reduce_rgba(&frame, region_16(), ReduceOp::Mean).expect("mean");
        assert!((v[0] - 64.0 / 255.0).abs() < 1e-3);
        assert!((v[1] - 128.0 / 255.0).abs() < 1e-3);
        assert!((v[2] - 192.0 / 255.0).abs() < 1e-3);
        assert!((v[3] - 1.0).abs() < 1e-3);
    }

    #[test]
    fn max_gradient_region() {
        let frame = gradient_rgba(16, 16);
        // values 0..255, so max channel = 255/255 = 1.0
        let v = reduce_rgba(&frame, region_16(), ReduceOp::Max).expect("max");
        assert!((v[0] - 1.0).abs() < 1e-3);
        assert!((v[1] - 1.0).abs() < 1e-3);
        assert!((v[2] - 1.0).abs() < 1e-3);
    }

    #[test]
    fn variance_solid_region_is_zero() {
        let frame = solid_rgba(16, 16, 100, 100, 100, 255);
        let v = reduce_rgba(&frame, region_16(), ReduceOp::Variance).expect("var");
        for ch in 0..4 {
            assert!(v[ch].abs() < 1e-4, "channel {ch}: variance = {}", v[ch]);
        }
    }

    #[test]
    fn variance_gradient_region() {
        // gradient 0..255 across 16x16=256 pixels → values are 0,1,2,...,255 each exactly once.
        // CPU-side expected: mean = 127.5/255, variance = sum((v - mean)^2)/256.
        let frame = gradient_rgba(16, 16);
        let v = reduce_rgba(&frame, region_16(), ReduceOp::Variance).expect("var");
        let mean = 127.5_f64 / 255.0;
        let mut sos: f64 = 0.0;
        for i in 0..256u32 {
            let f = (i as f64) / 255.0;
            sos += (f - mean) * (f - mean);
        }
        let expected = (sos / 256.0) as f32;
        for ch in 0..3 {
            assert!(
                (v[ch] - expected).abs() < 1e-3,
                "channel {ch}: got {} expected {}",
                v[ch],
                expected,
            );
        }
    }

    #[test]
    fn scalar_mean_averages_rgb() {
        let frame = solid_rgba(16, 16, 60, 120, 180, 255);
        let s = reduce(&frame, region_16(), ReduceOp::Mean).expect("scalar");
        let expected = ((60.0 + 120.0 + 180.0) / 3.0) / 255.0;
        assert!((s - expected).abs() < 1e-3, "got {s} expected {expected}");
    }

    #[test]
    fn region_offset_inside_larger_frame() {
        // Build a 32x32 frame; top-left 16x16 = white, rest = black.
        let mut pixels = Vec::with_capacity(32 * 32 * 4);
        for y in 0..32u32 {
            for x in 0..32u32 {
                if x < 16 && y < 16 {
                    pixels.extend_from_slice(&[255, 255, 255, 255]);
                } else {
                    pixels.extend_from_slice(&[0, 0, 0, 255]);
                }
            }
        }
        let frame = FrameSource::Rgba8 {
            width: 32,
            height: 32,
            pixels,
        };
        let white = reduce_rgba(&frame, Rect { x: 0, y: 0, width: 16, height: 16 }, ReduceOp::Mean)
            .expect("white");
        let black = reduce_rgba(&frame, Rect { x: 16, y: 16, width: 16, height: 16 }, ReduceOp::Mean)
            .expect("black");
        assert!((white[0] - 1.0).abs() < 1e-3);
        assert!(black[0].abs() < 1e-3);
    }
}
