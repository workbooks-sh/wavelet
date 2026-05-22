//! Atomic histogram primitive (wb-mxrk.3).
//!
//! Standalone GPU reduction — does not go through `dispatch_assertion`
//! because the output (variable-length `Vec<u32>` up to 256 entries) is
//! larger than the 64-float evidence array in the assertion ABI. The
//! result feeds higher-level validators (contrast, color distribution,
//! tonemapping checks) that wrap it in an assertion shape later.
//!
//! See `histogram.wgsl` for the two-stage workgroup-then-global atomic
//! reduction.
//!
//! webgpufundamentals atomic histogram parts 1-2:
//!   https://webgpufundamentals.org/webgpu/lessons/webgpu-compute-shaders-histogram.html

use anyhow::{anyhow, Context, Result};
use wgpu::util::DeviceExt;

use super::reduce::Rect;
use super::types::FrameSource;

const SHADER_SRC: &str = include_str!("histogram.wgsl");
const WG_X: u32 = 8;
const WG_Y: u32 = 8;
const MAX_BINS: u32 = 256;

/// Which channel of the RGBA source to histogram.
#[derive(Clone, Copy, Debug)]
pub enum Channel {
    /// Red channel.
    R,
    /// Green channel.
    G,
    /// Blue channel.
    B,
    /// Alpha channel.
    A,
    /// Rec.709 luminance: 0.2126R + 0.7152G + 0.0722B.
    Luma,
}

impl Channel {
    fn as_u32(self) -> u32 {
        match self {
            Self::R => 0,
            Self::G => 1,
            Self::B => 2,
            Self::A => 3,
            Self::Luma => 4,
        }
    }
}

/// GPU histogram. `bins` must be in `1..=256`. Returns a vector of length
/// `bins` whose values sum to the total in-region pixel count.
pub fn histogram(
    frame: &FrameSource,
    region: Rect,
    channel: Channel,
    bins: u32,
) -> Result<Vec<u32>> {
    if bins == 0 || bins > MAX_BINS {
        return Err(anyhow!("histogram bins must be 1..=256, got {bins}"));
    }
    let (device, queue) = create_device()?;
    let (color_tex, width, height) = upload_frame(&device, &queue, frame)?;

    let region = clamp_region(region, width, height);

    let color_view = color_tex.create_view(&wgpu::TextureViewDescriptor::default());

    let params = pack_params(width, height, region, channel, bins);
    let params_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("histogram_params"),
        contents: &params,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
    });

    let bins_buf = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("histogram_bins"),
        size: (MAX_BINS as u64) * 4,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC
            | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    queue.write_buffer(&bins_buf, 0, &vec![0u8; (MAX_BINS as usize) * 4]);

    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("histogram.wgsl"),
        source: wgpu::ShaderSource::Wgsl(SHADER_SRC.into()),
    });

    let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("histogram_bgl"),
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
                    ty: wgpu::BufferBindingType::Storage { read_only: false },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
        ],
    });

    let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("histogram_pl"),
        bind_group_layouts: &[&bgl],
        immediate_size: 0,
    });
    let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some("histogram_pipeline"),
        layout: Some(&pl),
        module: &module,
        entry_point: Some("assert_main"),
        compilation_options: wgpu::PipelineCompilationOptions::default(),
        cache: None,
    });

    let bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("histogram_bg"),
        layout: &bgl,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&color_view),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: params_buf.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: bins_buf.as_entire_binding(),
            },
        ],
    });

    let staging = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("histogram_staging"),
        size: (MAX_BINS as u64) * 4,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });

    let groups_x = region.width.div_ceil(WG_X).max(1);
    let groups_y = region.height.div_ceil(WG_Y).max(1);
    let mut encoder =
        device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
    {
        let mut cpass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("histogram_pass"),
            timestamp_writes: None,
        });
        cpass.set_pipeline(&pipeline);
        cpass.set_bind_group(0, &bg, &[]);
        cpass.dispatch_workgroups(groups_x, groups_y, 1);
    }
    encoder.copy_buffer_to_buffer(&bins_buf, 0, &staging, 0, (MAX_BINS as u64) * 4);
    queue.submit(Some(encoder.finish()));

    let slice = staging.slice(..);
    let (tx, rx) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |v| {
        let _ = tx.send(v);
    });
    device
        .poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: None,
        })
        .map_err(|e| anyhow!("device poll failed: {e:?}"))?;
    rx.recv()
        .map_err(|e| anyhow!("staging channel closed: {e}"))?
        .map_err(|e| anyhow!("buffer map failed: {e:?}"))?;

    let data = slice.get_mapped_range();
    let all: Vec<u32> = bytemuck::cast_slice::<u8, u32>(&data).to_vec();
    drop(data);
    staging.unmap();

    Ok(all.into_iter().take(bins as usize).collect())
}

fn clamp_region(r: Rect, w: u32, h: u32) -> Rect {
    let x = r.x.min(w);
    let y = r.y.min(h);
    let width = r.width.min(w.saturating_sub(x));
    let height = r.height.min(h.saturating_sub(y));
    Rect { x, y, width, height }
}

fn pack_params(
    width: u32,
    height: u32,
    region: Rect,
    channel: Channel,
    bins: u32,
) -> Vec<u8> {
    let mut bytes = vec![0u8; 256];
    let fields: [u32; 8] = [
        width,
        height,
        region.x,
        region.y,
        region.width,
        region.height,
        channel.as_u32(),
        bins,
    ];
    for (i, v) in fields.iter().enumerate() {
        bytes[i * 4..i * 4 + 4].copy_from_slice(&v.to_le_bytes());
    }
    bytes
}

fn create_device() -> Result<(wgpu::Device, wgpu::Queue)> {
    let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor::default());
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        compatible_surface: None,
        force_fallback_adapter: false,
    }))
    .map_err(|e| anyhow!("no wgpu adapter available: {e}"))?;
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("histogram_device"),
        required_features: wgpu::Features::empty(),
        required_limits: wgpu::Limits::default(),
        memory_hints: wgpu::MemoryHints::default(),
        trace: wgpu::Trace::Off,
        experimental_features: wgpu::ExperimentalFeatures::disabled(),
    }))
    .map_err(|e| anyhow!("device request failed: {e}"))?;
    Ok((device, queue))
}

fn upload_frame(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    frame: &FrameSource,
) -> Result<(wgpu::Texture, u32, u32)> {
    match frame {
        FrameSource::Texture(t) => Ok((t.clone(), t.width(), t.height())),
        FrameSource::PngPath(path) => {
            let file = std::fs::File::open(path)
                .with_context(|| format!("open png {}", path.display()))?;
            let decoder = png::Decoder::new(file);
            let mut reader = decoder.read_info()?;
            let mut buf = vec![0u8; reader.output_buffer_size()];
            let info = reader.next_frame(&mut buf)?;
            let pixels = png_to_rgba8(&buf[..info.buffer_size()], info.color_type)?;
            Ok(upload_rgba8(device, queue, info.width, info.height, &pixels))
        }
        FrameSource::Rgba8 { width, height, pixels } => {
            Ok(upload_rgba8(device, queue, *width, *height, pixels))
        }
    }
}

fn png_to_rgba8(bytes: &[u8], color: png::ColorType) -> Result<Vec<u8>> {
    match color {
        png::ColorType::Rgba => Ok(bytes.to_vec()),
        png::ColorType::Rgb => {
            let mut out = Vec::with_capacity(bytes.len() / 3 * 4);
            for px in bytes.chunks_exact(3) {
                out.extend_from_slice(px);
                out.push(255);
            }
            Ok(out)
        }
        other => Err(anyhow!("unsupported PNG color type: {other:?}")),
    }
}

fn upload_rgba8(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    width: u32,
    height: u32,
    pixels: &[u8],
) -> (wgpu::Texture, u32, u32) {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("histogram_color"),
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
    queue.write_texture(
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
    (texture, width, height)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid(width: u32, height: u32, rgba: [u8; 4]) -> FrameSource {
        let mut pixels = Vec::with_capacity((width * height) as usize * 4);
        for _ in 0..(width * height) {
            pixels.extend_from_slice(&rgba);
        }
        FrameSource::Rgba8 { width, height, pixels }
    }

    fn checkerboard(width: u32, height: u32) -> FrameSource {
        let mut pixels = Vec::with_capacity((width * height) as usize * 4);
        for y in 0..height {
            for x in 0..width {
                let v = if (x + y) % 2 == 0 { 0 } else { 255 };
                pixels.extend_from_slice(&[v, v, v, 255]);
            }
        }
        FrameSource::Rgba8 { width, height, pixels }
    }

    #[test]
    fn solid_red_puts_all_weight_in_one_bin() {
        let frame = solid(8, 8, [255, 0, 0, 255]);
        let bins = histogram(&frame, Rect { x: 0, y: 0, width: 8, height: 8 }, Channel::R, 4)
            .expect("histogram");
        assert_eq!(bins.len(), 4);
        let total: u32 = bins.iter().sum();
        assert_eq!(total, 64);
        assert_eq!(bins[3], 64, "all 64 red pixels should land in the top bin");
        assert_eq!(bins[0] + bins[1] + bins[2], 0);
    }

    #[test]
    fn solid_black_lands_in_first_bin() {
        let frame = solid(4, 4, [0, 0, 0, 255]);
        let bins = histogram(&frame, Rect { x: 0, y: 0, width: 4, height: 4 }, Channel::Luma, 8)
            .expect("histogram");
        assert_eq!(bins[0], 16);
        assert_eq!(bins[1..].iter().sum::<u32>(), 0);
    }

    #[test]
    fn checkerboard_is_bimodal() {
        let frame = checkerboard(8, 8);
        let bins = histogram(&frame, Rect { x: 0, y: 0, width: 8, height: 8 }, Channel::Luma, 4)
            .expect("histogram");
        let total: u32 = bins.iter().sum();
        assert_eq!(total, 64);
        assert!(bins[0] > 0, "black squares should populate the low bin");
        assert!(bins[3] > 0, "white squares should populate the high bin");
        assert_eq!(bins[1] + bins[2], 0, "no mid-tones in a pure checkerboard");
    }

    #[test]
    fn region_clamping_limits_coverage() {
        let frame = solid(8, 8, [128, 128, 128, 255]);
        let bins = histogram(&frame, Rect { x: 2, y: 2, width: 4, height: 4 }, Channel::R, 16)
            .expect("histogram");
        let total: u32 = bins.iter().sum();
        assert_eq!(total, 16, "only the 4x4 region contributes");
    }
}
