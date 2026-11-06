//! image.compress — the best encoder per format, by quality, target size or "visually
//! identical" (DSSIM). Never returns a bigger file: the original is copied instead.

use anyhow::{Context, Result};
use castor_core::options::{Choice, Cond, OptionSpec, Values};
use castor_core::{Job, Output, RunCx, Tool, ToolMeta, ToolStatus};
use image::DynamicImage;

use crate::codec::{self, DecodeOpts, EncodeOpts, Fmt};
use crate::common;

/// DSSIM at or below this is not visible at 100 % zoom on photos and screenshots.
const IDENTICAL: f64 = 0.0012;

pub struct Compress {
    meta: ToolMeta,
}

impl Compress {
    pub fn new() -> Self {
        Compress {
            meta: common::meta(
                "image.compress",
                "JPG · PNG · WEBP · AVIF · GIF",
                &["compress", "optimize", "smaller", "reduce", "shrink", "tinypng", "squoosh", "size", "kb", "web"],
                "min",
                ToolStatus::Ready,
                &[],
                false,
            ),
        }
    }
}

impl Tool for Compress {
    fn meta(&self) -> &ToolMeta {
        &self.meta
    }

    fn options(&self) -> Vec<OptionSpec> {
        vec![
            OptionSpec::choice("mode", vec![Choice::key("quality"), Choice::key("identical"), Choice::key("size")], "quality"),
            common::quality_option(78).when(Cond::Eq("mode", "quality")),
            OptionSpec::int("target_kb", 5, 100_000, 300).unit("KB").when(Cond::Eq("mode", "size")),
            common::format_option("keep"),
            OptionSpec::toggle("png_palette", true),
            OptionSpec::toggle("keep_metadata", false),
        ]
    }

    fn run(&self, job: &Job, v: &Values, cx: &RunCx) -> Result<Vec<Output>> {
        let input = job.input()?;
        let src = std::fs::read(&input.path).with_context(|| common::err("err.read", &input.path))?;
        let in_fmt = codec::detect(&input.path).context(common::err("err.unknown_format", &input.path))?;
        // Animated GIF: gifski re-encode.
        if in_fmt == Fmt::Gif && matches!(v.str("format"), "keep" | "") {
            if let Some(frames) = codec::decode_frames(&src, Fmt::Gif)? {
                let q = match v.str("mode") {
                    "quality" => v.int("quality").clamp(1, 100) as u8,
                    _ => 80,
                };
                let bytes = codec::encode_gifski(frames, q, 0, None, cx.cancel_token())?;
                return finish(input, &src, bytes, Fmt::Gif, cx);
            }
        }
        let d = common::load(input, DecodeOpts { orient: false, frames: false, svg_side: 0 })?;
        cx.progress(0.1);
        let alpha = codec::uses_alpha(&d.image);
        let fmt = common::out_format(v, d.format, alpha);
        let keep_meta = v.bool("keep_metadata");
        let base = EncodeOpts {
            icc: d.icc.clone(),
            // Pixels were not rotated: keep orientation (minimal EXIF) unless all metadata is kept.
            exif: if keep_meta { d.exif.clone() } else { crate::meta::minimal_exif(Some(d.orientation), None, None) },
            xmp: if keep_meta { d.xmp.clone() } else { None },
            png_palette: v.bool("png_palette"),
            png_effort: 3,
            avif_speed: 6,
            ..Default::default()
        };
        let img = if fmt == Fmt::Jpeg && !matches!(d.image, DynamicImage::ImageLuma8(_)) {
            DynamicImage::ImageRgb8(codec::flatten(&d.image, [255, 255, 255]))
        } else {
            d.image
        };
        let bytes = match v.str("mode") {
            "size" => by_size(&img, fmt, &base, v.int("target_kb").max(1) as u64 * 1000, cx)?,
            "identical" => identical(&img, fmt, &base, cx)?,
            _ => {
                let o = EncodeOpts { quality: v.int("quality").clamp(1, 100) as u8, ..base };
                codec::encode(&img, fmt, &o)?
            }
        };
        finish(input, &src, bytes, fmt, cx)
    }
}

fn finish(input: &castor_core::FileInfo, src: &[u8], bytes: Vec<u8>, fmt: Fmt, cx: &RunCx) -> Result<Vec<Output>> {
    let same_format = codec::detect(&input.path) == Some(fmt);
    if same_format && bytes.len() as u64 >= src.len() as u64 {
        let out = cx.output_path(Some(input), &input.extension());
        common::write(&out, src)?;
        return Ok(vec![Output::new(out).with_note(castor_core::tr("image.compress.kept"))]);
    }
    let out = cx.output_path(Some(input), fmt.ext());
    common::write(&out, &bytes)?;
    Ok(vec![Output::new(out).with_note(common::size_note(src.len() as u64, bytes.len() as u64))])
}

fn lossy(fmt: Fmt, o: &EncodeOpts) -> bool {
    match fmt {
        Fmt::Jpeg | Fmt::Avif | Fmt::Gif => true,
        Fmt::Webp => !o.lossless,
        Fmt::Png => o.png_palette,
        _ => false,
    }
}

/// Lowest quality whose output fits in `target` bytes (binary search, 7 encodes at most).
fn by_size(img: &DynamicImage, fmt: Fmt, base: &EncodeOpts, target: u64, cx: &RunCx) -> Result<Vec<u8>> {
    if !lossy(fmt, base) {
        return codec::encode(img, fmt, base);
    }
    let (mut lo, mut hi) = (5u8, 95u8);
    let mut best: Option<Vec<u8>> = None;
    let mut smallest: Option<Vec<u8>> = None;
    for i in 0..7 {
        cx.check_cancel()?;
        let q = ((lo as u16 + hi as u16) / 2) as u8;
        let b = codec::encode(img, fmt, &EncodeOpts { quality: q, ..base.clone() })?;
        cx.progress(0.1 + 0.9 * (i + 1) as f32 / 7.0);
        if smallest.as_ref().is_none_or(|s| b.len() < s.len()) {
            smallest = Some(b.clone());
        }
        if b.len() as u64 <= target {
            best = Some(b);
            lo = q + 1;
        } else {
            hi = q.saturating_sub(1);
        }
        if lo > hi {
            break;
        }
    }
    best.or(smallest).context("encoding failed")
}

fn rgba_px(img: &DynamicImage) -> (Vec<rgb::RGBA8>, usize, usize) {
    let r = img.to_rgba8();
    let (w, h) = (r.width() as usize, r.height() as usize);
    (r.pixels().map(|p| rgb::RGBA8::new(p[0], p[1], p[2], p[3])).collect(), w, h)
}

/// Work copy for the similarity measure: big photos are compared at ≤ 4 MP (artifacts that
/// survive a 2× reduction are the visible ones; this keeps the search fast).
fn dssim_copy(img: &DynamicImage) -> DynamicImage {
    let (w, h) = (img.width(), img.height());
    if (w as u64 * h as u64) > 4_000_000 {
        let s = (4_000_000f64 / (w as f64 * h as f64)).sqrt();
        crate::ops::resize_dyn(img, ((w as f64 * s) as u32).max(1), ((h as f64 * s) as u32).max(1))
    } else {
        img.clone()
    }
}

/// Lowest quality that stays visually identical (DSSIM ≤ IDENTICAL).
fn identical(img: &DynamicImage, fmt: Fmt, base: &EncodeOpts, cx: &RunCx) -> Result<Vec<u8>> {
    if !lossy(fmt, base) {
        return codec::encode(img, fmt, base);
    }
    let attr = dssim_core::Dssim::new();
    let reference = dssim_copy(img);
    let (px, w, h) = rgba_px(&reference);
    let orig = attr.create_image_rgba(&px, w, h).context("dssim")?;
    drop(px);
    let score = |bytes: &[u8]| -> Result<f64> {
        let dec = image::load_from_memory(bytes)?;
        let dec = if dec.dimensions_eq(&reference) { dec } else { dssim_copy(&dec) };
        let (p, w2, h2) = rgba_px(&dec);
        let other = attr.create_image_rgba(&p, w2, h2).context("dssim")?;
        Ok(attr.compare(&orig, other).0.into())
    };
    let decodable = !matches!(fmt, Fmt::Avif);
    let (mut lo, mut hi) = (30u8, 95u8);
    let mut best: Option<Vec<u8>> = None;
    for i in 0..6 {
        cx.check_cancel()?;
        let q = ((lo as u16 + hi as u16) / 2) as u8;
        let b = codec::encode(img, fmt, &EncodeOpts { quality: q, ..base.clone() })?;
        cx.progress(0.1 + 0.9 * (i + 1) as f32 / 6.0);
        let ok = if decodable { score(&b)? <= IDENTICAL } else { q >= 70 };
        if ok {
            best = Some(b);
            hi = q.saturating_sub(1);
        } else {
            lo = q + 1;
        }
        if lo > hi {
            break;
        }
    }
    match best {
        Some(b) => Ok(b),
        // Nothing passed: PNG falls back to lossless, others to high quality.
        None if fmt == Fmt::Png => codec::encode(img, fmt, &EncodeOpts { png_palette: false, ..base.clone() }),
        None => codec::encode(img, fmt, &EncodeOpts { quality: 95, ..base.clone() }),
    }
}

trait DimsEq {
    fn dimensions_eq(&self, o: &DynamicImage) -> bool;
}
impl DimsEq for DynamicImage {
    fn dimensions_eq(&self, o: &DynamicImage) -> bool {
        self.width() == o.width() && self.height() == o.height()
    }
}
