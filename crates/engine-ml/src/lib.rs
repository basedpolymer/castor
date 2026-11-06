//! Castor ML engine: local ONNX models for background removal, upscaling and face detection.
//!
//! Models are downloaded on first use (see [`models`]) and cached per process: a batch loads a
//! model once and parallel jobs share it. Big images are processed in tiles (upscaling) or at
//! the model's resolution and refined at full size (mattes), so memory stays bounded.

pub mod backend;
pub mod matting;
pub mod models;
mod util;

use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use castor_core::RunCx;
use image::{GrayImage, RgbImage, RgbaImage};

pub use backend::{Backend, backend, unload_all};

/// Make sure a model is on disk (downloading it once, with progress), return its path.
pub fn ensure_model(id: &str, cx: &RunCx) -> Result<PathBuf> {
    let m = models::get(id).with_context(|| format!("unknown model '{id}'"))?;
    if m.asset.is_present() {
        return Ok(m.asset.path());
    }
    cx.message(castor_core::tr_args("ml.downloading", &[("name", m.name), ("size", &castor_core::units::bytes(m.asset.size))]));
    let p = castor_core::download::ensure(&m.asset, &cx.cancel_token(), &|done, total| {
        if total > 0 {
            cx.progress(done as f32 / total as f32 * 0.99);
        }
    })?;
    cx.progress(0.0);
    Ok(p)
}

/// Alpha matte of the foreground (255 = keep) at the size of `rgba`.
///
/// `model`: "isnet-general-use" (default, fast) or "birefnet-lite" (finer hair and edges).
/// The matte is predicted at 1024 × 1024 and then refined at full resolution with a guided
/// filter so edges follow the photo. Small detached parts are kept (no "largest blob" filter).
pub fn remove_background(rgba: &RgbaImage, model: &str, cx: &RunCx) -> Result<GrayImage> {
    let m = models::get(model).with_context(|| format!("unknown model '{model}'"))?;
    if m.task != models::Task::Matte {
        bail!("'{model}' is not a background-removal model");
    }
    let (w, h) = rgba.dimensions();
    if w == 0 || h == 0 {
        bail!("empty image");
    }
    let path = ensure_model(model, cx)?;
    cx.check_cancel()?;
    cx.progress(0.05);

    const S: u32 = 1024;
    let rgb = util::rgba_to_rgb_over(rgba, [255, 255, 255]);
    let small = util::resize_rgb(&rgb, S, S);
    let (mean, std, scale) = match model {
        // rembg / DIS: x / max(x) - mean, std 1.
        "isnet-general-use" => {
            let mx = small.as_raw().iter().copied().max().unwrap_or(255).max(1) as f32;
            ([0.485, 0.456, 0.406], [1.0, 1.0, 1.0], 1.0 / mx)
        }
        _ => ([0.485, 0.456, 0.406], [0.229, 0.224, 0.225], 1.0 / 255.0),
    };
    let input = util::to_nchw(&small, mean, std, scale, false);
    drop(small);
    let net = backend::load(&path, &input.shape, &[0])?;
    cx.progress(0.1);
    cx.check_cancel()?;
    let out = net.run(input)?.into_iter().next().context("model returned nothing")?;
    cx.check_cancel()?;
    cx.progress(0.8);

    let mut pred = out.data;
    // ISNet: min-max normalise; BiRefNet: logits or probabilities depending on the export.
    let (mn, mx) = pred.iter().fold((f32::MAX, f32::MIN), |(a, b), v| (a.min(*v), b.max(*v)));
    if model == "isnet-general-use" {
        let d = (mx - mn).max(1e-6);
        pred.iter_mut().for_each(|v| *v = (*v - mn) / d);
    } else if mn < -0.01 || mx > 1.01 {
        pred.iter_mut().for_each(|v| *v = 1.0 / (1.0 + (-*v).exp()));
    }
    let small_matte =
        GrayImage::from_raw(S, S, pred.iter().map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8).collect()).context("matte")?;
    let matte = matting::refine(&rgb, &small_matte, cx)?;
    cx.progress(1.0);
    Ok(matte)
}

/// Upscale by `scale` (2, 3 or 4) with a super-resolution model.
///
/// `model`: "realesrgan-x4plus" (photos), "realesrgan-x2plus" (photos, ×2 native),
/// "realesrgan-x4plus-anime6b" (illustration / anime), "realesr-general-x4v3" (fast).
/// Runs in tiles (constant size, reflected borders, overlapping margins), with progress per
/// tile and cancellation between tiles.
pub fn upscale(rgb: &RgbImage, scale: u32, model: &str, cx: &RunCx) -> Result<RgbImage> {
    let m = models::get(model).with_context(|| format!("unknown model '{model}'"))?;
    let models::Task::Upscale { native } = m.task else { bail!("'{model}' is not an upscaling model") };
    let scale = scale.clamp(1, 8);
    let (w, h) = rgb.dimensions();
    if w == 0 || h == 0 {
        bail!("empty image");
    }
    let out_px = (w as u64 * scale as u64) * (h as u64 * scale as u64);
    if out_px > MAX_OUTPUT_PIXELS {
        bail!("{}", castor_core::tr_args("ml.too_big", &[("mp", &format!("{}", out_px / 1_000_000))]));
    }
    let path = ensure_model(model, cx)?;
    cx.check_cancel()?;
    let up = upscale_native(rgb, native, &path, cx)?;
    let (tw, th) = (w * scale, h * scale);
    Ok(if up.dimensions() == (tw, th) { up } else { util::resize_rgb(&up, tw, th) })
}

/// 400 MP: ~1.2 GB of RGB output. Beyond that the result is not useful anyway.
pub const MAX_OUTPUT_PIXELS: u64 = 400_000_000;

fn upscale_native(rgb: &RgbImage, native: u32, path: &std::path::Path, cx: &RunCx) -> Result<RgbImage> {
    // Tiles split the image evenly (no wasted overlap), at most TILE px per side; every tile
    // has the same size so that backends compiling per shape (tract) compile once.
    let tile_max: u32 = if backend::backend() == backend::Backend::Tract { 256 } else { 384 };
    const PAD: u32 = 16;
    let (w, h) = rgb.dimensions();
    let (tw, th) = (w.div_ceil(w.div_ceil(tile_max)), h.div_ceil(h.div_ceil(tile_max)));
    let (iw, ih) = (tw + 2 * PAD, th + 2 * PAD);
    let shape = vec![1, 3, ih as usize, iw as usize];
    let net = backend::load(path, &shape, &[0])?;
    let cols = w.div_ceil(tw);
    let rows = h.div_ceil(th);
    let n = (cols * rows) as usize;
    let mut out = RgbImage::new(w * native, h * native);
    let tiles: Vec<(u32, u32)> = (0..rows).flat_map(|r| (0..cols).map(move |c| (c, r))).collect();

    let run_tile = |(c, r): (u32, u32)| -> Result<(u32, u32, u32, u32, Vec<f32>)> {
        // Tile origin, clamped so that the last tile is full-size (overlaps its neighbour).
        let x0 = (c * tw).min(w - tw);
        let y0 = (r * th).min(h - th);
        let mut data = vec![0f32; 3 * (iw * ih) as usize];
        let plane = (iw * ih) as usize;
        for yy in 0..ih {
            let sy = util::reflect(y0 as i64 + yy as i64 - PAD as i64, h);
            for xx in 0..iw {
                let sx = util::reflect(x0 as i64 + xx as i64 - PAD as i64, w);
                let p = rgb.get_pixel(sx, sy).0;
                let i = (yy * iw + xx) as usize;
                data[i] = p[0] as f32 / 255.0;
                data[plane + i] = p[1] as f32 / 255.0;
                data[2 * plane + i] = p[2] as f32 / 255.0;
            }
        }
        let o = net.run(backend::Tensor::new(shape.clone(), data))?.into_iter().next().context("model returned nothing")?;
        anyhow::ensure!(
            o.shape.len() == 4 && o.shape[1] == 3 && o.shape[2] == (ih * native) as usize && o.shape[3] == (iw * native) as usize,
            "unexpected model output shape {:?}",
            o.shape
        );
        Ok((x0, y0, c, r, o.data))
    };

    let mut write_tile = |(x0, y0, c, r, o): (u32, u32, u32, u32, Vec<f32>)| {
        let ow = (iw * native) as usize;
        let plane = ow * (ih * native) as usize;
        // Region of the output owned by this tile: [c*tw, min((c+1)*tw, w)) in input pixels.
        let (ox0, oy0) = (c * tw, r * th);
        let (ox1, oy1) = (((c + 1) * tw).min(w), ((r + 1) * th).min(h));
        for y in (oy0 * native)..(oy1 * native) {
            let ty = (y - y0 * native + PAD * native) as usize;
            for x in (ox0 * native)..(ox1 * native) {
                let tx = (x - x0 * native + PAD * native) as usize;
                let i = ty * ow + tx;
                let px = [o[i], o[plane + i], o[2 * plane + i]].map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8);
                out.put_pixel(x, y, image::Rgb(px));
            }
        }
    };

    if net.parallel_friendly() {
        use rayon::prelude::*;
        let par = rayon::current_num_threads().max(1);
        for (k, chunk) in tiles.chunks(par).enumerate() {
            cx.check_cancel()?;
            let res: Vec<_> = chunk.par_iter().map(|t| run_tile(*t)).collect();
            for r in res {
                write_tile(r?);
            }
            cx.progress(((k + 1) * par).min(n) as f32 / n as f32);
        }
    } else {
        for (i, t) in tiles.iter().enumerate() {
            cx.check_cancel()?;
            write_tile(run_tile(*t)?);
            cx.progress((i + 1) as f32 / n as f32);
        }
    }
    Ok(out)
}

/// Face boxes `[x0, y0, x1, y1]` in pixels (YuNet). Detects small faces in big photos by also
/// scanning overlapping tiles.
pub fn detect_faces(rgb: &RgbImage, cx: &RunCx) -> Result<Vec<[f32; 4]>> {
    let path = ensure_model("yunet", cx)?;
    let (w, h) = rgb.dimensions();
    if w == 0 || h == 0 {
        return Ok(Vec::new());
    }
    const N: u32 = 640;
    let net = backend::load(&path, &[1, 3, N as usize, N as usize], &(0..9).collect::<Vec<_>>())?;
    let side = w.max(h);
    // Windows: the whole image, then tiles of 1600 and 800 px (with 25 % overlap) on big photos.
    let mut windows = vec![(0u32, 0u32, w, h)];
    for t in [1600u32, 800] {
        if side > t + t / 4 {
            let step = t * 3 / 4;
            let mut y = 0;
            loop {
                let mut x = 0;
                let th = t.min(h);
                loop {
                    let tw = t.min(w);
                    windows.push((x.min(w - tw), y.min(h - th), tw, th));
                    if x + tw >= w {
                        break;
                    }
                    x += step;
                }
                if y + th >= h {
                    break;
                }
                y += step;
            }
        }
    }
    let mut dets: Vec<([f32; 4], f32)> = Vec::new();
    for (i, (x0, y0, ww, wh)) in windows.iter().copied().enumerate() {
        cx.check_cancel()?;
        let crop = image::imageops::crop_imm(rgb, x0, y0, ww, wh).to_image();
        let s = N as f32 / ww.max(wh) as f32;
        let (rw, rh) = (((ww as f32 * s).round() as u32).clamp(1, N), ((wh as f32 * s).round() as u32).clamp(1, N));
        let small = util::resize_rgb(&crop, rw, rh);
        drop(crop);
        // Letterbox into 640 × 640, BGR, 0..255.
        let plane = (N * N) as usize;
        let mut data = vec![0f32; 3 * plane];
        for (x, y, p) in small.enumerate_pixels() {
            let k = (y * N + x) as usize;
            data[k] = p[2] as f32;
            data[plane + k] = p[1] as f32;
            data[2 * plane + k] = p[0] as f32;
        }
        let outs = net.run(backend::Tensor::new(vec![1, 3, N as usize, N as usize], data))?;
        for (si, stride) in [8u32, 16, 32].into_iter().enumerate() {
            let (cls, obj, bbox) = (&outs[si].data, &outs[3 + si].data, &outs[6 + si].data);
            let cols = N / stride;
            for idx in 0..cls.len() {
                let score = (cls[idx].clamp(0.0, 1.0) * obj[idx].clamp(0.0, 1.0)).sqrt();
                if score < 0.6 {
                    continue;
                }
                let (r, c) = ((idx as u32 / cols) as f32, (idx as u32 % cols) as f32);
                let st = stride as f32;
                let cxp = (c + bbox[idx * 4]) * st;
                let cyp = (r + bbox[idx * 4 + 1]) * st;
                let bw = bbox[idx * 4 + 2].exp() * st;
                let bh = bbox[idx * 4 + 3].exp() * st;
                let b = [
                    x0 as f32 + (cxp - bw / 2.0) / s,
                    y0 as f32 + (cyp - bh / 2.0) / s,
                    x0 as f32 + (cxp + bw / 2.0) / s,
                    y0 as f32 + (cyp + bh / 2.0) / s,
                ];
                // Drop boxes cut by a tile border (the neighbouring tile sees them whole).
                let inner = i == 0
                    || ((b[0] > x0 as f32 + 2.0 || x0 == 0)
                        && (b[1] > y0 as f32 + 2.0 || y0 == 0)
                        && (b[2] < (x0 + ww) as f32 - 2.0 || x0 + ww == w)
                        && (b[3] < (y0 + wh) as f32 - 2.0 || y0 + wh == h));
                if inner {
                    dets.push((b.map(|v| v.max(0.0)), score));
                }
            }
        }
        cx.progress((i + 1) as f32 / windows.len() as f32);
    }
    Ok(util::nms(dets, 0.3).into_iter().map(|mut b| {
        b[2] = b[2].min(w as f32);
        b[3] = b[3].min(h as f32);
        b
    }).collect())
}
