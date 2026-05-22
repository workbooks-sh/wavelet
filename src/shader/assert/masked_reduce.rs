//! Region-masked scalar reduce. Given a metric texture and a Cryptomatte-
//! style ID buffer, accumulate `op` over the pixels whose ID matches
//! `target_id` and return the finalized scalar. Foundation for queries
//! like "mean edge magnitude inside the logo region" — compose with
//! `sobel` at the call site.
//!
//! ID buffer shape: `r32uint` `wgpu::Texture` (per ABI binding 1). Pass it
//! in via `FrameSource::Texture(...)`. The `id_texture_from_u32` helper
//! builds one from a flat `&[u32]` for tests and call sites that aren't
//! sourcing the buffer directly from a render pass.

use anyhow::{anyhow, Result};
use bytemuck::{Pod, Zeroable};
use wgpu::util::DeviceExt;

use super::sobel::{create_device, load_color_texture};
use super::types::FrameSource;

const MASKED_REDUCE_WGSL: &str = include_str!("masked_reduce.wgsl");
const SCALE: f32 = 1024.0;

/// Reduction kind. Mean = sum / count over the masked region (per-pixel
/// R-channel value clamped to [0,1]). Sum = raw sum without dividing.
/// Local enum scoped to this module — the broader `ReduceOp` lives with
/// the parallel reduce primitive (wb-mxrk.2); we'll dedupe when both
/// land.
#[derive(Clone, Copy, Debug)]
pub enum MaskedReduceOp {
    /// Mean of the R channel over matched pixels. Returns 0.0 if no match.
    Mean,
    /// Sum of the R channel over matched pixels.
    Sum,
}

/// Build an `r32uint` `wgpu::Texture` from a flat row-major `u32` buffer.
/// Wrap in `FrameSource::Texture` to feed `masked_reduce` as the ID buffer.
pub fn id_texture_from_u32(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    width: u32,
    height: u32,
    data: &[u32],
) -> wgpu::Texture {
    assert_eq!(data.len(), (width * height) as usize, "id buffer size mismatch");
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("masked_reduce_ids"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::R32Uint,
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
        bytemuck::cast_slice(data),
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

/// Reduce the R channel of `frame` over pixels whose `id_buf` value equals
/// `target_id`. Returns the finalized scalar per `op`. The id_buf must be
/// a `FrameSource::Texture` carrying an `r32uint` texture matching frame
/// dimensions; other variants return an error.
pub fn masked_reduce(
    frame: &FrameSource,
    id_buf: &FrameSource,
    target_id: u32,
    op: MaskedReduceOp,
) -> Result<f32> {
    let (device, queue) = create_device()?;
    masked_reduce_with_device(&device, &queue, frame, id_buf, target_id, op)
}

/// Same as `masked_reduce` but reuses a caller-provided device/queue so
/// callers running multiple primitives back-to-back can share a single
/// adapter and avoid the per-call adapter handshake.
pub fn masked_reduce_with_device(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    frame: &FrameSource,
    id_buf: &FrameSource,
    target_id: u32,
    op: MaskedReduceOp,
) -> Result<f32> {
    let (color_texture, width, height) = load_color_texture(device, queue, frame)?;
    let id_texture = match id_buf {
        FrameSource::Texture(t) => {
            if t.format() != wgpu::TextureFormat::R32Uint {
                return Err(anyhow!(
                    "id_buf must be R32Uint, got {:?}",
                    t.format()
                ));
            }
            if t.width() != width || t.height() != height {
                return Err(anyhow!(
                    "id_buf dims {}x{} do not match frame {}x{}",
                    t.width(),
                    t.height(),
                    width,
                    height
                ));
            }
            t.clone()
        }
        _ => {
            return Err(anyhow!(
                "id_buf must be FrameSource::Texture(R32Uint); build one with id_texture_from_u32"
            ));
        }
    };

    let color_view = color_texture.create_view(&wgpu::TextureViewDescriptor::default());
    let id_view = id_texture.create_view(&wgpu::TextureViewDescriptor::default());

    let op_code: u32 = match op {
        MaskedReduceOp::Mean => 0,
        MaskedReduceOp::Sum => 1,
    };
    let params = ParamsBuf {
        width,
        height,
        target_id,
        op: op_code,
    };
    let params_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("masked_reduce_params"),
        contents: bytemuck::bytes_of(&params),
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
    });

    let acc_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("masked_reduce_acc"),
        contents: bytemuck::bytes_of(&AccBuf { sum: 0, count: 0 }),
        usage: wgpu::BufferUsages::STORAGE
            | wgpu::BufferUsages::COPY_SRC
            | wgpu::BufferUsages::COPY_DST,
    });

    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("masked_reduce"),
        source: wgpu::ShaderSource::Wgsl(MASKED_REDUCE_WGSL.into()),
    });

    let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("masked_reduce_bgl"),
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
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Uint,
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 2,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
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
        label: Some("masked_reduce_pl"),
        bind_group_layouts: &[&layout],
        immediate_size: 0,
    });
    let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some("masked_reduce_pipeline"),
        layout: Some(&pipeline_layout),
        module: &module,
        entry_point: Some("masked_reduce_main"),
        compilation_options: wgpu::PipelineCompilationOptions::default(),
        cache: None,
    });

    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("masked_reduce_bg"),
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
                resource: params_buf.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 3,
                resource: acc_buf.as_entire_binding(),
            },
        ],
    });

    let staging = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("masked_reduce_staging"),
        size: std::mem::size_of::<AccBuf>() as u64,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });

    let groups_x = width.div_ceil(8).max(1);
    let groups_y = height.div_ceil(8).max(1);

    let mut encoder =
        device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
    {
        let mut cpass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("masked_reduce_pass"),
            timestamp_writes: None,
        });
        cpass.set_pipeline(&pipeline);
        cpass.set_bind_group(0, &bind_group, &[]);
        cpass.dispatch_workgroups(groups_x, groups_y, 1);
    }
    encoder.copy_buffer_to_buffer(
        &acc_buf,
        0,
        &staging,
        0,
        std::mem::size_of::<AccBuf>() as u64,
    );
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
    let acc: AccBuf = *bytemuck::from_bytes(&data);
    drop(data);
    staging.unmap();

    let sum = acc.sum as f32 / SCALE;
    match op {
        MaskedReduceOp::Sum => Ok(sum),
        MaskedReduceOp::Mean => {
            if acc.count == 0 {
                Ok(0.0)
            } else {
                Ok(sum / acc.count as f32)
            }
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct ParamsBuf {
    width: u32,
    height: u32,
    target_id: u32,
    op: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct AccBuf {
    sum: u32,
    count: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn three_region_means() {
        let (device, queue) = create_device().expect("device");
        let w: u32 = 12;
        let h: u32 = 4;
        // Three vertical 4x4 bands: red R=0.25, green R=0.50, blue R=0.75
        // (R-channel is what masked_reduce sums; G/B are decorative).
        let mut pixels = Vec::with_capacity((w * h * 4) as usize);
        for _y in 0..h {
            for x in 0..w {
                let r: u8 = if x < 4 {
                    (0.25 * 255.0) as u8
                } else if x < 8 {
                    (0.50 * 255.0) as u8
                } else {
                    (0.75 * 255.0) as u8
                };
                pixels.extend_from_slice(&[r, 0, 0, 255]);
            }
        }
        let mut ids = Vec::with_capacity((w * h) as usize);
        for _y in 0..h {
            for x in 0..w {
                let id: u32 = if x < 4 {
                    1
                } else if x < 8 {
                    2
                } else {
                    3
                };
                ids.push(id);
            }
        }

        let id_tex = id_texture_from_u32(&device, &queue, w, h, &ids);
        let frame = FrameSource::Rgba8 {
            width: w,
            height: h,
            pixels,
        };
        let id_frame = FrameSource::Texture(id_tex);

        let m1 = masked_reduce_with_device(&device, &queue, &frame, &id_frame, 1, MaskedReduceOp::Mean)
            .expect("mean 1");
        let m2 = masked_reduce_with_device(&device, &queue, &frame, &id_frame, 2, MaskedReduceOp::Mean)
            .expect("mean 2");
        let m3 = masked_reduce_with_device(&device, &queue, &frame, &id_frame, 3, MaskedReduceOp::Mean)
            .expect("mean 3");

        // 8-bit quantization + 10-bit fixed-point scaling → ~0.003 tolerance.
        let tol = 0.01;
        assert!((m1 - 0.25).abs() < tol, "id=1 mean: {m1}");
        assert!((m2 - 0.50).abs() < tol, "id=2 mean: {m2}");
        assert!((m3 - 0.75).abs() < tol, "id=3 mean: {m3}");

        let s2 = masked_reduce_with_device(&device, &queue, &frame, &id_frame, 2, MaskedReduceOp::Sum)
            .expect("sum 2");
        // 16 pixels at ~0.5 → 8.0.
        assert!((s2 - 8.0).abs() < 0.2, "id=2 sum: {s2}");

        let m_missing = masked_reduce_with_device(
            &device,
            &queue,
            &frame,
            &id_frame,
            99,
            MaskedReduceOp::Mean,
        )
        .expect("mean missing");
        assert_eq!(m_missing, 0.0);
    }

    #[test]
    fn sobel_composed_via_id_buffer() {
        use super::super::sobel::sobel_with_device;

        let (device, queue) = create_device().expect("device");
        let w: u32 = 16;
        let h: u32 = 16;
        // Left half (id=1) is flat grey, right half (id=2) has a sharp
        // vertical edge at x=12. Expect mean-edge-magnitude(id=1) ≪
        // mean-edge-magnitude(id=2).
        let mut pixels = Vec::with_capacity((w * h * 4) as usize);
        for _y in 0..h {
            for x in 0..w {
                let v: u8 = if x < 8 {
                    128
                } else if x < 12 {
                    0
                } else {
                    255
                };
                pixels.extend_from_slice(&[v, v, v, 255]);
            }
        }
        let frame = FrameSource::Rgba8 {
            width: w,
            height: h,
            pixels,
        };
        let mut ids = Vec::with_capacity((w * h) as usize);
        for _y in 0..h {
            for x in 0..w {
                ids.push(if x < 8 { 1u32 } else { 2u32 });
            }
        }
        let id_tex = id_texture_from_u32(&device, &queue, w, h, &ids);

        let sobel_out = sobel_with_device(&device, &queue, &frame).expect("sobel");
        let mag_frame = FrameSource::Texture(sobel_out.texture);
        let id_frame = FrameSource::Texture(id_tex);

        let left = masked_reduce_with_device(
            &device,
            &queue,
            &mag_frame,
            &id_frame,
            1,
            MaskedReduceOp::Mean,
        )
        .expect("left");
        let right = masked_reduce_with_device(
            &device,
            &queue,
            &mag_frame,
            &id_frame,
            2,
            MaskedReduceOp::Mean,
        )
        .expect("right");

        // The left region (id=1) ends at x=7; its column adjacent to the
        // x=8 transition picks up some Sobel response. Right region (id=2)
        // contains the 0→255 edge at x=12 and is much hotter overall.
        assert!(right > left * 3.0, "right (edge region) {right} not >> left {left}");
    }
}
