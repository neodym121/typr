//! The dot's shadow, which Slint's software renderer can't draw (it has no
//! drop shadows): made as an image at the screen's scale, so the window's
//! dot looks like the recording indicator it takes over from. The card has
//! no shadow.

use slint::{Image, Rgba8Pixel, SharedPixelBuffer};

/// The dot's shadow, as CSS writes it: `0 2px 6px rgba(0,0,0,.28)`
const OFFSET: f32 = 2.0;
const BLUR: f32 = 6.0;
const ALPHA: f32 = 0.28;
const DOT_SIZE: f32 = 40.0;
/// How far the dot's shadow reaches past it on each side, logical pixels
pub const DOT_REACH: f32 = 12.0;

/// The dot's shadow, `(40 + 2 × DOT_REACH)` logical pixels square, drawn at
/// the screen's `scale`.
pub fn dot(scale: f32) -> Image {
    let s = scale.max(0.5);
    let size = ((DOT_SIZE + DOT_REACH * 2.0) * s).round() as usize;
    let center = size as f32 / 2.0;
    let radius = DOT_SIZE / 2.0 * s;
    let disc = |dy: f32| circle_mask(size, center, center + dy, radius);
    let blurred = blur(&disc(OFFSET * s), size, size, BLUR / 2.0 * s);
    // Nothing under the dot itself, as CSS draws a box shadow
    let under = disc(0.0);
    let alpha: Vec<f32> = blurred.iter().zip(&under).map(|(b, c)| ALPHA * b * (1.0 - c)).collect();

    let mut buffer = SharedPixelBuffer::<Rgba8Pixel>::new(size as u32, size as u32);
    for (pixel, a) in buffer.make_mut_slice().iter_mut().zip(&alpha) {
        // Black, premultiplied: only the alpha
        *pixel = Rgba8Pixel::new(0, 0, 0, (a.clamp(0.0, 1.0) * 255.0).round() as u8);
    }
    Image::from_rgba8_premultiplied(buffer)
}

/// Coverage of a circle, anti-aliased
fn circle_mask(size: usize, cx: f32, cy: f32, r: f32) -> Vec<f32> {
    let mut mask = vec![0.0; size * size];
    for py in 0..size {
        for px in 0..size {
            let d = (px as f32 + 0.5 - cx).hypot(py as f32 + 0.5 - cy) - r;
            mask[py * size + px] = (0.5 - d).clamp(0.0, 1.0);
        }
    }
    mask
}

/// Gaussian blur with standard deviation `sigma`, in two passes
fn blur(source: &[f32], width: usize, height: usize, sigma: f32) -> Vec<f32> {
    let radius = (sigma * 3.0).ceil() as isize;
    let kernel: Vec<f32> = {
        let raw: Vec<f32> = (-radius..=radius)
            .map(|i| (-(i * i) as f32 / (2.0 * sigma * sigma)).exp())
            .collect();
        let sum: f32 = raw.iter().sum();
        raw.into_iter().map(|k| k / sum).collect()
    };
    let pass = |input: &[f32], horizontal: bool| -> Vec<f32> {
        let mut output = vec![0.0; width * height];
        for y in 0..height as isize {
            for x in 0..width as isize {
                let mut sum = 0.0;
                for (k, weight) in kernel.iter().enumerate() {
                    let offset = k as isize - radius;
                    let (sx, sy) = if horizontal { (x + offset, y) } else { (x, y + offset) };
                    if sx >= 0 && sy >= 0 && (sx as usize) < width && (sy as usize) < height {
                        sum += weight * input[sy as usize * width + sx as usize];
                    }
                }
                output[y as usize * width + x as usize] = sum;
            }
        }
        output
    };
    pass(&pass(source, true), false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_blur_keeps_the_total() {
        let mut source = vec![0.0; 21 * 21];
        source[10 * 21 + 10] = 1.0;
        let blurred = blur(&source, 21, 21, 2.0);
        let total: f32 = blurred.iter().sum();
        assert!((total - 1.0).abs() < 0.01);
    }

    #[test]
    fn test_dot_shadow_has_the_dot_s_size_and_reach() {
        let image = dot(1.25);
        let side = ((40.0 + DOT_REACH * 2.0) * 1.25_f32).round() as u32;
        assert_eq!((image.size().width, image.size().height), (side, side));
    }
}
