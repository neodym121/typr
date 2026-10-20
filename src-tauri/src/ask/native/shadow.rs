//! The shadows Slint's software renderer can't draw (it has no drop
//! shadows), made once as images: the card's as a nine-slice image that
//! stretches to any card, the dot's at the screen's scale.

use slint::{Image, Rgba8Pixel, SharedPixelBuffer};

/// One shadow, as CSS writes it: `0 <offset>px <blur>px rgba(0,0,0,<alpha>)`
struct Layer {
    offset: f32,
    blur: f32,
    alpha: f32,
}

/// The card's: `0 12px 32px rgba(0,0,0,.45), 0 2px 8px rgba(0,0,0,.3)`
const CARD: [Layer; 2] = [
    Layer {
        offset: 12.0,
        blur: 32.0,
        alpha: 0.45,
    },
    Layer {
        offset: 2.0,
        blur: 8.0,
        alpha: 0.3,
    },
];
const CARD_RADIUS: f32 = 16.0;
/// The shadow is exact for cards at least this big (an open card is 72 px
/// tall at least); smaller ones (the bar while unfolding) have none
pub const CARD_CORE: u16 = 72;

/// The dot's: `0 2px 6px rgba(0,0,0,.28)`
const DOT: Layer = Layer {
    offset: 2.0,
    blur: 6.0,
    alpha: 0.28,
};
const DOT_SIZE: f32 = 40.0;
/// How far the dot's shadow reaches past it on each side, logical pixels
pub const DOT_REACH: f32 = 12.0;

pub struct CardShadow {
    pub image: Image,
    /// How far it reaches past the card, logical pixels
    pub left: f32,
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
}

/// A blur of `blur` px reaches about 1.5 × that (three standard deviations)
fn reach(layer: &Layer) -> f32 {
    (layer.blur * 1.5).ceil()
}

/// The card's shadow at logical resolution: nine-slice edges are logical.
pub fn card() -> CardShadow {
    let side = CARD.iter().map(reach).fold(0.0, f32::max);
    let top = CARD.iter().map(|l| reach(l) - l.offset).fold(0.0, f32::max);
    let bottom = CARD.iter().map(|l| reach(l) + l.offset).fold(0.0, f32::max);
    let core = CARD_CORE as f32;
    let width = (side * 2.0 + core) as usize;
    let height = (top + core + bottom) as usize;

    let shape = |dy: f32| rounded_rect_mask(width, height, side, top + dy, core, core, CARD_RADIUS);
    let card = shape(0.0);
    let mut alpha = vec![0.0f32; width * height];
    for layer in &CARD {
        let blurred = blur(&shape(layer.offset), width, height, layer.blur / 2.0);
        for (a, b) in alpha.iter_mut().zip(blurred) {
            *a = 1.0 - (1.0 - *a) * (1.0 - layer.alpha * b);
        }
    }
    // Nothing under the card itself, as CSS draws a box shadow
    for (a, c) in alpha.iter_mut().zip(&card) {
        *a *= 1.0 - c;
    }

    let mut image = Image::from_rgba8_premultiplied(to_pixels(&alpha, width, height));
    // Cut through the middle: two pixels in each direction stretch
    let half = (CARD_CORE / 2) as u16;
    let (side, top_edge) = (side as u16, top as u16);
    image.set_nine_slice_edges(
        top_edge + half - 1,
        width as u16 - (side + half + 1),
        height as u16 - (top_edge + half + 1),
        side + half - 1,
    );
    CardShadow {
        image,
        left: side as f32,
        top,
        right: side as f32,
        bottom,
    }
}

/// The dot's shadow, `(40 + 2 × DOT_REACH)` logical pixels square, drawn at
/// the screen's `scale`.
pub fn dot(scale: f32) -> Image {
    let s = scale.max(0.5);
    let size = ((DOT_SIZE + DOT_REACH * 2.0) * s).round() as usize;
    let center = size as f32 / 2.0;
    let radius = DOT_SIZE / 2.0 * s;
    let disc = |dy: f32| circle_mask(size, center, center + dy, radius);
    let blurred = blur(&disc(DOT.offset * s), size, size, DOT.blur / 2.0 * s);
    let under = disc(0.0);
    let alpha: Vec<f32> = blurred
        .iter()
        .zip(&under)
        .map(|(b, c)| DOT.alpha * b * (1.0 - c))
        .collect();
    Image::from_rgba8_premultiplied(to_pixels(&alpha, size, size))
}

fn to_pixels(alpha: &[f32], width: usize, height: usize) -> SharedPixelBuffer<Rgba8Pixel> {
    let mut buffer = SharedPixelBuffer::<Rgba8Pixel>::new(width as u32, height as u32);
    for (pixel, a) in buffer.make_mut_slice().iter_mut().zip(alpha) {
        // Black, premultiplied: only the alpha
        *pixel = Rgba8Pixel::new(0, 0, 0, (a.clamp(0.0, 1.0) * 255.0).round() as u8);
    }
    buffer
}

/// Coverage of a rounded rectangle, anti-aliased
fn rounded_rect_mask(width: usize, height: usize, x: f32, y: f32, w: f32, h: f32, r: f32) -> Vec<f32> {
    let (cx, cy) = (x + w / 2.0, y + h / 2.0);
    let (hx, hy) = (w / 2.0 - r, h / 2.0 - r);
    let mut mask = vec![0.0; width * height];
    for py in 0..height {
        for px in 0..width {
            let qx = (px as f32 + 0.5 - cx).abs() - hx;
            let qy = (py as f32 + 0.5 - cy).abs() - hy;
            let outside = qx.max(0.0).hypot(qy.max(0.0));
            let sdf = outside + qx.max(qy).min(0.0) - r;
            mask[py * width + px] = (0.5 - sdf).clamp(0.0, 1.0);
        }
    }
    mask
}

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
    fn test_card_shadow_is_soft_and_absent_under_the_card() {
        let shadow = card();
        let size = shadow.image.size();
        assert_eq!(size.width as f32, shadow.left + CARD_CORE as f32 + shadow.right);
        assert_eq!(size.height as f32, shadow.top + CARD_CORE as f32 + shadow.bottom);
        // Deeper below the card than above it
        assert!(shadow.bottom > shadow.top);
    }

    #[test]
    fn test_blur_keeps_the_total() {
        let mut source = vec![0.0; 21 * 21];
        source[10 * 21 + 10] = 1.0;
        let blurred = blur(&source, 21, 21, 2.0);
        let total: f32 = blurred.iter().sum();
        assert!((total - 1.0).abs() < 0.01);
    }
}
