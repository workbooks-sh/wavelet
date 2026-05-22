//! Windowed mean + variance via separable Gaussian convolution (wb-mxrk.3).
//!
//! Two-pass: horizontal blur of luma + luma² into temp buffers, then vertical
//! blur into the final mean / variance buffers. `variance = E[L²] - E[L]²`.
//!
//! Output shape: flat `Vec<f32>` of `width * height` per buffer, row-major.
//! Returns CPU side rather than GPU textures because (1) the ticket API
//! signature says so and (2) tests need direct readback; SSIM and downstream
//! validators that want the GPU side can switch to a texture-returning
//! sibling later.
//!
//! Kernel form follows the SSIM paper (Wang et al., 2004):
//!   https://ece.uwaterloo.ca/~z70wang/publications/ssim.pdf
//! Defaults: 11×11 window, sigma = 1.5 — matches the canonical SSIM kernel.

use anyhow::{anyhow, Context, Result};
use wgpu::util::DeviceExt;

use super::types::FrameSource;

const SHADER_SRC: &str = include_str!("windowed_stats.wgsl");
const WG: u32 = 8;
const MAX_WINDOW: u32 = 64;

/// Canonical SSIM window: 11×11 Gaussian.
pub const DEFAULT_WINDOW: u32 = 11;
/// Canonical SSIM sigma matching `DEFAULT_WINDOW`.
pub const DEFAULT_SIGMA: f32 = 1.5;

/// Run a separable Gaussian-weighted local mean + variance over the frame's
/// luma channel. Returns `(mean, variance)`, each a row-major `Vec<f32>` of
/// length `width * height`. Edge handling: clamp-to-edge.
///
/// `window_size` must be odd and in `1..=64`. `sigma` must be > 0.
pub fn windowed_stats(
    frame: &FrameSource,
    window_size: u32,
    sigma: f32,
) -> Result<(Vec<f32>, Vec<f32>)> {
    if window_size == 0 || window_size > MAX_WINDOW {
        return Err(anyhow!(
            "window_size must be 1..=64, got {window_size}"
        ));
    }
    if window_size % 2 == 0 {
        return Err(anyhow!("window_size must be odd, got {window_size}"));
    }
    if !(sigma > 0.0) {
        return Err(anyhow!("sigma must be > 0, got {sigma}"));
    }

    let (device, queue) = create_device()?;
    let (color_tex, width, height) = upload_frame(&device, &queue, frame)?;
    let pixel_count = (width as usize) * (height as usize);
    let buf_bytes = (pixel_count * 4) as u64;

    let color_view = color_tex.create_view(&wgpu::TextureViewDescriptor::default());

    let params = pack_params(width, height, window_size);
    let params_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("ws_params"),
        contents: &params,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
    });

    let kernel = pack_kernel(window_size, sigma);
    let kernel_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("ws_kernel"),
        contents: &kernel,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
    });

    let tmp_mean = make_storage(&device, "ws_tmp_mean", buf_bytes);
    let tmp_m2 = make_storage(&device, "ws_tmp_m2", buf_bytes);
    let out_mean = make_storage(&device, "ws_out_mean", buf_bytes);
    let out_var = make_storage(&device, "ws_out_var", buf_bytes);

    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("windowed_stats.wgsl"),
        source: wgpu::ShaderSource::Wgsl(SHADER_SRC.into()),
    });

    let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("ws_bgl"),
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
            uniform_entry(1),
            uniform_entry(2),
            storage_entry(3),
            storage_entry(4),
            storage_entry(5),
            storage_entry(6),
        ],
    });

    let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("ws_pl"),
        bind_group_layouts: &[&bgl],
        immediate_size: 0,
    });
    let pipeline_h = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some("ws_pipeline_h"),
        layout: Some(&pl),
        module: &module,
        entry_point: Some("pass_h"),
        compilation_options: wgpu::PipelineCompilationOptions::default(),
        cache: None,
    });
    let pipeline_v = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some("ws_pipeline_v"),
        layout: Some(&pl),
        module: &module,
        entry_point: Some("pass_v"),
        compilation_options: wgpu::PipelineCompilationOptions::default(),
        cache: None,
    });

    let bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("ws_bg"),
        layout: &bgl,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&color_view),
            },
            wgpu::BindGroupEntry { binding: 1, resource: params_buf.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 2, resource: kernel_buf.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 3, resource: tmp_mean.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 4, resource: tmp_m2.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 5, resource: out_mean.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 6, resource: out_var.as_entire_binding() },
        ],
    });

    let stage_mean = make_staging(&device, "ws_stage_mean", buf_bytes);
    let stage_var = make_staging(&device, "ws_stage_var", buf_bytes);

    let groups_x = width.div_ceil(WG).max(1);
    let groups_y = height.div_ceil(WG).max(1);

    let mut encoder =
        device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
    {
        let mut cpass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("ws_pass_h"),
            timestamp_writes: None,
        });
        cpass.set_pipeline(&pipeline_h);
        cpass.set_bind_group(0, &bg, &[]);
        cpass.dispatch_workgroups(groups_x, groups_y, 1);
    }
    {
        let mut cpass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("ws_pass_v"),
            timestamp_writes: None,
        });
        cpass.set_pipeline(&pipeline_v);
        cpass.set_bind_group(0, &bg, &[]);
        cpass.dispatch_workgroups(groups_x, groups_y, 1);
    }
    encoder.copy_buffer_to_buffer(&out_mean, 0, &stage_mean, 0, buf_bytes);
    encoder.copy_buffer_to_buffer(&out_var, 0, &stage_var, 0, buf_bytes);
    queue.submit(Some(encoder.finish()));

    let mean = map_and_read(&device, &stage_mean)?;
    let variance = map_and_read(&device, &stage_var)?;
    Ok((mean, variance))
}

fn pack_params(width: u32, height: u32, window_size: u32) -> Vec<u8> {
    let mut bytes = vec![0u8; 256];
    let fields: [u32; 4] = [width, height, window_size, 0];
    for (i, v) in fields.iter().enumerate() {
        bytes[i * 4..i * 4 + 4].copy_from_slice(&v.to_le_bytes());
    }
    bytes
}

fn pack_kernel(window_size: u32, sigma: f32) -> Vec<u8> {
    let n = window_size as usize;
    let half = (n as f32 - 1.0) * 0.5;
    let mut w = vec![0f32; 64];
    let two_sigma_sq = 2.0 * sigma * sigma;
    let mut sum = 0.0f32;
    for i in 0..n {
        let d = i as f32 - half;
        let v = (-(d * d) / two_sigma_sq).exp();
        w[i] = v;
        sum += v;
    }
    for v in w.iter_mut().take(n) {
        *v /= sum;
    }
    let mut bytes = vec![0u8; 64 * 4];
    for (i, v) in w.iter().enumerate() {
        bytes[i * 4..i * 4 + 4].copy_from_slice(&v.to_le_bytes());
    }
    bytes
}

fn uniform_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

fn storage_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only: false },
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

fn make_storage(device: &wgpu::Device, label: &'static str, bytes: u64) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label),
        size: bytes,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC
            | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

fn make_staging(device: &wgpu::Device, label: &'static str, bytes: u64) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label),
        size: bytes,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    })
}

fn map_and_read(device: &wgpu::Device, staging: &wgpu::Buffer) -> Result<Vec<f32>> {
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
    let out = bytemuck::cast_slice::<u8, f32>(&data).to_vec();
    drop(data);
    staging.unmap();
    Ok(out)
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
        label: Some("ws_device"),
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
        label: Some("ws_color"),
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
    fn solid_color_has_zero_variance_everywhere() {
        let frame = solid(16, 16, [128, 128, 128, 255]);
        let (mean, var) =
            windowed_stats(&frame, DEFAULT_WINDOW, DEFAULT_SIGMA).expect("windowed_stats");
        assert_eq!(mean.len(), 16 * 16);
        assert_eq!(var.len(), 16 * 16);
        let expected_mean = 128.0 / 255.0;
        for (i, m) in mean.iter().enumerate() {
            assert!(
                (m - expected_mean).abs() < 1e-4,
                "mean[{i}] = {m}, expected ~{expected_mean}"
            );
        }
        for (i, v) in var.iter().enumerate() {
            assert!(v.abs() < 1e-5, "var[{i}] = {v}, expected ~0");
        }
    }

    #[test]
    fn checkerboard_has_nonzero_variance() {
        let frame = checkerboard(16, 16);
        let (_mean, var) = windowed_stats(&frame, 11, 1.5).expect("windowed_stats");
        let cx = 8usize;
        let cy = 8usize;
        let center = var[cy * 16 + cx];
        assert!(
            center > 1e-3,
            "center variance should be substantial for a checkerboard, got {center}"
        );
        let any_nonzero = var.iter().any(|v| *v > 1e-3);
        assert!(any_nonzero, "expected some non-zero variance somewhere");
    }

    #[test]
    fn kernel_normalizes_to_one() {
        let bytes = pack_kernel(11, 1.5);
        let weights: Vec<f32> = bytes
            .chunks_exact(4)
            .take(11)
            .map(|c| f32::from_le_bytes(c.try_into().unwrap()))
            .collect();
        let sum: f32 = weights.iter().sum();
        assert!((sum - 1.0).abs() < 1e-5, "kernel must sum to 1, got {sum}");
    }
}
