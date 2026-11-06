//! Image helpers for scans: skew estimation, rotation, contrast/white balance, binarisation,
//! document crop and whitespace trimming. Pure Rust on `image` buffers.

use image::{GrayImage, Luma, Rgb, RgbImage};

/// Otsu threshold of a gray image.
pub fn otsu(img: &GrayImage) -> u8 {
    let mut hist = [0u64; 256];
    for p in img.pixels() {
        hist[p[0] as usize] += 1;
    }
    let total: u64 = hist.iter().sum();
    if total == 0 {
        return 128;
    }
    let sum: f64 = hist.iter().enumerate().map(|(i, h)| i as f64 * *h as f64).sum();
    let (mut sum_b, mut w_b, mut best, mut thr) = (0.0, 0u64, 0.0, 128u8);
    for (t, h) in hist.iter().enumerate() {
        w_b += h;
        if w_b == 0 {
            continue;
        }
        let w_f = total - w_b;
        if w_f == 0 {
            break;
        }
        sum_b += t as f64 * *h as f64;
        let m_b = sum_b / w_b as f64;
        let m_f = (sum - sum_b) / w_f as f64;
        let between = w_b as f64 * w_f as f64 * (m_b - m_f) * (m_b - m_f);
        if between > best {
            best = between;
            thr = t as u8;
        }
    }
    thr
}

/// Downscale so the longest side is at most `max` (nearest-ish, fast).
fn small(img: &GrayImage, max: u32) -> GrayImage {
    let (w, h) = img.dimensions();
    let longest = w.max(h);
    if longest <= max {
        return img.clone();
    }
    let s = max as f32 / longest as f32;
    image::imageops::resize(img, ((w as f32 * s) as u32).max(1), ((h as f32 * s) as u32).max(1), image::imageops::FilterType::Triangle)
}

/// Estimated skew of text lines, in degrees (positive = text rises to the right, counter-
/// clockwise). Searches ±`max_deg`. Returns 0 when the page has too little ink.
pub fn estimate_skew(img: &GrayImage, max_deg: f32) -> f32 {
    let s = small(img, 1200);
    let thr = otsu(&s);
    let (w, h) = s.dimensions();
    let ink: Vec<(f32, f32)> = s
        .enumerate_pixels()
        .filter(|(_, _, p)| p[0] < thr.saturating_sub(10).max(1))
        .map(|(x, y, _)| (x as f32 - w as f32 / 2.0, y as f32 - h as f32 / 2.0))
        .collect();
    if ink.len() < 200 || ink.len() > (w * h) as usize / 2 {
        return 0.0;
    }
    let diag = ((w * w + h * h) as f32).sqrt() as usize + 2;
    let score = |deg: f32| -> f64 {
        let (sn, cs) = deg.to_radians().sin_cos();
        let mut bins = vec![0u32; diag * 2];
        for &(x, y) in &ink {
            // Row position after rotating the points by -deg.
            let r = y * cs - x * sn;
            let i = (r + diag as f32) as usize;
            if i < bins.len() {
                bins[i] += 1;
            }
        }
        bins.iter().map(|b| (*b as f64) * (*b as f64)).sum()
    };
    let mut best = (0.0f32, score(0.0));
    let mut a = -max_deg;
    while a <= max_deg {
        let sc = score(a);
        if sc > best.1 {
            best = (a, sc);
        }
        a += 0.5;
    }
    let mut a = best.0 - 0.5;
    while a <= best.0 + 0.5 {
        let sc = score(a);
        if sc > best.1 {
            best = (a, sc);
        }
        a += 0.1;
    }
    // The projection was computed for points rotated by -deg; text rising to the right means a
    // negative slope in image coordinates (y down).
    -best.0
}

/// Rotate a gray image by `deg` counter-clockwise around its centre, keeping its size,
/// filling with white. Bilinear.
pub fn rotate_gray(img: &GrayImage, deg: f32) -> GrayImage {
    if deg.abs() < 0.01 {
        return img.clone();
    }
    let (w, h) = img.dimensions();
    let (cx, cy) = (w as f32 / 2.0, h as f32 / 2.0);
    let (sn, cs) = deg.to_radians().sin_cos();
    let mut out = GrayImage::from_pixel(w, h, Luma([255]));
    for y in 0..h {
        for x in 0..w {
            let dx = x as f32 - cx;
            let dy = y as f32 - cy;
            // Inverse mapping (image y axis points down: ccw rotation by deg on screen).
            let sx = dx * cs - dy * sn + cx;
            let sy = dx * sn + dy * cs + cy;
            if sx < 0.0 || sy < 0.0 || sx >= (w - 1) as f32 || sy >= (h - 1) as f32 {
                continue;
            }
            let (x0, y0) = (sx as u32, sy as u32);
            let (fx, fy) = (sx - x0 as f32, sy - y0 as f32);
            let p = |xx: u32, yy: u32| img.get_pixel(xx, yy)[0] as f32;
            let v = p(x0, y0) * (1.0 - fx) * (1.0 - fy)
                + p(x0 + 1, y0) * fx * (1.0 - fy)
                + p(x0, y0 + 1) * (1.0 - fx) * fy
                + p(x0 + 1, y0 + 1) * fx * fy;
            out.put_pixel(x, y, Luma([v.round() as u8]));
        }
    }
    out
}

/// Same as [`rotate_gray`] for RGB.
pub fn rotate_rgb(img: &RgbImage, deg: f32) -> RgbImage {
    if deg.abs() < 0.01 {
        return img.clone();
    }
    let (w, h) = img.dimensions();
    let (cx, cy) = (w as f32 / 2.0, h as f32 / 2.0);
    let (sn, cs) = deg.to_radians().sin_cos();
    let mut out = RgbImage::from_pixel(w, h, Rgb([255, 255, 255]));
    for y in 0..h {
        for x in 0..w {
            let dx = x as f32 - cx;
            let dy = y as f32 - cy;
            let sx = dx * cs - dy * sn + cx;
            let sy = dx * sn + dy * cs + cy;
            if sx < 0.0 || sy < 0.0 || sx >= (w - 1) as f32 || sy >= (h - 1) as f32 {
                continue;
            }
            let (x0, y0) = (sx as u32, sy as u32);
            let (fx, fy) = (sx - x0 as f32, sy - y0 as f32);
            let mut px = [0u8; 3];
            for (c, slot) in px.iter_mut().enumerate() {
                let p = |xx: u32, yy: u32| img.get_pixel(xx, yy)[c] as f32;
                let v = p(x0, y0) * (1.0 - fx) * (1.0 - fy)
                    + p(x0 + 1, y0) * fx * (1.0 - fy)
                    + p(x0, y0 + 1) * (1.0 - fx) * fy
                    + p(x0 + 1, y0 + 1) * fx * fy;
                *slot = v.round() as u8;
            }
            out.put_pixel(x, y, Rgb(px));
        }
    }
    out
}

/// Map a point of the rotated image back to the original one (inverse of `rotate_*`).
pub fn unrotate_point(x: f32, y: f32, w: u32, h: u32, deg: f32) -> (f32, f32) {
    let (cx, cy) = (w as f32 / 2.0, h as f32 / 2.0);
    let (sn, cs) = deg.to_radians().sin_cos();
    let (dx, dy) = (x - cx, y - cy);
    (dx * cs - dy * sn + cx, dx * sn + dy * cs + cy)
}

/// White balance + contrast stretch: per channel, map the 1st/99.5th percentiles to 0/255,
/// using the paper colour (bright percentile) as white.
pub fn enhance(img: &RgbImage) -> RgbImage {
    let mut hist = [[0u64; 256]; 3];
    for p in img.pixels() {
        for c in 0..3 {
            hist[c][p[c] as usize] += 1;
        }
    }
    let n = (img.width() as u64 * img.height() as u64).max(1);
    let pct = |h: &[u64; 256], q: f64| -> u8 {
        let target = (n as f64 * q) as u64;
        let mut acc = 0;
        for (i, v) in h.iter().enumerate() {
            acc += v;
            if acc >= target {
                return i as u8;
            }
        }
        255
    };
    let mut lut = [[0u8; 256]; 3];
    for c in 0..3 {
        let lo = pct(&hist[c], 0.01) as f32;
        let hi = (pct(&hist[c], 0.90) as f32).max(lo + 16.0);
        for i in 0..256 {
            lut[c][i] = (((i as f32 - lo) / (hi - lo)) * 255.0).clamp(0.0, 255.0) as u8;
        }
    }
    let mut out = img.clone();
    for p in out.pixels_mut() {
        for c in 0..3 {
            p[c] = lut[c][p[c] as usize];
        }
    }
    out
}

/// Sauvola-style adaptive binarisation (good for uneven lighting). Returns black/white.
pub fn binarize(img: &GrayImage) -> GrayImage {
    let (w, h) = img.dimensions();
    let (wu, hu) = (w as usize, h as usize);
    let mut sum = vec![0u64; (wu + 1) * (hu + 1)];
    let mut sq = vec![0u64; (wu + 1) * (hu + 1)];
    for y in 0..hu {
        let mut rs = 0u64;
        let mut rq = 0u64;
        for x in 0..wu {
            let v = img.get_pixel(x as u32, y as u32)[0] as u64;
            rs += v;
            rq += v * v;
            sum[(y + 1) * (wu + 1) + x + 1] = sum[y * (wu + 1) + x + 1] + rs;
            sq[(y + 1) * (wu + 1) + x + 1] = sq[y * (wu + 1) + x + 1] + rq;
        }
    }
    let r = ((w.min(h) / 40).max(7)) as usize;
    let mut out = GrayImage::new(w, h);
    for y in 0..hu {
        let (y0, y1) = (y.saturating_sub(r), (y + r + 1).min(hu));
        for x in 0..wu {
            let (x0, x1) = (x.saturating_sub(r), (x + r + 1).min(wu));
            let area = ((y1 - y0) * (x1 - x0)) as f64;
            let at = |t: &Vec<u64>, yy: usize, xx: usize| t[yy * (wu + 1) + xx] as f64;
            let s = at(&sum, y1, x1) - at(&sum, y0, x1) - at(&sum, y1, x0) + at(&sum, y0, x0);
            let q = at(&sq, y1, x1) - at(&sq, y0, x1) - at(&sq, y1, x0) + at(&sq, y0, x0);
            let mean = s / area;
            let sd = (q / area - mean * mean).max(0.0).sqrt();
            let t = mean * (1.0 + 0.2 * (sd / 128.0 - 1.0));
            let v = img.get_pixel(x as u32, y as u32)[0] as f64;
            out.put_pixel(x as u32, y as u32, Luma([if v > t { 255 } else { 0 }]));
        }
    }
    out
}

/// Bounding box (x0, y0, x1, y1) of the content: pixels darker than the background by more
/// than `tolerance`. None if the image is blank.
pub fn content_bbox(img: &GrayImage, tolerance: u8) -> Option<(u32, u32, u32, u32)> {
    let (w, h) = img.dimensions();
    let bg = 255u8.saturating_sub(tolerance);
    let (mut x0, mut y0, mut x1, mut y1) = (w, h, 0u32, 0u32);
    for (x, y, p) in img.enumerate_pixels() {
        if p[0] < bg {
            x0 = x0.min(x);
            y0 = y0.min(y);
            x1 = x1.max(x + 1);
            y1 = y1.max(y + 1);
        }
    }
    (x1 > x0 && y1 > y0).then_some((x0, y0, x1, y1))
}

/// Find a sheet of paper on a darker background: the bounding box of the largest bright
/// region (rows/columns where more than half of the pixels are "paper"). Returns None when the
/// photo seems to be the page already (nothing worth cropping).
pub fn document_bbox(img: &GrayImage) -> Option<(u32, u32, u32, u32)> {
    let s = small(img, 800);
    let (w, h) = s.dimensions();
    let scale = img.width() as f32 / w as f32;
    let thr = otsu(&s);
    let bright = |x: u32, y: u32| s.get_pixel(x, y)[0] > thr;
    let rows: Vec<bool> = (0..h).map(|y| (0..w).filter(|x| bright(*x, y)).count() as f32 > w as f32 * 0.35).collect();
    let cols: Vec<bool> = (0..w).map(|x| (0..h).filter(|y| bright(x, *y)).count() as f32 > h as f32 * 0.35).collect();
    let span = |v: &[bool]| -> Option<(usize, usize)> {
        // Longest run of true.
        let (mut best, mut start) = ((0usize, 0usize), None);
        for (i, b) in v.iter().chain(std::iter::once(&false)).enumerate() {
            match (b, start) {
                (true, None) => start = Some(i),
                (false, Some(s)) => {
                    if i - s > best.1 - best.0 {
                        best = (s, i);
                    }
                    start = None;
                }
                _ => {}
            }
        }
        (best.1 > best.0).then_some(best)
    };
    let (ry0, ry1) = span(&rows)?;
    let (cx0, cx1) = span(&cols)?;
    let area = ((ry1 - ry0) * (cx1 - cx0)) as f32 / (w * h) as f32;
    if !(0.2..0.93).contains(&area) {
        return None;
    }
    Some((
        (cx0 as f32 * scale) as u32,
        (ry0 as f32 * scale) as u32,
        ((cx1 as f32 * scale) as u32).min(img.width()),
        ((ry1 as f32 * scale) as u32).min(img.height()),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines_image(angle: f32) -> GrayImage {
        let mut img = GrayImage::from_pixel(800, 600, Luma([255]));
        for row in 0..12 {
            let y0 = 60 + row * 40;
            for y in y0..y0 + 8 {
                for x in 100..700 {
                    img.put_pixel(x, y, Luma([0]));
                }
            }
        }
        rotate_gray(&img, angle)
    }

    #[test]
    fn skew_is_found() {
        for a in [-3.0f32, 0.0, 2.0] {
            let est = estimate_skew(&lines_image(a), 10.0);
            assert!((est - a).abs() < 0.3, "angle {a}: estimated {est}");
        }
    }

    #[test]
    fn bbox_of_blank_is_none() {
        assert!(content_bbox(&GrayImage::from_pixel(10, 10, Luma([255])), 10).is_none());
    }
}
