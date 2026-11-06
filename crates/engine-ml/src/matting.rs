//! Matte refinement and foreground colour estimation, at full resolution with bounded memory.
//!
//! The heavy work happens on a working copy of at most [`WORK`] pixels on the long side; the
//! full-size result is computed row by row from bilinear samples of the low-resolution
//! coefficients (fast guided filter, He & Sun 2015; blur-fusion foreground estimation,
//! Forte & Pitié 2021).

use anyhow::Result;
use castor_core::RunCx;
use fast_image_resize as fir;
use image::{GrayImage, RgbImage, RgbaImage};
use rayon::prelude::*;

const WORK: u32 = 1536;

/// A single-channel f32 plane.
#[derive(Clone)]
pub struct Plane {
    pub w: usize,
    pub h: usize,
    pub v: Vec<f32>,
}

impl Plane {
    fn new(w: usize, h: usize) -> Plane {
        Plane { w, h, v: vec![0.0; w * h] }
    }
    fn map2(&self, o: &Plane, f: impl Fn(f32, f32) -> f32 + Sync) -> Plane {
        Plane { w: self.w, h: self.h, v: self.v.par_iter().zip(&o.v).map(|(a, b)| f(*a, *b)).collect() }
    }
    /// Mean over a (2r+1)² window, edges normalised by the number of pixels in the window.
    pub fn box_mean(&self, r: usize) -> Plane {
        let (w, h) = (self.w, self.h);
        let mut tmp = Plane::new(w, h);
        tmp.v.par_chunks_mut(w).enumerate().for_each(|(y, row)| {
            let src = &self.v[y * w..(y + 1) * w];
            let mut acc = 0f64;
            let mut cnt = 0usize;
            for x in 0..r.min(w) {
                acc += src[x] as f64;
                cnt += 1;
            }
            for x in 0..w {
                if x + r < w {
                    acc += src[x + r] as f64;
                    cnt += 1;
                }
                if x > r {
                    acc -= src[x - r - 1] as f64;
                    cnt -= 1;
                }
                row[x] = (acc / cnt as f64) as f32;
            }
        });
        let mut out = Plane::new(w, h);
        // Vertical pass, column blocks in parallel.
        let cols: Vec<Vec<f32>> = (0..w)
            .into_par_iter()
            .map(|x| {
                let mut col = vec![0f32; h];
                let mut acc = 0f64;
                let mut cnt = 0usize;
                for y in 0..r.min(h) {
                    acc += tmp.v[y * w + x] as f64;
                    cnt += 1;
                }
                for y in 0..h {
                    if y + r < h {
                        acc += tmp.v[(y + r) * w + x] as f64;
                        cnt += 1;
                    }
                    if y > r {
                        acc -= tmp.v[(y - r - 1) * w + x] as f64;
                        cnt -= 1;
                    }
                    col[y] = (acc / cnt as f64) as f32;
                }
                col
            })
            .collect();
        for (x, col) in cols.into_iter().enumerate() {
            for (y, v) in col.into_iter().enumerate() {
                out.v[y * w + x] = v;
            }
        }
        out
    }
    /// Bilinear sample at continuous coordinates (pixel centres at i + 0.5).
    #[inline]
    fn sample(&self, x: f32, y: f32) -> f32 {
        let fx = (x - 0.5).clamp(0.0, (self.w - 1) as f32);
        let fy = (y - 0.5).clamp(0.0, (self.h - 1) as f32);
        let (x0, y0) = (fx as usize, fy as usize);
        let (x1, y1) = ((x0 + 1).min(self.w - 1), (y0 + 1).min(self.h - 1));
        let (tx, ty) = (fx - x0 as f32, fy - y0 as f32);
        let a = self.v[y0 * self.w + x0] * (1.0 - tx) + self.v[y0 * self.w + x1] * tx;
        let b = self.v[y1 * self.w + x0] * (1.0 - tx) + self.v[y1 * self.w + x1] * tx;
        a * (1.0 - ty) + b * ty
    }
    fn from_gray(g: &GrayImage) -> Plane {
        Plane { w: g.width() as usize, h: g.height() as usize, v: g.as_raw().iter().map(|v| *v as f32 / 255.0).collect() }
    }
}

fn work_size(w: u32, h: u32) -> (u32, u32) {
    let s = (WORK as f32 / w.max(h) as f32).min(1.0);
    (((w as f32 * s).round() as u32).max(1), ((h as f32 * s).round() as u32).max(1))
}

fn luma(p: &[u8]) -> f32 {
    (0.299 * p[0] as f32 + 0.587 * p[1] as f32 + 0.114 * p[2] as f32) / 255.0
}

fn resize_rgb(img: &RgbImage, w: u32, h: u32) -> RgbImage {
    crate::util::resize_rgb(img, w, h)
}

/// Upsample a low-resolution matte to the size of `rgb`, snapping its edges to the photo with
/// a guided filter. Small detached regions are kept.
pub fn refine(rgb: &RgbImage, small: &GrayImage, cx: &RunCx) -> Result<GrayImage> {
    let (w, h) = rgb.dimensions();
    let (lw, lh) = work_size(w, h);
    let guide_img = resize_rgb(rgb, lw, lh);
    let i_low = Plane { w: lw as usize, h: lh as usize, v: guide_img.pixels().map(|p| luma(&p.0)).collect() };
    drop(guide_img);
    let p_low = Plane::from_gray(&crate::util::resize_gray(small, lw, lh, fir::FilterType::CatmullRom));
    cx.check_cancel()?;

    // Radius: the blur of the 1024-px prediction, in working pixels, plus a little.
    let r = ((lw.max(lh) as f32 / 1024.0) * 2.0 + 2.0).round() as usize;
    let eps = 1e-3f32;
    let mean_i = i_low.box_mean(r);
    let mean_p = p_low.box_mean(r);
    let corr_i = i_low.map2(&i_low, |a, b| a * b).box_mean(r);
    let corr_ip = i_low.map2(&p_low, |a, b| a * b).box_mean(r);
    let var_i = corr_i.map2(&mean_i, |c, m| c - m * m);
    let cov_ip = corr_ip.map2(&mean_i.map2(&mean_p, |a, b| a * b), |c, m| c - m);
    let a = cov_ip.map2(&var_i, |c, v| c / (v + eps));
    let b = mean_p.map2(&a.map2(&mean_i, |a, m| a * m), |p, am| p - am);
    let mean_a = a.box_mean(r);
    let mean_b = b.box_mean(r);
    drop((mean_i, mean_p, corr_i, corr_ip, var_i, cov_ip, a, b, i_low, p_low));
    cx.check_cancel()?;

    let small_p = Plane::from_gray(small);
    let (sx, sy) = (lw as f32 / w as f32, lh as f32 / h as f32);
    let (mx, my) = (small_p.w as f32 / w as f32, small_p.h as f32 / h as f32);
    let mut out = GrayImage::new(w, h);
    let raw = rgb.as_raw();
    out.par_chunks_mut(w as usize).enumerate().for_each(|(y, row)| {
        let fy = y as f32 + 0.5;
        for (x, o) in row.iter_mut().enumerate() {
            let fx = x as f32 + 0.5;
            let k = (y * w as usize + x) * 3;
            let i = luma(&raw[k..k + 3]);
            let base = small_p.sample(fx * mx, fy * my);
            let q = mean_a.sample(fx * sx, fy * sy) * i + mean_b.sample(fx * sx, fy * sy);
            // Trust the model where it is confident; the guided result only shapes the edges.
            let conf = ((base - 0.5).abs() * 2.0).powi(4);
            let v = conf * base + (1.0 - conf) * q;
            let v = if v < 0.015 { 0.0 } else if v > 0.985 { 1.0 } else { v };
            *o = (v.clamp(0.0, 1.0) * 255.0).round() as u8;
        }
    });
    Ok(out)
}

/// Foreground colours for semi-transparent pixels, so that edges carry no fringe of the old
/// background ("colour decontamination"). Opaque pixels keep their colour.
pub fn decontaminate(rgba: &mut RgbaImage, alpha: &GrayImage) {
    let (w, h) = rgba.dimensions();
    let (lw, lh) = work_size(w, h);
    let k = lw as f32 / w as f32;
    let rgb_full = crate::util::rgba_to_rgb_over(rgba, [0, 0, 0]);
    let img = resize_rgb(&rgb_full, lw, lh);
    drop(rgb_full);
    let a = Plane::from_gray(&crate::util::resize_gray(alpha, lw, lh, fir::FilterType::Bilinear));
    let chans: Vec<Plane> =
        (0..3).map(|c| Plane { w: lw as usize, h: lh as usize, v: img.pixels().map(|p| p[c] as f32 / 255.0).collect() }).collect();
    let mut f = chans.clone();
    let mut b = chans.clone();
    let mut last: Option<(Vec<Plane>, Vec<Plane>)> = None;
    for r in [90.0f32, 6.0] {
        let r = ((r * k).round() as usize).max(1);
        let ba = a.box_mean(r);
        let one_minus = Plane { w: a.w, h: a.h, v: a.v.iter().map(|v| 1.0 - v).collect() };
        let mut bfa = Vec::new();
        let mut bba = Vec::new();
        for c in 0..3 {
            let fa = f[c].map2(&a, |x, y| x * y).box_mean(r).map2(&ba, |n, d| n / (d + 1e-5));
            let bb = b[c].map2(&one_minus, |x, y| x * y).box_mean(r).map2(&ba, |n, d| n / (1.0 - d + 1e-5));
            bfa.push(fa);
            bba.push(bb);
        }
        for c in 0..3 {
            for i in 0..a.v.len() {
                let al = a.v[i];
                let resid = chans[c].v[i] - al * bfa[c].v[i] - (1.0 - al) * bba[c].v[i];
                f[c].v[i] = (bfa[c].v[i] + al * resid).clamp(0.0, 1.0);
                b[c].v[i] = (bba[c].v[i] + (1.0 - al) * resid).clamp(0.0, 1.0);
            }
        }
        last = Some((bfa, bba));
    }
    let Some((bfa, bba)) = last else { return };
    let (sx, sy) = (lw as f32 / w as f32, lh as f32 / h as f32);
    let araw = alpha.as_raw();
    rgba.par_chunks_mut(w as usize * 4).enumerate().for_each(|(y, row)| {
        let fy = y as f32 + 0.5;
        for x in 0..w as usize {
            let al = araw[y * w as usize + x] as f32 / 255.0;
            let px = &mut row[x * 4..x * 4 + 4];
            if al <= 0.0 || al >= 0.999 {
                continue;
            }
            let fx = x as f32 + 0.5;
            for c in 0..3 {
                let fa = bfa[c].sample(fx * sx, fy * sy);
                let bb = bba[c].sample(fx * sx, fy * sy);
                let i = px[c] as f32 / 255.0;
                let resid = i - al * fa - (1.0 - al) * bb;
                px[c] = ((fa + al * resid).clamp(0.0, 1.0) * 255.0).round() as u8;
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn box_mean_constant() {
        let p = Plane { w: 7, h: 5, v: vec![0.5; 35] };
        let m = p.box_mean(2);
        assert!(m.v.iter().all(|v| (v - 0.5).abs() < 1e-6));
    }
    #[test]
    fn box_mean_step() {
        let mut p = Plane::new(10, 1);
        p.v[5] = 1.0;
        let m = p.box_mean(1);
        assert!((m.v[4] - 1.0 / 3.0).abs() < 1e-6);
        assert!((m.v[5] - 1.0 / 3.0).abs() < 1e-6);
        assert_eq!(m.v[7], 0.0);
    }
}
