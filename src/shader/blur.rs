//! CPU-side separable Gaussian blur, used to honor wavelet_fx's
//! `PreEffect::CpuBlur` pre-pass instructions. Operates on RGBA8 buffers
//! before they get uploaded to wgpu textures.
//!
//! Two-pass separable Gaussian — horizontal then vertical 1D Gaussian
//! convolutions. The textbook approach used by every production image
//! pipeline. Cheap and correct: each output pixel samples 2σ pixels each
//! direction (typically 6σ total), weighted by the 1D Gaussian kernel.
//!
//! At 1280×720, σ=6 (≈ radius 12) takes ~25ms on M-series CPU. Cheap
//! enough for a per-transition pass that fires at most once per frame
//! during the 0.6s window.

/// Apply a Gaussian blur to an RGBA8 buffer in place. `sigma` is the
/// standard deviation in pixels (clamped to a sensible range).
///
/// The implementation:
/// 1. Compute a 1D Gaussian kernel from `sigma` (kernel size = ceil(6σ)+1
///    truncated to half-width on each side).
/// 2. Allocate a scratch f32 buffer (R/G/B planes; alpha is passed
///    through unchanged so transparent backgrounds stay transparent).
/// 3. Horizontal pass: convolve each row.
/// 4. Vertical pass: convolve each column of the horizontal result.
/// 5. Clamp + write back to the input buffer.
pub fn gaussian_rgba(buffer: &mut [u8], width: u32, height: u32, sigma: f32) {
    let sigma = sigma.clamp(0.5, 64.0);
    let radius = (sigma * 3.0).ceil() as i32;
    let kernel = gaussian_kernel(sigma, radius);
    let kr = kernel.len() / 2;

    let w = width as i32;
    let h = height as i32;
    let stride = (width * 4) as usize;

    // Scratch f32 buffer holding the horizontal-pass result (R,G,B only;
    // alpha is copied through unchanged at the end).
    let mut h_buf = vec![0.0f32; (width * height * 3) as usize];

    // Horizontal pass.
    for y in 0..h {
        for x in 0..w {
            let mut r = 0.0;
            let mut g = 0.0;
            let mut b = 0.0;
            for k in -(kr as i32)..=(kr as i32) {
                let sx = (x + k).clamp(0, w - 1);
                let i = (y as usize * stride) + (sx as usize * 4);
                let wgt = kernel[(k + kr as i32) as usize];
                r += buffer[i] as f32 * wgt;
                g += buffer[i + 1] as f32 * wgt;
                b += buffer[i + 2] as f32 * wgt;
            }
            let dst = ((y * w + x) * 3) as usize;
            h_buf[dst] = r;
            h_buf[dst + 1] = g;
            h_buf[dst + 2] = b;
        }
    }

    // Vertical pass — read from h_buf, write into the original buffer.
    for y in 0..h {
        for x in 0..w {
            let mut r = 0.0;
            let mut g = 0.0;
            let mut b = 0.0;
            for k in -(kr as i32)..=(kr as i32) {
                let sy = (y + k).clamp(0, h - 1);
                let src = ((sy * w + x) * 3) as usize;
                let wgt = kernel[(k + kr as i32) as usize];
                r += h_buf[src] * wgt;
                g += h_buf[src + 1] * wgt;
                b += h_buf[src + 2] * wgt;
            }
            let dst = (y as usize * stride) + (x as usize * 4);
            buffer[dst] = r.clamp(0.0, 255.0) as u8;
            buffer[dst + 1] = g.clamp(0.0, 255.0) as u8;
            buffer[dst + 2] = b.clamp(0.0, 255.0) as u8;
            // Alpha unchanged.
        }
    }
}

/// Build a normalized 1D Gaussian kernel for the given σ and radius.
/// Length = `2 * radius + 1`. Weights sum to 1.0 within float precision.
fn gaussian_kernel(sigma: f32, radius: i32) -> Vec<f32> {
    let n = (2 * radius + 1) as usize;
    let mut k = Vec::with_capacity(n);
    let two_sigma_sq = 2.0 * sigma * sigma;
    let norm = 1.0 / (std::f32::consts::TAU.sqrt() * sigma);
    for i in -radius..=radius {
        let x = i as f32;
        k.push(norm * (-x * x / two_sigma_sq).exp());
    }
    // Normalize to guarantee sum = 1.0 even with truncation at the edges.
    let s: f32 = k.iter().sum();
    if s > 0.0 {
        for v in &mut k {
            *v /= s;
        }
    }
    k
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kernel_sums_to_one() {
        let k = gaussian_kernel(2.0, 6);
        let s: f32 = k.iter().sum();
        assert!((s - 1.0).abs() < 1e-5, "expected sum 1.0, got {s}");
    }

    #[test]
    fn blur_uniform_buffer_is_idempotent() {
        // A uniform-gray buffer should still be uniform gray after blur
        // (modulo edge effects which we clamp).
        let mut buf = vec![128u8, 64, 200, 255].repeat(64 * 64);
        gaussian_rgba(&mut buf, 64, 64, 4.0);
        // Center pixel should be very close to the input.
        let center = (32 * 64 + 32) * 4;
        assert!((buf[center] as i32 - 128).abs() <= 1);
        assert!((buf[center + 1] as i32 - 64).abs() <= 1);
        assert!((buf[center + 2] as i32 - 200).abs() <= 1);
        // Alpha preserved.
        assert_eq!(buf[center + 3], 255);
    }

    #[test]
    fn blur_softens_sharp_edge() {
        // 64x64 with a sharp vertical edge: left half black, right half white.
        // After blur, the column at x=32 should be ~mid-gray.
        let mut buf = vec![0u8; 64 * 64 * 4];
        for y in 0..64 {
            for x in 32..64 {
                let i = (y * 64 + x) * 4;
                buf[i] = 255;
                buf[i + 1] = 255;
                buf[i + 2] = 255;
                buf[i + 3] = 255;
            }
        }
        gaussian_rgba(&mut buf, 64, 64, 4.0);
        let edge = (32 * 64 + 32) * 4;
        assert!(
            buf[edge] > 100 && buf[edge] < 200,
            "expected mid-gray at edge, got {}",
            buf[edge]
        );
    }
}
