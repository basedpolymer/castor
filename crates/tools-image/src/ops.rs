//! Pixel operations shared by the tools: SIMD resizing, fitting, blur, compositing, drawing,
//! text (rendered through SVG with system fonts), auto-trim and saliency.

use fast_image_resize as fir;
use image::{DynamicImage, GrayImage, Rgba, RgbaImage};
use rayon::prelude::*;

/// Resize an RGBA image (alpha-correct, Lanczos-3 / Catmull-Rom).
pub fn resize_rgba(img: &RgbaImage, w: u32, h: u32) -> RgbaImage {
    let (w, h) = (w.max(1), h.max(1));
    if img.dimensions() == (w, h) {
        return img.clone();
    }
    let up = w > img.width() || h > img.height();
    let f = if up { fir::FilterType::CatmullRom } else { fir::FilterType::Lanczos3 };
    let mut dst = RgbaImage::new(w, h);
    let mut r = fir::Resizer::new();
    let opts = fir::ResizeOptions::new().resize_alg(fir::ResizeAlg::Convolution(f)).use_alpha(true);
    if r.resize(img, &mut dst, &opts).is_err() {
        return image::imageops::resize(img, w, h, image::imageops::FilterType::Lanczos3);
    }
    dst
}

/// Resize any image, keeping its pixel type when the SIMD resizer supports it.
pub fn resize_dyn(img: &DynamicImage, w: u32, h: u32) -> DynamicImage {
    let (w, h) = (w.max(1), h.max(1));
    if (img.width(), img.height()) == (w, h) {
        return img.clone();
    }
    let up = w > img.width() || h > img.height();
    let f = if up { fir::FilterType::CatmullRom } else { fir::FilterType::Lanczos3 };
    let opts = fir::ResizeOptions::new().resize_alg(fir::ResizeAlg::Convolution(f)).use_alpha(true);
    let mut r = fir::Resizer::new();
    let mut dst = match img {
        DynamicImage::ImageLuma8(_) => DynamicImage::ImageLuma8(GrayImage::new(w, h)),
        DynamicImage::ImageRgb8(_) => DynamicImage::ImageRgb8(image::RgbImage::new(w, h)),
        DynamicImage::ImageRgba8(_) => DynamicImage::ImageRgba8(RgbaImage::new(w, h)),
        DynamicImage::ImageRgb16(_) => DynamicImage::ImageRgb16(image::ImageBuffer::new(w, h)),
        DynamicImage::ImageRgba16(_) => DynamicImage::ImageRgba16(image::ImageBuffer::new(w, h)),
        DynamicImage::ImageLuma16(_) => DynamicImage::ImageLuma16(image::ImageBuffer::new(w, h)),
        other => return DynamicImage::ImageRgba8(resize_rgba(&other.to_rgba8(), w, h)),
    };
    if r.resize(img, &mut dst, &opts).is_err() {
        return DynamicImage::ImageRgba8(resize_rgba(&img.to_rgba8(), w, h));
    }
    dst
}

/// Size that fits (w, h) inside (bw, bh), keeping the ratio.
pub fn fit_size(w: u32, h: u32, bw: u32, bh: u32) -> (u32, u32) {
    let s = (bw as f64 / w as f64).min(bh as f64 / h as f64);
    (((w as f64 * s).round() as u32).max(1), ((h as f64 * s).round() as u32).max(1))
}

/// Scale into a `side`² square, centred on a background (transparent allowed).
pub fn fit_square(img: &RgbaImage, side: u32, bg: [u8; 4]) -> RgbaImage {
    let (w, h) = fit_size(img.width(), img.height(), side, side);
    let small = resize_rgba(img, w, h);
    let mut out = RgbaImage::from_pixel(side, side, Rgba(bg));
    overlay(&mut out, &small, ((side - w) / 2) as i64, ((side - h) / 2) as i64, 1.0);
    out
}

/// Alpha-composite `top` onto `base` at (x, y) with an extra opacity (source-over).
pub fn overlay(base: &mut RgbaImage, top: &RgbaImage, x: i64, y: i64, opacity: f32) {
    let (bw, bh) = (base.width() as i64, base.height() as i64);
    let (tw, th) = (top.width() as i64, top.height() as i64);
    let x0 = x.max(0);
    let y0 = y.max(0);
    let x1 = (x + tw).min(bw);
    let y1 = (y + th).min(bh);
    if x0 >= x1 || y0 >= y1 {
        return;
    }
    let stride = bw as usize * 4;
    let tstride = tw as usize * 4;
    let traw = top.as_raw();
    base.par_chunks_mut(stride).enumerate().skip(y0 as usize).take((y1 - y0) as usize).for_each(|(yy, row)| {
        let ty = (yy as i64 - y) as usize;
        for xx in x0..x1 {
            let tx = (xx - x) as usize;
            let s = &traw[ty * tstride + tx * 4..ty * tstride + tx * 4 + 4];
            let d = &mut row[xx as usize * 4..xx as usize * 4 + 4];
            blend_px(d, s, opacity);
        }
    });
}

#[inline]
pub fn blend_px(d: &mut [u8], s: &[u8], opacity: f32) {
    let sa = s[3] as f32 / 255.0 * opacity;
    if sa <= 0.0 {
        return;
    }
    let da = d[3] as f32 / 255.0;
    let oa = sa + da * (1.0 - sa);
    if oa <= 0.0 {
        return;
    }
    for c in 0..3 {
        let v = (s[c] as f32 * sa + d[c] as f32 * da * (1.0 - sa)) / oa;
        d[c] = v.round().clamp(0.0, 255.0) as u8;
    }
    d[3] = (oa * 255.0).round() as u8;
}

/// Fast approximate Gaussian blur (3 box passes) on RGBA, premultiplied to avoid dark fringes.
pub fn blur_rgba(img: &RgbaImage, sigma: f32) -> RgbaImage {
    if sigma < 0.3 {
        return img.clone();
    }
    let (w, h) = (img.width() as usize, img.height() as usize);
    let mut ch: Vec<Vec<f32>> = vec![vec![0.0; w * h]; 4];
    for (i, p) in img.pixels().enumerate() {
        let a = p[3] as f32 / 255.0;
        ch[0][i] = p[0] as f32 * a;
        ch[1][i] = p[1] as f32 * a;
        ch[2][i] = p[2] as f32 * a;
        ch[3][i] = p[3] as f32;
    }
    let r = box_radius(sigma);
    ch.par_iter_mut().for_each(|c| {
        for _ in 0..3 {
            box_blur_plane(c, w, h, r);
        }
    });
    let mut out = RgbaImage::new(w as u32, h as u32);
    for (i, p) in out.pixels_mut().enumerate() {
        let a = ch[3][i];
        let af = (a / 255.0).max(1e-6);
        p.0 = [
            (ch[0][i] / af).round().clamp(0.0, 255.0) as u8,
            (ch[1][i] / af).round().clamp(0.0, 255.0) as u8,
            (ch[2][i] / af).round().clamp(0.0, 255.0) as u8,
            a.round().clamp(0.0, 255.0) as u8,
        ];
    }
    out
}

/// Blur of a single channel.
pub fn blur_gray(img: &GrayImage, sigma: f32) -> GrayImage {
    let (w, h) = (img.width() as usize, img.height() as usize);
    let mut c: Vec<f32> = img.as_raw().iter().map(|v| *v as f32).collect();
    let r = box_radius(sigma);
    for _ in 0..3 {
        box_blur_plane(&mut c, w, h, r);
    }
    GrayImage::from_raw(w as u32, h as u32, c.iter().map(|v| v.round().clamp(0.0, 255.0) as u8).collect()).unwrap_or_else(|| img.clone())
}

fn box_radius(sigma: f32) -> usize {
    // Three box passes of width 2r+1 approximate a Gaussian of this sigma.
    (((12.0 * sigma * sigma / 3.0 + 1.0).sqrt() - 1.0) / 2.0).round().max(1.0) as usize
}

fn box_blur_plane(c: &mut [f32], w: usize, h: usize, r: usize) {
    let mut tmp = vec![0f32; w * h];
    tmp.par_chunks_mut(w).enumerate().for_each(|(y, row)| {
        let src = &c[y * w..(y + 1) * w];
        let mut acc = 0f32;
        for i in 0..=r.min(w - 1) {
            acc += src[i];
        }
        acc += src[0] * r as f32;
        let n = (2 * r + 1) as f32;
        for x in 0..w {
            row[x] = acc / n;
            let add = src[(x + r + 1).min(w - 1)];
            let sub = src[x.saturating_sub(r)];
            acc += add - sub;
        }
    });
    // Vertical pass on transposed blocks for cache friendliness.
    let cols: Vec<(usize, Vec<f32>)> = (0..w)
        .into_par_iter()
        .map(|x| {
            let mut col = vec![0f32; h];
            let n = (2 * r + 1) as f32;
            let mut acc = 0f32;
            for i in 0..=r.min(h - 1) {
                acc += tmp[i * w + x];
            }
            acc += tmp[x] * r as f32;
            for y in 0..h {
                col[y] = acc / n;
                let add = tmp[(y + r + 1).min(h - 1) * w + x];
                let sub = tmp[y.saturating_sub(r) * w + x];
                acc += add - sub;
            }
            (x, col)
        })
        .collect();
    for (x, col) in cols {
        for (y, v) in col.into_iter().enumerate() {
            c[y * w + x] = v;
        }
    }
}

/// Pixelate (mosaic) a region in place.
pub fn pixelate_region(img: &mut RgbaImage, x0: u32, y0: u32, x1: u32, y1: u32, block: u32) {
    let block = block.max(2);
    let mut by = y0;
    while by < y1 {
        let mut bx = x0;
        let bh = block.min(y1 - by);
        while bx < x1 {
            let bw = block.min(x1 - bx);
            let mut s = [0u64; 4];
            for y in by..by + bh {
                for x in bx..bx + bw {
                    let p = img.get_pixel(x, y);
                    for c in 0..4 {
                        s[c] += p[c] as u64;
                    }
                }
            }
            let n = (bw * bh) as u64;
            let avg = Rgba([(s[0] / n) as u8, (s[1] / n) as u8, (s[2] / n) as u8, (s[3] / n) as u8]);
            for y in by..by + bh {
                for x in bx..bx + bw {
                    img.put_pixel(x, y, avg);
                }
            }
            bx += block;
        }
        by += block;
    }
}

/// Signed distance-ish coverage of a rounded rectangle at pixel (x, y) (anti-aliased, 0..1).
#[inline]
pub fn rounded_rect_cov(x: f32, y: f32, w: f32, h: f32, r: f32) -> f32 {
    let r = r.min(w / 2.0).min(h / 2.0).max(0.0);
    let px = x + 0.5;
    let py = y + 0.5;
    let qx = (px - w / 2.0).abs() - (w / 2.0 - r);
    let qy = (py - h / 2.0).abs() - (h / 2.0 - r);
    let outside = (qx.max(0.0).powi(2) + qy.max(0.0).powi(2)).sqrt() + qx.max(qy).min(0.0) - r;
    (0.5 - outside).clamp(0.0, 1.0)
}

/// Apply rounded corners to an image's alpha.
pub fn round_corners(img: &mut RgbaImage, radius: f32) {
    if radius <= 0.0 {
        return;
    }
    let (w, h) = (img.width() as f32, img.height() as f32);
    let r = radius.min(w / 2.0).min(h / 2.0);
    let iw = img.width();
    img.par_chunks_mut(iw as usize * 4).enumerate().for_each(|(y, row)| {
        let yf = y as f32;
        if yf >= r && yf < h - r {
            return;
        }
        for x in 0..iw as usize {
            let xf = x as f32;
            if xf >= r && xf < w - r {
                continue;
            }
            let c = rounded_rect_cov(xf, yf, w, h, r);
            row[x * 4 + 3] = (row[x * 4 + 3] as f32 * c).round() as u8;
        }
    });
}

/// Fill a rounded rectangle (anti-aliased) with a colour, source-over.
pub fn fill_rounded_rect(img: &mut RgbaImage, x: f32, y: f32, w: f32, h: f32, r: f32, color: [u8; 4]) {
    let x0 = x.floor().max(0.0) as u32;
    let y0 = y.floor().max(0.0) as u32;
    let x1 = ((x + w).ceil() as u32).min(img.width());
    let y1 = ((y + h).ceil() as u32).min(img.height());
    for py in y0..y1 {
        for px in x0..x1 {
            let c = rounded_rect_cov(px as f32 - x, py as f32 - y, w, h, r);
            if c > 0.0 {
                let s = [color[0], color[1], color[2], (color[3] as f32 * c).round() as u8];
                blend_px(&mut img.get_pixel_mut(px, py).0, &s, 1.0);
            }
        }
    }
}

/// Fill a circle (anti-aliased).
pub fn fill_circle(img: &mut RgbaImage, cx: f32, cy: f32, r: f32, color: [u8; 4]) {
    let x0 = (cx - r - 1.0).floor().max(0.0) as u32;
    let y0 = (cy - r - 1.0).floor().max(0.0) as u32;
    let x1 = ((cx + r + 1.0).ceil() as u32).min(img.width());
    let y1 = ((cy + r + 1.0).ceil() as u32).min(img.height());
    for py in y0..y1 {
        for px in x0..x1 {
            let d = ((px as f32 + 0.5 - cx).powi(2) + (py as f32 + 0.5 - cy).powi(2)).sqrt();
            let c = (r - d + 0.5).clamp(0.0, 1.0);
            if c > 0.0 {
                let s = [color[0], color[1], color[2], (color[3] as f32 * c).round() as u8];
                blend_px(&mut img.get_pixel_mut(px, py).0, &s, 1.0);
            }
        }
    }
}

/// Drop shadow of an RGBA shape: blurred alpha, tinted, offset.
pub fn shadow(shape: &RgbaImage, blur: f32, color: [u8; 4], pad: u32) -> RgbaImage {
    let (w, h) = (shape.width() + 2 * pad, shape.height() + 2 * pad);
    let mut a = GrayImage::new(w, h);
    for (x, y, p) in shape.enumerate_pixels() {
        a.put_pixel(x + pad, y + pad, image::Luma([p[3]]));
    }
    let a = blur_gray(&a, blur);
    let mut out = RgbaImage::new(w, h);
    for (o, v) in out.pixels_mut().zip(a.pixels()) {
        o.0 = [color[0], color[1], color[2], ((v[0] as u32 * color[3] as u32) / 255) as u8];
    }
    out
}

/// Escape text for SVG.
pub fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;").replace('\'', "&apos;")
}

/// Render one line of text (system font by family name) to a tight RGBA image.
/// `size` = font size in pixels. Returns None for empty text.
pub fn render_text(text: &str, family: &str, size: f32, color: [u8; 4], bold: bool) -> Option<RgbaImage> {
    let text = text.trim_end();
    if text.is_empty() {
        return None;
    }
    let family = if family.trim().is_empty() { "sans-serif" } else { family };
    let lines: Vec<&str> = text.lines().collect();
    let lh = size * 1.25;
    let max_chars = lines.iter().map(|l| l.chars().count()).max().unwrap_or(1) as f32;
    let w = (max_chars * size * 0.75 + size * 2.0).ceil();
    let h = (lh * lines.len() as f32 + size).ceil();
    let mut tspans = String::new();
    for (i, l) in lines.iter().enumerate() {
        tspans += &format!("<tspan x=\"{}\" y=\"{}\">{}</tspan>", size * 0.5, size + i as f32 * lh, xml_escape(l));
    }
    let svg = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{w}\" height=\"{h}\"><text font-family=\"{}, sans-serif\" font-size=\"{size}\" font-weight=\"{}\" fill=\"rgb({},{},{})\" fill-opacity=\"{}\" xml:space=\"preserve\">{tspans}</text></svg>",
        xml_escape(family),
        if bold { 700 } else { 400 },
        color[0],
        color[1],
        color[2],
        color[3] as f32 / 255.0
    );
    let img = crate::codec::render_svg(svg.as_bytes(), 0, None).ok()?;
    trim_transparent(&img)
}

/// Crop away fully transparent borders.
pub fn trim_transparent(img: &RgbaImage) -> Option<RgbaImage> {
    let (w, h) = img.dimensions();
    let (mut x0, mut y0, mut x1, mut y1) = (w, h, 0u32, 0u32);
    for (x, y, p) in img.enumerate_pixels() {
        if p[3] > 0 {
            x0 = x0.min(x);
            y0 = y0.min(y);
            x1 = x1.max(x);
            y1 = y1.max(y);
        }
    }
    if x0 > x1 {
        return None;
    }
    Some(image::imageops::crop_imm(img, x0, y0, x1 - x0 + 1, y1 - y0 + 1).to_image())
}

/// Bounding box of the content inside uniform borders (auto-trim): the border colour is taken
/// from the corners; `tolerance` 0..=255 per channel.
pub fn content_bounds(img: &RgbaImage, tolerance: u8) -> Option<(u32, u32, u32, u32)> {
    let (w, h) = img.dimensions();
    if w < 3 || h < 3 {
        return None;
    }
    let corners = [img.get_pixel(0, 0), img.get_pixel(w - 1, 0), img.get_pixel(0, h - 1), img.get_pixel(w - 1, h - 1)];
    // The most common corner colour is the border.
    let bg = *corners.iter().max_by_key(|c| corners.iter().filter(|d| close(c, d, tolerance)).count())?;
    let differs = |p: &Rgba<u8>| !close(&p, &bg, tolerance);
    let row_has = |y: u32| (0..w).any(|x| differs(img.get_pixel(x, y)));
    let col_has = |x: u32, y0: u32, y1: u32| (y0..=y1).any(|y| differs(img.get_pixel(x, y)));
    let y0 = (0..h).find(|y| row_has(*y))?;
    let y1 = (0..h).rev().find(|y| row_has(*y))?;
    let x0 = (0..w).find(|x| col_has(*x, y0, y1))?;
    let x1 = (0..w).rev().find(|x| col_has(*x, y0, y1))?;
    Some((x0, y0, x1 - x0 + 1, y1 - y0 + 1))
}

fn close(a: &Rgba<u8>, b: &Rgba<u8>, t: u8) -> bool {
    // Fully transparent pixels match each other whatever their colour.
    if a[3] == 0 && b[3] == 0 {
        return true;
    }
    (0..4).all(|c| (a[c] as i16 - b[c] as i16).unsigned_abs() <= t as u16)
}

/// Rotate by an arbitrary angle (degrees, clockwise) with bilinear sampling. `expand`: grow the
/// canvas to keep every pixel (corners filled with `bg`); otherwise crop to the largest
/// axis-aligned rectangle with no empty corners.
pub fn rotate_any(img: &RgbaImage, deg: f32, bg: [u8; 4], expand: bool) -> RgbaImage {
    let a = deg.to_radians();
    let (s, c) = a.sin_cos();
    let (w, h) = (img.width() as f32, img.height() as f32);
    let (ow, oh) = if expand {
        ((w * c.abs() + h * s.abs() - 1e-3).ceil(), (w * s.abs() + h * c.abs() - 1e-3).ceil())
    } else {
        // Largest rectangle of the same aspect ratio inside the rotated image.
        let (sa, ca) = (s.abs(), c.abs());
        let (long, short) = if w >= h { (w, h) } else { (h, w) };
        let (wr, hr) = if short <= 2.0 * sa * ca * long || (sa - ca).abs() < 1e-10 {
            let x = 0.5 * short;
            if w >= h { (x / sa, x / ca) } else { (x / ca, x / sa) }
        } else {
            let cos2 = ca * ca - sa * sa;
            ((w * ca - h * sa) / cos2, (h * ca - w * sa) / cos2)
        };
        (wr.floor().max(1.0), hr.floor().max(1.0))
    };
    let (ow_u, oh_u) = (ow as u32, oh as u32);
    let mut out = RgbaImage::new(ow_u, oh_u);
    let (cx, cy) = (w / 2.0, h / 2.0);
    let (ocx, ocy) = (ow / 2.0, oh / 2.0);
    let raw = img.as_raw();
    let iw = img.width() as i64;
    let ih = img.height() as i64;
    out.par_chunks_mut(ow_u as usize * 4).enumerate().for_each(|(y, row)| {
        for x in 0..ow_u as usize {
            let dx = x as f32 + 0.5 - ocx;
            let dy = y as f32 + 0.5 - ocy;
            // Inverse rotation.
            let sx = c * dx + s * dy + cx - 0.5;
            let sy = -s * dx + c * dy + cy - 0.5;
            let x0 = sx.floor() as i64;
            let y0 = sy.floor() as i64;
            let (tx, ty) = (sx - x0 as f32, sy - y0 as f32);
            let mut acc = [0f32; 4];
            let mut wsum = 0f32;
            for (xx, yy, wgt) in [(x0, y0, (1.0 - tx) * (1.0 - ty)), (x0 + 1, y0, tx * (1.0 - ty)), (x0, y0 + 1, (1.0 - tx) * ty), (x0 + 1, y0 + 1, tx * ty)] {
                let p: [f32; 4] = if xx >= 0 && yy >= 0 && xx < iw && yy < ih {
                    let k = ((yy * iw + xx) * 4) as usize;
                    [raw[k] as f32, raw[k + 1] as f32, raw[k + 2] as f32, raw[k + 3] as f32]
                } else {
                    [bg[0] as f32, bg[1] as f32, bg[2] as f32, bg[3] as f32]
                };
                let a = p[3] / 255.0 * wgt;
                acc[0] += p[0] * a;
                acc[1] += p[1] * a;
                acc[2] += p[2] * a;
                acc[3] += p[3] * wgt;
                wsum += a;
            }
            let px = &mut row[x * 4..x * 4 + 4];
            if wsum > 0.0 {
                px[0] = (acc[0] / wsum).round() as u8;
                px[1] = (acc[1] / wsum).round() as u8;
                px[2] = (acc[2] / wsum).round() as u8;
            }
            px[3] = acc[3].round().clamp(0.0, 255.0) as u8;
        }
    });
    out
}

/// Saliency centre (0..1, 0..1) for automatic focus: gradient energy weighted by colourfulness
/// and skin-like tones, centre-biased.
pub fn saliency_center(img: &DynamicImage) -> (f32, f32) {
    let small = img.thumbnail(128, 128).to_rgb8();
    let (w, h) = small.dimensions();
    if w < 3 || h < 3 {
        return (0.5, 0.5);
    }
    let lum = |x: u32, y: u32| {
        let p = small.get_pixel(x, y);
        0.299 * p[0] as f32 + 0.587 * p[1] as f32 + 0.114 * p[2] as f32
    };
    let (mut sx, mut sy, mut sw) = (0f32, 0f32, 0f32);
    for y in 1..h - 1 {
        for x in 1..w - 1 {
            let gx = lum(x + 1, y) - lum(x - 1, y);
            let gy = lum(x, y + 1) - lum(x, y - 1);
            let p = small.get_pixel(x, y);
            let (r, g, b) = (p[0] as f32, p[1] as f32, p[2] as f32);
            let mx = r.max(g).max(b);
            let mn = r.min(g).min(b);
            let sat = if mx > 0.0 { (mx - mn) / mx } else { 0.0 };
            let skin = if r > 95.0 && g > 40.0 && b > 20.0 && r > g && r > b && (r - g) > 15.0 { 1.5 } else { 1.0 };
            let dxn = x as f32 / w as f32 - 0.5;
            let dyn_ = y as f32 / h as f32 - 0.5;
            let centre = 1.0 - (dxn * dxn + dyn_ * dyn_).sqrt();
            let e = ((gx * gx + gy * gy).sqrt() * (0.5 + sat) * skin).powf(1.5) * (0.4 + centre);
            sx += e * x as f32;
            sy += e * y as f32;
            sw += e;
        }
    }
    if sw <= 0.0 {
        return (0.5, 0.5);
    }
    ((sx / sw + 0.5) / w as f32, (sy / sw + 0.5) / h as f32)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn trim_bounds() {
        let mut img = RgbaImage::from_pixel(50, 40, Rgba([255, 255, 255, 255]));
        for y in 10..20 {
            for x in 5..30 {
                img.put_pixel(x, y, Rgba([0, 0, 0, 255]));
            }
        }
        assert_eq!(content_bounds(&img, 10), Some((5, 10, 25, 10)));
    }
    #[test]
    fn rotate_90_expand_keeps_size() {
        let img = RgbaImage::from_pixel(40, 20, Rgba([10, 20, 30, 255]));
        let r = rotate_any(&img, 90.0, [0, 0, 0, 0], true);
        assert_eq!(r.dimensions(), (20, 40));
    }
    #[test]
    fn blur_keeps_flat() {
        let img = RgbaImage::from_pixel(30, 30, Rgba([100, 150, 200, 255]));
        let b = blur_rgba(&img, 3.0);
        assert_eq!(b.get_pixel(15, 15).0, [100, 150, 200, 255]);
    }
}
