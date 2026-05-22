//! Fused 3x3 Sobel edge-magnitude pass. Output is a `wgpu::Texture` in
//! `Rgba8Unorm` with magnitude broadcast across R/G/B. Magnitude only —
//! direction is intentionally out of scope; downstream consumers
//! (edge-density via `masked_reduce`, contrast checks) only need scalar
//! magnitude, and the extra channel doubles bandwidth without a caller.

use anyhow::{anyhow, Result};

use super::types::FrameSource;

const SOBEL_WGSL: &str = include_str!("sobel.wgsl");

/// Result of the Sobel pass. The magnitude texture is Rgba8Unorm with the
/// scalar magnitude replicated across R, G, B (A=1) so it can be fed back
/// in as a `FrameSource::Texture` to any binding-0 consumer.
pub struct SobelOutput {
    /// Magnitude texture, Rgba8Unorm, scalar broadcast across RGB.
    pub texture: wgpu::Texture,
    /// Frame width in pixels.
    pub width: u32,
    /// Frame height in pixels.
    pub height: u32,
}

/// Run a Sobel edge-magnitude pass over `frame`. Returns the magnitude
/// texture; compose with `masked_reduce` for region edge density.
pub fn sobel(frame: &FrameSource) -> Result<SobelOutput> {
    let (device, queue) = create_device()?;
    sobel_with_device(&device, &queue, frame)
}

pub(crate) fn sobel_with_device(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    frame: &FrameSource,
) -> Result<SobelOutput> {
    let (src_texture, width, height) = load_color_texture(device, queue, frame)?;

    let dst_texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("sobel_dst"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::STORAGE_BINDING
            | wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });

    let src_view = src_texture.create_view(&wgpu::TextureViewDescriptor::default());
    let dst_view = dst_texture.create_view(&wgpu::TextureViewDescriptor::default());

    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("sobel"),
        source: wgpu::ShaderSource::Wgsl(SOBEL_WGSL.into()),
    });

    let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("sobel_bgl"),
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
                ty: wgpu::BindingType::StorageTexture {
                    access: wgpu::StorageTextureAccess::WriteOnly,
                    format: wgpu::TextureFormat::Rgba8Unorm,
                    view_dimension: wgpu::TextureViewDimension::D2,
                },
                count: None,
            },
        ],
    });

    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("sobel_pl"),
        bind_group_layouts: &[&layout],
        immediate_size: 0,
    });
    let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some("sobel_pipeline"),
        layout: Some(&pipeline_layout),
        module: &module,
        entry_point: Some("sobel_main"),
        compilation_options: wgpu::PipelineCompilationOptions::default(),
        cache: None,
    });

    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("sobel_bg"),
        layout: &layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&src_view),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::TextureView(&dst_view),
            },
        ],
    });

    let groups_x = width.div_ceil(8).max(1);
    let groups_y = height.div_ceil(8).max(1);

    let mut encoder =
        device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
    {
        let mut cpass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("sobel_pass"),
            timestamp_writes: None,
        });
        cpass.set_pipeline(&pipeline);
        cpass.set_bind_group(0, &bind_group, &[]);
        cpass.dispatch_workgroups(groups_x, groups_y, 1);
    }
    queue.submit(Some(encoder.finish()));

    Ok(SobelOutput {
        texture: dst_texture,
        width,
        height,
    })
}

pub(crate) fn create_device() -> Result<(wgpu::Device, wgpu::Queue)> {
    let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor::default());
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        compatible_surface: None,
        force_fallback_adapter: false,
    }))
    .map_err(|e| anyhow!("no wgpu adapter available: {e}"))?;
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("gamut_sobel_device"),
        required_features: wgpu::Features::empty(),
        required_limits: wgpu::Limits::default(),
        memory_hints: wgpu::MemoryHints::default(),
        trace: wgpu::Trace::Off,
        experimental_features: wgpu::ExperimentalFeatures::disabled(),
    }))
    .map_err(|e| anyhow!("device request failed: {e}"))?;
    Ok((device, queue))
}

pub(crate) fn load_color_texture(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    frame: &FrameSource,
) -> Result<(wgpu::Texture, u32, u32)> {
    match frame {
        FrameSource::Texture(t) => Ok((clone_texture_handle(t), t.width(), t.height())),
        FrameSource::PngPath(path) => {
            let file = std::fs::File::open(path)
                .map_err(|e| anyhow!("open png {}: {e}", path.display()))?;
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

fn clone_texture_handle(t: &wgpu::Texture) -> wgpu::Texture {
    t.clone()
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

pub(crate) fn upload_rgba8(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    width: u32,
    height: u32,
    pixels: &[u8],
) -> (wgpu::Texture, u32, u32) {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("sobel_src"),
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

    fn readback_magnitude(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        out: &SobelOutput,
    ) -> Vec<u8> {
        let bytes_per_row = (out.width * 4).next_multiple_of(256);
        let buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("sobel_readback"),
            size: (bytes_per_row * out.height) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder =
            device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &out.texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &buf,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(bytes_per_row),
                    rows_per_image: Some(out.height),
                },
            },
            wgpu::Extent3d {
                width: out.width,
                height: out.height,
                depth_or_array_layers: 1,
            },
        );
        queue.submit(Some(encoder.finish()));
        let slice = buf.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |v| {
            let _ = tx.send(v);
        });
        device
            .poll(wgpu::PollType::Wait {
                submission_index: None,
                timeout: None,
            })
            .unwrap();
        rx.recv().unwrap().unwrap();
        let data = slice.get_mapped_range();
        let mut out_vec = Vec::with_capacity((out.width * out.height) as usize);
        for row in 0..out.height {
            let start = (row * bytes_per_row) as usize;
            for x in 0..out.width {
                out_vec.push(data[start + (x * 4) as usize]);
            }
        }
        drop(data);
        buf.unmap();
        out_vec
    }

    #[test]
    fn solid_color_zero_magnitude() {
        let (device, queue) = create_device().expect("device");
        let frame = FrameSource::Rgba8 {
            width: 16,
            height: 16,
            pixels: vec![128u8; 16 * 16 * 4]
                .chunks_exact(4)
                .flat_map(|_| [128u8, 128, 128, 255])
                .collect(),
        };
        let out = sobel_with_device(&device, &queue, &frame).expect("sobel");
        let mags = readback_magnitude(&device, &queue, &out);
        for (i, m) in mags.iter().enumerate() {
            assert!(*m <= 1, "expected ~0 magnitude at idx {i}, got {m}");
        }
    }

    #[test]
    fn sharp_edge_nonzero_magnitude() {
        let (device, queue) = create_device().expect("device");
        let mut pixels = Vec::with_capacity(16 * 16 * 4);
        for _y in 0..16u32 {
            for x in 0..16u32 {
                let v: u8 = if x < 8 { 0 } else { 255 };
                pixels.extend_from_slice(&[v, v, v, 255]);
            }
        }
        let frame = FrameSource::Rgba8 {
            width: 16,
            height: 16,
            pixels,
        };
        let out = sobel_with_device(&device, &queue, &frame).expect("sobel");
        let mags = readback_magnitude(&device, &queue, &out);
        let w = out.width as usize;
        for y in 1..(out.height as usize - 1) {
            let edge_left = mags[y * w + 7];
            let edge_right = mags[y * w + 8];
            assert!(
                edge_left > 100 || edge_right > 100,
                "edge magnitude weak at row {y}: left={edge_left} right={edge_right}",
            );
        }
        let center_left = mags[8 * w + 2];
        let center_right = mags[8 * w + 13];
        assert!(center_left < 10, "far-left should be flat, got {center_left}");
        assert!(center_right < 10, "far-right should be flat, got {center_right}");
    }
}
