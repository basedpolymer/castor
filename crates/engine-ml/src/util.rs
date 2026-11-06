//! Small image helpers shared by the tasks.

use fast_image_resize as fir;
use image::{GrayImage, RgbImage, RgbaImage};

use crate::backend::Tensor;

/// Composite RGBA over a solid colour.
pub fn rgba_to_rgb_over(img: &RgbaImage, bg: [u8; 3]) -> RgbImage {
    let mut out = RgbImage::new(img.width(), img.height());
    for (o, p) in out.pixels_mut().zip(img.pixels()) {
        let a = p[3] as u32;
        for c in 0..3 {
            o[c] = ((p[c] as u32 * a + bg[c] as u32 * (255 - a) + 127) / 255) as u8;
        }
    }
    out
}

fn resize_any<I: fir::IntoImageView, O: fir::IntoImageViewMut>(src: &I, dst: &mut O, alg: fir::ResizeAlg) {
    let mut r = fir::Resizer::new();
    // Both buffers are created by us with matching pixel types: this cannot fail.
    if let Err(e) = r.resize(src, dst, &fir::ResizeOptions::new().resize_alg(alg)) {
        tracing::error!("resize failed: {e}");
    }
}

/// High-quality resize (Lanczos-3 down, Catmull-Rom up).
pub fn resize_rgb(img: &RgbImage, w: u32, h: u32) -> RgbImage {
    if img.dimensions() == (w, h) {
        return img.clone();
    }
    let up = w > img.width() || h > img.height();
    let f = if up { fir::FilterType::CatmullRom } else { fir::FilterType::Lanczos3 };
    let mut dst = RgbImage::new(w, h);
    resize_any(img, &mut dst, fir::ResizeAlg::Convolution(f));
    dst
}

pub fn resize_gray(img: &GrayImage, w: u32, h: u32, filter: fir::FilterType) -> GrayImage {
    if img.dimensions() == (w, h) {
        return img.clone();
    }
    let mut dst = GrayImage::new(w, h);
    resize_any(img, &mut dst, fir::ResizeAlg::Convolution(filter));
    dst
}

/// RGB → NCHW f32 tensor: (v * scale - mean) / std per channel; `bgr` swaps channels.
pub fn to_nchw(img: &RgbImage, mean: [f32; 3], std: [f32; 3], scale: f32, bgr: bool) -> Tensor {
    let (w, h) = img.dimensions();
    let plane = (w * h) as usize;
    let mut data = vec![0f32; 3 * plane];
    for (i, p) in img.pixels().enumerate() {
        for c in 0..3 {
            let src = if bgr { 2 - c } else { c };
            data[c * plane + i] = (p[src] as f32 * scale - mean[c]) / std[c];
        }
    }
    Tensor::new(vec![1, 3, h as usize, w as usize], data)
}

/// Mirror an out-of-range coordinate back into 0..n (no edge repeat).
pub fn reflect(i: i64, n: u32) -> u32 {
    let n = n as i64;
    if n == 1 {
        return 0;
    }
    let period = 2 * (n - 1);
    let mut m = i.rem_euclid(period);
    if m >= n {
        m = period - m;
    }
    m as u32
}

fn area(b: &[f32; 4]) -> f32 {
    (b[2] - b[0]).max(0.0) * (b[3] - b[1]).max(0.0)
}

/// Non-maximum suppression. Also drops a box that lies mostly inside a better one
/// (the same face seen at two scales).
pub fn nms(mut dets: Vec<([f32; 4], f32)>, iou: f32) -> Vec<[f32; 4]> {
    dets.sort_by(|a, b| b.1.total_cmp(&a.1));
    let mut keep: Vec<[f32; 4]> = Vec::new();
    for (b, _) in dets {
        let dup = keep.iter().any(|k| {
            let ix = (b[2].min(k[2]) - b[0].max(k[0])).max(0.0);
            let iy = (b[3].min(k[3]) - b[1].max(k[1])).max(0.0);
            let inter = ix * iy;
            let union = area(&b) + area(k) - inter;
            inter / union.max(1e-6) > iou || inter / area(&b).min(area(k)).max(1e-6) > 0.6
        });
        if !dup {
            keep.push(b);
        }
    }
    keep
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reflect_works() {
        assert_eq!(reflect(-1, 5), 1);
        assert_eq!(reflect(5, 5), 3);
        assert_eq!(reflect(2, 5), 2);
        assert_eq!(reflect(-3, 1), 0);
    }
    #[test]
    fn nms_merges() {
        let r = nms(vec![([0., 0., 10., 10.], 0.9), ([1., 1., 10., 10.], 0.8), ([20., 20., 30., 30.], 0.7)], 0.3);
        assert_eq!(r.len(), 2);
    }
}
