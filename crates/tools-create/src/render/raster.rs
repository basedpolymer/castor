//! Raster helpers: compositing, box-blurred shadows, perspective warps (homographies) and the
//! procedural "scene" backgrounds of the mockup tool.

use image::{Rgba, RgbaImage};
use rayon::prelude::*;

use crate::render::noise::{Noise, hash2, smooth};
use crate::render::style;

/// Source-over composite of `top` onto `dst` at (x, y), straight alpha.
pub fn over(dst: &mut RgbaImage, top: &RgbaImage, x: i64, y: i64) {
    image::imageops::overlay(dst, top, x, y);
}

/// Blur the alpha of `src` into a shadow of colour `c` (3 box passes ≈ gaussian), with an
/// extra margin of `r` pixels on each side so nothing is clipped.
pub fn shadow(src: &RgbaImage, r: u32, c: [u8; 3], opacity: f32) -> RgbaImage {
    let (w, h) = (src.width() + 2 * r, src.height() + 2 * r);
    let mut a = vec![0f32; (w * h) as usize];
    for (x, y, p) in src.enumerate_pixels() {
        a[((y + r) * w + x + r) as usize] = p[3] as f32 / 255.0;
    }
    let box_r = (r / 3).max(1) as i64;
    for _ in 0..3 {
        a = box_h(&a, w as usize, h as usize, box_r);
        a = box_v(&a, w as usize, h as usize, box_r);
    }
    let mut out = RgbaImage::new(w, h);
    for (i, p) in out.pixels_mut().enumerate() {
        *p = Rgba([c[0], c[1], c[2], (a[i] * opacity * 255.0).clamp(0.0, 255.0) as u8]);
    }
    out
}

fn box_h(a: &[f32], w: usize, h: usize, r: i64) -> Vec<f32> {
    let mut o = vec![0f32; a.len()];
    o.par_chunks_mut(w).enumerate().for_each(|(y, row)| {
        let src = &a[y * w..(y + 1) * w];
        let n = (2 * r + 1) as f32;
        let mut acc = 0f32;
        for i in -r..=r {
            acc += src[i.clamp(0, w as i64 - 1) as usize];
        }
        for x in 0..w as i64 {
            row[x as usize] = acc / n;
            let add = (x + r + 1).clamp(0, w as i64 - 1) as usize;
            let sub = (x - r).clamp(0, w as i64 - 1) as usize;
            acc += src[add] - src[sub];
        }
    });
    let _ = h;
    o
}

fn box_v(a: &[f32], w: usize, h: usize, r: i64) -> Vec<f32> {
    // Transpose, blur rows, transpose back (cache friendly enough for our sizes).
    let mut t = vec![0f32; a.len()];
    for y in 0..h {
        for x in 0..w {
            t[x * h + y] = a[y * w + x];
        }
    }
    let b = box_h(&t, h, w, r);
    let mut o = vec![0f32; a.len()];
    for y in 0..h {
        for x in 0..w {
            o[y * w + x] = b[x * h + y];
        }
    }
    o
}

/// 3 × 3 homography mapping the unit square corners (0,0) (1,0) (1,1) (0,1) to `q`.
fn square_to_quad(q: [[f64; 2]; 4]) -> [f64; 9] {
    let [(x0, y0), (x1, y1), (x2, y2), (x3, y3)] = q.map(|p| (p[0], p[1]));
    let (dx1, dx2, dx3) = (x1 - x2, x3 - x2, x0 - x1 + x2 - x3);
    let (dy1, dy2, dy3) = (y1 - y2, y3 - y2, y0 - y1 + y2 - y3);
    let (g, h) = if dx3.abs() < 1e-12 && dy3.abs() < 1e-12 {
        (0.0, 0.0)
    } else {
        let den = dx1 * dy2 - dx2 * dy1;
        ((dx3 * dy2 - dx2 * dy3) / den, (dx1 * dy3 - dx3 * dy1) / den)
    };
    [x1 - x0 + g * x1, x3 - x0 + h * x3, x0, y1 - y0 + g * y1, y3 - y0 + h * y3, y0, g, h, 1.0]
}

fn invert(m: [f64; 9]) -> [f64; 9] {
    let [a, b, c, d, e, f, g, h, i] = m;
    let det = a * (e * i - f * h) - b * (d * i - f * g) + c * (d * h - e * g);
    let k = 1.0 / det;
    [
        (e * i - f * h) * k,
        (c * h - b * i) * k,
        (b * f - c * e) * k,
        (f * g - d * i) * k,
        (a * i - c * g) * k,
        (c * d - a * f) * k,
        (d * h - e * g) * k,
        (b * g - a * h) * k,
        (a * e - b * d) * k,
    ]
}

/// Warp `src` so its corners land on `quad` (tl, tr, br, bl) in an image of size (w, h).
pub fn warp(src: &RgbaImage, quad: [[f64; 2]; 4], w: u32, h: u32) -> RgbaImage {
    let inv = invert(square_to_quad(quad));
    let (sw, sh) = (src.width() as f64, src.height() as f64);
    let mut out = vec![0u8; (w * h * 4) as usize];
    out.par_chunks_mut((w * 4) as usize).enumerate().for_each(|(y, row)| {
        for x in 0..w as usize {
            let (px, py) = (x as f64 + 0.5, y as f64 + 0.5);
            let z = inv[6] * px + inv[7] * py + inv[8];
            let u = (inv[0] * px + inv[1] * py + inv[2]) / z;
            let v = (inv[3] * px + inv[4] * py + inv[5]) / z;
            if !(-0.002..=1.002).contains(&u) || !(-0.002..=1.002).contains(&v) {
                continue;
            }
            let c = bilinear(src, u * sw - 0.5, v * sh - 0.5);
            row[x * 4..x * 4 + 4].copy_from_slice(&c);
        }
    });
    RgbaImage::from_raw(w, h, out).expect("size")
}

fn bilinear(img: &RgbaImage, x: f64, y: f64) -> [u8; 4] {
    let (w, h) = (img.width() as i64, img.height() as i64);
    let (x0, y0) = (x.floor() as i64, y.floor() as i64);
    let (fx, fy) = ((x - x0 as f64) as f32, (y - y0 as f64) as f32);
    let px = |xx: i64, yy: i64| -> [f32; 4] {
        if xx < 0 || yy < 0 || xx >= w || yy >= h {
            return [0.0; 4];
        }
        let p = img.get_pixel(xx as u32, yy as u32);
        // Premultiply for correct edge filtering.
        let a = p[3] as f32 / 255.0;
        [p[0] as f32 * a, p[1] as f32 * a, p[2] as f32 * a, p[3] as f32]
    };
    let (a, b, c, d) = (px(x0, y0), px(x0 + 1, y0), px(x0, y0 + 1), px(x0 + 1, y0 + 1));
    let mut o = [0f32; 4];
    for i in 0..4 {
        o[i] = (a[i] * (1.0 - fx) + b[i] * fx) * (1.0 - fy) + (c[i] * (1.0 - fx) + d[i] * fx) * fy;
    }
    if o[3] <= 0.0 {
        return [0; 4];
    }
    let al = o[3] / 255.0;
    [(o[0] / al).min(255.0) as u8, (o[1] / al).min(255.0) as u8, (o[2] / al).min(255.0) as u8, o[3].min(255.0) as u8]
}

/// Corners of a w × h rectangle centred at (cx, cy) after a rotation of `yaw` degrees around
/// the vertical axis and `pitch` around the horizontal one, seen by a camera at distance
/// `dist` (in multiples of the larger side). Order: tl, tr, br, bl.
pub fn tilt(cx: f64, cy: f64, w: f64, h: f64, yaw: f64, pitch: f64, dist: f64) -> [[f64; 2]; 4] {
    let (sy, cyw) = yaw.to_radians().sin_cos();
    let (sp, cp) = pitch.to_radians().sin_cos();
    let d = dist * w.max(h);
    let pts = [(-w / 2.0, -h / 2.0), (w / 2.0, -h / 2.0), (w / 2.0, h / 2.0), (-w / 2.0, h / 2.0)];
    pts.map(|(x, y)| {
        // Rotate around Y, then X.
        let (x1, z1) = (x * cyw, -x * sy);
        let (y2, z2) = (y * cp - z1 * sp, y * sp + z1 * cp);
        let k = d / (d + z2);
        [cx + x1 * k, cy + y2 * k]
    })
}

pub const SCENES: &[&str] = &[
    "castor", "sunset", "ocean", "aurora", "peach", "forest", "dusk", "ink", "paper", "night", "tint", "wood", "walnut", "concrete", "marble",
    "linen", "felt", "terrazzo", "desk", "studio", "custom", "transparent",
];

/// Render a scene background (RGBA; transparent only for "transparent").
pub fn scene(name: &str, w: u32, h: u32, seed: u64, custom: [u8; 4]) -> RgbaImage {
    if let Some((_, cols)) = crate::gradient::PALETTES.iter().find(|(n, _)| *n == name) {
        let cols: Vec<[u8; 4]> = cols.iter().filter_map(|c| castor_core::options::parse_hex_color(c)).collect();
        let m = crate::gradient::Mesh::with(&cols, 6, seed, [0.4, 0.65, 0.3, 0.12]);
        return image::DynamicImage::ImageRgb8(m.render(w, h, 0.0)).to_rgba8();
    }
    let n = Noise::new(seed);
    let short = w.min(h) as f32;
    let px = |f: &(dyn Fn(f32, f32, u32, u32) -> [f32; 3] + Sync)| -> RgbaImage {
        let mut buf = vec![0u8; (w * h * 4) as usize];
        buf.par_chunks_mut((w * 4) as usize).enumerate().for_each(|(y, row)| {
            for x in 0..w as usize {
                let c = f(x as f32 / short, y as f32 / short, x as u32, y as u32);
                // Studio light: brighter top-left, gentle vignette.
                let (u, v) = (x as f32 / w as f32, y as f32 / h as f32);
                let light = 1.0 + 0.10 * (1.0 - ((u - 0.25).powi(2) + (v - 0.15).powi(2)).sqrt()) - 0.18 * ((u - 0.5).powi(2) + (v - 0.5).powi(2));
                for i in 0..3 {
                    row[x * 4 + i] = style::linear_to_srgb(c[i] * light);
                }
                row[x * 4 + 3] = 255;
            }
        });
        RgbaImage::from_raw(w, h, buf).expect("size")
    };
    let lin = |c: [u8; 4]| [style::srgb_to_linear(c[0]), style::srgb_to_linear(c[1]), style::srgb_to_linear(c[2])];
    let mix = |a: [f32; 3], b: [f32; 3], t: f32| [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t];
    let hex = |s: &str| lin(castor_core::options::parse_hex_color(s).unwrap_or([0, 0, 0, 255]));
    match name {
        "transparent" => RgbaImage::new(w, h),
        "paper" => px(&|x, y, gx, gy| {
            let g = (hash2(gx, gy, 7) - 0.5) * 0.015 + n.fbm(x * 6.0, y * 6.0, 0.0, 3) * 0.01;
            let c = hex("#fcfbf9");
            [c[0] + g, c[1] + g, c[2] + g]
        }),
        "night" => px(&|x, y, _, _| mix(hex("#161514"), hex("#2a2826"), smooth(0.0, 1.4, 1.2 - y - x * 0.2))),
        "tint" => px(&|_, y, _, _| mix(hex("#ffeca4"), hex("#ffdcc8"), y.min(1.0))),
        "custom" => px(&|_, _, _, _| lin(custom)),
        "wood" | "walnut" => {
            let (a, b) = if name == "wood" { (hex("#c49a6c"), hex("#8a5a33")) } else { (hex("#6b4329"), hex("#3a2215")) };
            px(&move |x, y, _, _| {
                let warp = n.fbm(x * 0.8, y * 3.0, 0.0, 3) * 1.2;
                let rings = ((y * 9.0 + warp * 4.0 + x * 0.3).sin() * 0.5 + 0.5).powf(2.0);
                let streak = n.fbm(x * 0.6, y * 40.0, 2.0, 4) * 0.5 + 0.5;
                mix(a, b, (rings * 0.55 + streak * 0.45).clamp(0.0, 1.0))
            })
        }
        "concrete" => px(&|x, y, gx, gy| {
            let base = 0.5 + 0.5 * n.fbm(x * 2.0, y * 2.0, 1.0, 6);
            let speck = if hash2(gx, gy, 3) > 0.985 { -0.08 } else { 0.0 };
            mix(hex("#9a9893"), hex("#c9c6bf"), base) .map(|c| c + speck)
        }),
        "marble" => px(&|x, y, _, _| {
            let t = n.fbm(x * 1.4, y * 1.4, 3.0, 6) * 4.0;
            let vein = ((x * 2.0 + y * 3.0 + t).sin()).abs();
            let v = smooth(0.0, 0.12, vein);
            let base = mix(hex("#f4f2ee"), hex("#ffffff"), 0.5 + 0.5 * n.get(x * 3.0, y * 3.0, 9.0));
            mix(hex("#8c8a84"), base, 0.35 + 0.65 * v)
        }),
        "linen" => px(&|x, y, gx, gy| {
            let weave = ((gx as f32 * 1.7).sin() * (gy as f32 * 1.7).cos()) * 0.02 + (hash2(gx, gy, 5) - 0.5) * 0.03;
            let c = mix(hex("#e9e2d4"), hex("#d8cfbd"), 0.5 + 0.5 * n.fbm(x * 3.0, y * 3.0, 0.0, 3));
            c.map(|v| v + weave)
        }),
        "felt" => px(&|x, y, gx, gy| {
            let c = mix(hex("#2f4f3f"), hex("#3c6350"), 0.5 + 0.5 * n.fbm(x * 4.0, y * 4.0, 0.0, 4));
            let f = (hash2(gx, gy, 11) - 0.5) * 0.05;
            c.map(|v| v + f)
        }),
        "terrazzo" => {
            let mut rng = crate::common::Rng::new(seed);
            let chips: Vec<(f32, f32, f32, [f32; 3])> = (0..((w * h) / 2500).min(4000))
                .map(|i| {
                    let col = ["#ffeca4", "#ade1ff", "#fbe2ff", "#5c5a55", "#ffdcc8"][i as usize % 5];
                    (rng.range(0.0, w as f32 / short), rng.range(0.0, h as f32 / short), rng.range(0.004, 0.014), hex(col))
                })
                .collect();
            let base = hex("#f1eee8");
            // Grid buckets for speed.
            let cell = 0.05f32;
            let gw = ((w as f32 / short) / cell) as usize + 2;
            let gh = ((h as f32 / short) / cell) as usize + 2;
            let mut grid: Vec<Vec<usize>> = vec![Vec::new(); gw * gh];
            for (i, c) in chips.iter().enumerate() {
                let (cx, cy) = ((c.0 / cell) as usize, (c.1 / cell) as usize);
                grid[cy.min(gh - 1) * gw + cx.min(gw - 1)].push(i);
            }
            px(&move |x, y, _, _| {
                let (cx, cy) = ((x / cell) as usize, (y / cell) as usize);
                for dy in 0..3 {
                    for dx in 0..3 {
                        let (gx, gy) = ((cx + dx).saturating_sub(1), (cy + dy).saturating_sub(1));
                        if gx >= gw || gy >= gh {
                            continue;
                        }
                        for &i in &grid[gy * gw + gx] {
                            let c = &chips[i];
                            let d = ((x - c.0).powi(2) * 1.3 + (y - c.1).powi(2)).sqrt() + n.get(x * 80.0, y * 80.0, i as f32) * 0.003;
                            if d < c.2 {
                                return c.3;
                            }
                        }
                    }
                }
                base
            })
        }
        "desk" => px(&|x, y, _, _| {
            // Light oak desk seen from above.
            let warp = n.fbm(x * 0.5, y * 4.0, 0.0, 3);
            let g = (n.fbm(x * 0.4 + warp, y * 30.0, 1.0, 4) * 0.5 + 0.5).powf(1.4);
            mix(hex("#e6cfa8"), hex("#c9a571"), g)
        }),
        "studio" => px(&|x, y, _, _| {
            // Seamless studio sweep: light top, soft floor shadow.
            mix(hex("#f4f2ee"), hex("#d9d6cf"), smooth(0.2, 1.4, y + 0.1 * x))
        }),
        _ => px(&|_, _, _, _| hex("#1b1616")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn homography_maps_corners() {
        let q = [[10.0, 5.0], [90.0, 12.0], [85.0, 70.0], [12.0, 80.0]];
        let m = square_to_quad(q);
        let map = |u: f64, v: f64| {
            let z = m[6] * u + m[7] * v + m[8];
            [(m[0] * u + m[1] * v + m[2]) / z, (m[3] * u + m[4] * v + m[5]) / z]
        };
        for (i, (u, v)) in [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)].into_iter().enumerate() {
            let p = map(u, v);
            assert!((p[0] - q[i][0]).abs() < 1e-6 && (p[1] - q[i][1]).abs() < 1e-6);
        }
    }
    #[test]
    fn every_scene_renders() {
        for s in SCENES {
            let img = scene(s, 64, 40, 1, [200, 100, 50, 255]);
            assert_eq!(img.dimensions(), (64, 40));
        }
    }
}
