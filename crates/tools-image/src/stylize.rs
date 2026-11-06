//! image.stylize — looks: film, vintage, noir, duotone, halftone, sketch, posterize, cool, warm,
//! fade, pixel. Each with an intensity.

use anyhow::Result;
use castor_core::options::{Choice, Cond, OptionSpec, Values};
use castor_core::{FileInfo, Job, Output, Preview, RunCx, Tool, ToolMeta, ToolStatus};
use image::{DynamicImage, Rgba, RgbaImage};
use rayon::prelude::*;

use crate::codec::{DecodeOpts, Fmt};
use crate::{common, ops};

pub struct Stylize {
    meta: ToolMeta,
}

pub const LOOKS: &[&str] = &["film", "vintage", "noir", "duotone", "halftone", "sketch", "posterize", "cool", "warm", "fade", "pixel"];

impl Stylize {
    pub fn new() -> Self {
        Stylize {
            meta: common::meta(
                "image.stylize",
                "JPG · PNG · WEBP",
                &["filter", "effect", "look", "film", "vintage", "black and white", "duotone", "halftone", "sketch", "retro"],
                "styled",
                ToolStatus::Ready,
                &[],
                false,
            ),
        }
    }
}

impl Tool for Stylize {
    fn meta(&self) -> &ToolMeta {
        &self.meta
    }

    fn options(&self) -> Vec<OptionSpec> {
        vec![
            OptionSpec::choice("look", LOOKS.iter().map(|l| Choice::key(l)).collect(), "film"),
            OptionSpec::int("intensity", 0, 100, 80).slider().unit("%"),
            OptionSpec::color("dark", "#1b1f6b").when(Cond::Eq("look", "duotone")),
            OptionSpec::color("light", "#ffcf6e").when(Cond::Eq("look", "duotone")),
            OptionSpec::int("levels", 2, 16, 5).when(Cond::Eq("look", "posterize")),
            OptionSpec::int("cell", 2, 100, 10).unit("px").when(Cond::Eq("look", "halftone")),
            common::format_option("keep"),
        ]
    }

    fn run(&self, job: &Job, v: &Values, cx: &RunCx) -> Result<Vec<Output>> {
        let input = job.input()?;
        let d = common::load(input, DecodeOpts::default())?;
        cx.progress(0.3);
        let out_img = apply(&d.image.to_rgba8(), v, 1.0);
        cx.progress(0.8);
        let fmt = common::out_format(v, d.format, crate::codec::uses_alpha(&d.image));
        let q = if fmt == Fmt::Jpeg && d.format == Fmt::Jpeg { None } else { Some(90) };
        let o = common::resave_opts(&d, &input.path, q, true);
        let out = common::save(&DynamicImage::ImageRgba8(out_img), fmt, &o, Some(input), cx)?;
        Ok(vec![Output::new(out)])
    }

    fn preview(&self, input: Option<&FileInfo>, v: &Values, max_side: u32) -> Result<Option<Preview>> {
        let Some(f) = input else { return Ok(None) };
        let full_w = crate::codec::decode(&f.path, DecodeOpts::default()).map(|d| d.image.width()).unwrap_or(1);
        let img = common::load_preview(f, max_side)?.to_rgba8();
        // Pixel sizes (halftone cell, pixel look) scale with the preview.
        let k = img.width() as f32 / full_w.max(1) as f32;
        common::preview(&apply(&img, v, k), max_side, None)
    }
}

#[inline]
fn lum(p: &[u8]) -> f32 {
    0.2126 * p[0] as f32 + 0.7152 * p[1] as f32 + 0.0722 * p[2] as f32
}

fn mix(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

fn per_pixel(img: &RgbaImage, f: impl Fn(u32, u32, [f32; 3]) -> [f32; 3] + Sync) -> RgbaImage {
    let mut out = img.clone();
    let w = img.width();
    out.par_chunks_mut(w as usize * 4).enumerate().for_each(|(y, row)| {
        for x in 0..w as usize {
            let p = &mut row[x * 4..x * 4 + 4];
            let r = f(x as u32, y as u32, [p[0] as f32, p[1] as f32, p[2] as f32]);
            for c in 0..3 {
                p[c] = r[c].round().clamp(0.0, 255.0) as u8;
            }
        }
    });
    out
}

fn grain(x: u32, y: u32) -> f32 {
    let mut n = x.wrapping_mul(0x27d4_eb2d) ^ y.wrapping_mul(0x1656_67b1);
    n = (n ^ (n >> 15)).wrapping_mul(0x85eb_ca6b);
    ((n ^ (n >> 13)) & 0xffff) as f32 / 32_767.5 - 1.0
}

/// `k` scales pixel-sized parameters (previews are smaller than the file).
pub fn apply(img: &RgbaImage, v: &Values, k: f32) -> RgbaImage {
    let t = v.int("intensity").clamp(0, 100) as f32 / 100.0;
    let (w, h) = img.dimensions();
    let styled = match v.str("look") {
        "film" => per_pixel(img, |x, y, c| {
            // Lifted blacks, warm highlights, slight S-curve, grain.
            let l = lum(&[c[0] as u8, c[1] as u8, c[2] as u8]) / 255.0;
            let s = |v: f32| {
                let v = v / 255.0;
                let v = v * v * (3.0 - 2.0 * v);
                (0.06 + v * 0.9) * 255.0
            };
            let g = grain(x, y) * 7.0;
            [s(c[0]) + 6.0 * l + g, s(c[1]) + 2.0 + g, s(c[2]) - 8.0 * l + 6.0 * (1.0 - l) + g]
        }),
        "vintage" => per_pixel(img, |x, y, c| {
            let sep = [
                0.393 * c[0] + 0.769 * c[1] + 0.189 * c[2],
                0.349 * c[0] + 0.686 * c[1] + 0.168 * c[2],
                0.272 * c[0] + 0.534 * c[1] + 0.131 * c[2],
            ];
            let dx = x as f32 / w as f32 - 0.5;
            let dy = y as f32 / h as f32 - 0.5;
            let vig = 1.0 - (dx * dx + dy * dy) * 1.1;
            let g = grain(x, y) * 5.0;
            [(sep[0] * 0.85 + 30.0) * vig + g, (sep[1] * 0.85 + 22.0) * vig + g, (sep[2] * 0.8 + 18.0) * vig + g]
        }),
        "noir" => per_pixel(img, |x, y, c| {
            let l = (0.3 * c[0] + 0.59 * c[1] + 0.11 * c[2]) / 255.0;
            let l = ((l - 0.5) * 1.45 + 0.5).clamp(0.0, 1.0);
            let dx = x as f32 / w as f32 - 0.5;
            let dy = y as f32 / h as f32 - 0.5;
            let v = l * (1.0 - (dx * dx + dy * dy) * 0.9) * 255.0 + grain(x, y) * 4.0;
            [v, v, v]
        }),
        "duotone" => {
            let a = v.color("dark");
            let b = v.color("light");
            per_pixel(img, move |_, _, c| {
                let l = lum(&[c[0] as u8, c[1] as u8, c[2] as u8]) / 255.0;
                [mix(a[0] as f32, b[0] as f32, l), mix(a[1] as f32, b[1] as f32, l), mix(a[2] as f32, b[2] as f32, l)]
            })
        }
        "posterize" => {
            let n = v.int("levels").clamp(2, 32) as f32;
            per_pixel(img, move |_, _, c| c.map(|x| ((x / 255.0 * (n - 1.0)).round() / (n - 1.0)) * 255.0))
        }
        "cool" => per_pixel(img, |_, _, c| [c[0] * 0.9, c[1] * 0.98 + 4.0, c[2] * 1.08 + 10.0]),
        "warm" => per_pixel(img, |_, _, c| [c[0] * 1.08 + 10.0, c[1] * 1.0 + 3.0, c[2] * 0.88]),
        "fade" => per_pixel(img, |_, _, c| {
            let l = lum(&[c[0] as u8, c[1] as u8, c[2] as u8]);
            c.map(|x| mix(x, l, 0.35) * 0.82 + 40.0)
        }),
        "sketch" => sketch(img),
        "halftone" => halftone(img, ((v.int("cell").max(2) as f32) * k).max(2.0)),
        "pixel" => {
            let mut o = img.clone();
            let block = ((w.max(h) as f32 / 64.0) * (0.3 + t * 1.4)).max(2.0) as u32;
            ops::pixelate_region(&mut o, 0, 0, w, h, block);
            return o;
        }
        _ => img.clone(),
    };
    // Intensity: blend with the original (alpha kept from the original).
    let mut out = styled;
    if t < 0.999 {
        for (o, p) in out.pixels_mut().zip(img.pixels()) {
            for c in 0..3 {
                o[c] = mix(p[c] as f32, o[c] as f32, t).round() as u8;
            }
        }
    }
    out
}

/// Pencil sketch: colour dodge of the greyscale by its blurred negative.
fn sketch(img: &RgbaImage) -> RgbaImage {
    let (w, h) = img.dimensions();
    let gray = image::GrayImage::from_fn(w, h, |x, y| image::Luma([lum(&img.get_pixel(x, y).0) as u8]));
    let inv = image::GrayImage::from_fn(w, h, |x, y| image::Luma([255 - gray.get_pixel(x, y)[0]]));
    let blur = ops::blur_gray(&inv, (w.max(h) as f32 / 250.0).max(1.5));
    let mut out = img.clone();
    for (x, y, p) in out.enumerate_pixels_mut() {
        let g = gray.get_pixel(x, y)[0] as f32;
        let b = blur.get_pixel(x, y)[0] as f32;
        let v = if b >= 255.0 { 255.0 } else { (g * 255.0 / (255.0 - b)).min(255.0) };
        let v = (v / 255.0).powf(1.6) * 255.0;
        *p = Rgba([v as u8, v as u8, v as u8, p[3]]);
    }
    out
}

/// Black dots on white, size by darkness, on a 45° grid.
fn halftone(img: &RgbaImage, cell: f32) -> RgbaImage {
    let (w, h) = img.dimensions();
    let mut out = RgbaImage::from_fn(w, h, |x, y| Rgba([255, 255, 255, img.get_pixel(x, y)[3]]));
    let (s, c) = (std::f32::consts::FRAC_PI_4.sin(), std::f32::consts::FRAC_PI_4.cos());
    let diag = (w as f32).hypot(h as f32);
    let n = (diag / cell).ceil() as i32 + 2;
    for i in -n..n {
        for j in -n..n {
            let gx = (i as f32 + 0.5) * cell;
            let gy = (j as f32 + 0.5) * cell;
            let x = gx * c - gy * s + w as f32 / 2.0;
            let y = gx * s + gy * c + h as f32 / 2.0;
            if x < -cell || y < -cell || x > w as f32 + cell || y > h as f32 + cell {
                continue;
            }
            let sx = (x as u32).min(w - 1);
            let sy = (y as u32).min(h - 1);
            let dark = 1.0 - lum(&img.get_pixel(sx, sy).0) / 255.0;
            let r = cell * 0.72 * dark.sqrt();
            if r > 0.3 {
                ops::fill_circle(&mut out, x, y, r, [20, 20, 20, 255]);
            }
        }
    }
    out
}
