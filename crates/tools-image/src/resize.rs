//! image.resize — fit / fill (crop with focal point or auto focus) / pad / exact / percent,
//! with social, marketplace and device presets.

use anyhow::Result;
use castor_core::options::{Anchor, Choice, Cond, OptionSpec, Values};
use castor_core::{FileInfo, Job, Output, Preview, RunCx, Tool, ToolMeta, ToolStatus};
use image::{DynamicImage, Rgba, RgbaImage};

use crate::codec::{DecodeOpts, Fmt};
use crate::{common, ops};

pub struct Resize {
    meta: ToolMeta,
}

/// (id, width, height). Ids are stable (chains).
pub const PRESETS: &[(&str, u32, u32)] = &[
    ("instagram-square", 1080, 1080),
    ("instagram-portrait", 1080, 1350),
    ("instagram-story", 1080, 1920),
    ("x-post", 1600, 900),
    ("x-header", 1500, 500),
    ("linkedin-post", 1200, 627),
    ("linkedin-cover", 1584, 396),
    ("facebook-post", 1200, 630),
    ("facebook-cover", 851, 315),
    ("youtube-thumbnail", 1280, 720),
    ("pinterest-pin", 1000, 1500),
    ("open-graph", 1200, 630),
    ("etsy-listing", 2000, 2000),
    ("amazon-product", 2000, 2000),
    ("shopify-product", 2048, 2048),
    ("app-store-iphone", 1290, 2796),
    ("app-store-ipad", 2048, 2732),
    ("play-store-phone", 1080, 1920),
    ("hd-1080p", 1920, 1080),
    ("uhd-4k", 3840, 2160),
    ("a4-300dpi", 2480, 3508),
    ("letter-300dpi", 2550, 3300),
];

impl Resize {
    pub fn new() -> Self {
        Resize {
            meta: common::meta(
                "image.resize",
                "JPG · PNG · WEBP",
                &["resize", "scale", "dimensions", "smaller", "bigger", "instagram", "thumbnail", "width", "height", "pixels"],
                "resized",
                ToolStatus::Ready,
                &[],
                false,
            ),
        }
    }
}

impl Tool for Resize {
    fn meta(&self) -> &ToolMeta {
        &self.meta
    }

    fn options(&self) -> Vec<OptionSpec> {
        let mut presets = vec![Choice::key("custom")];
        presets.extend(PRESETS.iter().map(|(id, _, _)| Choice::key(id)));
        vec![
            OptionSpec::choice("preset", presets, "custom"),
            OptionSpec::choice(
                "mode",
                vec![Choice::key("fit"), Choice::key("fill"), Choice::key("pad"), Choice::key("exact"), Choice::key("percent")],
                "fit",
            ),
            OptionSpec::size("size", 1920, 1080).when(Cond::Eq("preset", "custom")),
            OptionSpec::int("percent", 1, 1000, 50).unit("%").when(Cond::Eq("mode", "percent")),
            OptionSpec::toggle("auto_focus", true).when(Cond::Eq("mode", "fill")),
            OptionSpec::anchor("focus", Anchor::Center).when(Cond::False("auto_focus")),
            OptionSpec::color("background", "#ffffff").when(Cond::Eq("mode", "pad")),
            OptionSpec::toggle("enlarge", false),
            common::format_option("keep"),
            common::quality_option(88),
            OptionSpec::toggle("keep_metadata", true).advanced(),
        ]
    }

    fn run(&self, job: &Job, v: &Values, cx: &RunCx) -> Result<Vec<Output>> {
        let input = job.input()?;
        let d = common::load(input, DecodeOpts { svg_side: 0, ..Default::default() })?;
        cx.progress(0.3);
        let (w0, h0) = (d.image.width(), d.image.height());
        let img = apply(&d.image, v)?;
        cx.check_cancel()?;
        cx.progress(0.7);
        let alpha = crate::codec::uses_alpha(&img);
        let fmt = common::out_format(v, d.format, alpha);
        let q = if v.str("format") == "keep" && d.format == Fmt::Jpeg { None } else { Some(v.int("quality") as u8) };
        let o = common::resave_opts(&d, &input.path, q, v.bool("keep_metadata"));
        let out = common::save(&img, fmt, &o, Some(input), cx)?;
        let note = format!("{} → {}", common::dims(w0, h0), common::dims(img.width(), img.height()));
        Ok(vec![Output::new(out).with_note(note)])
    }

    fn preview(&self, input: Option<&FileInfo>, v: &Values, max_side: u32) -> Result<Option<Preview>> {
        let Some(f) = input else { return Ok(None) };
        let img = common::load_preview(f, max_side)?;
        let full = crate::codec::decode(&f.path, DecodeOpts::default()).map(|d| (d.image.width(), d.image.height())).ok();
        let (fw, fh) = full.unwrap_or((img.width(), img.height()));
        let (tw, th) = target_size(fw, fh, v);
        // Same geometry on the small copy.
        let out = apply_with_target(&img, v, scale_target(tw, th, fw, img.width()))?;
        common::preview(&out.to_rgba8(), max_side, Some(common::dims(tw, th)))
    }
}

fn scale_target(tw: u32, th: u32, full_w: u32, small_w: u32) -> (u32, u32) {
    let s = small_w as f64 / full_w.max(1) as f64;
    (((tw as f64 * s).round() as u32).max(1), ((th as f64 * s).round() as u32).max(1))
}

fn requested(v: &Values) -> (u32, u32) {
    let p = v.str("preset");
    if let Some((_, w, h)) = PRESETS.iter().find(|(id, _, _)| *id == p) {
        return (*w, *h);
    }
    let (w, h) = v.size("size");
    (w, h)
}

/// Final pixel size for an input of (w, h).
pub fn target_size(w: u32, h: u32, v: &Values) -> (u32, u32) {
    let (rw, rh) = requested(v);
    let enlarge = v.bool("enlarge");
    match v.str("mode") {
        "percent" => {
            let p = v.int("percent").max(1) as f64 / 100.0;
            (((w as f64 * p).round() as u32).max(1), ((h as f64 * p).round() as u32).max(1))
        }
        "exact" | "fill" | "pad" => (rw.max(1), rh.max(1)),
        _ => {
            // fit: width or height may be 0 = free.
            let (bw, bh) = (if rw == 0 { u32::MAX } else { rw }, if rh == 0 { u32::MAX } else { rh });
            if !enlarge && w <= bw && h <= bh {
                return (w, h);
            }
            ops::fit_size(w, h, bw.min(1 << 20), bh.min(1 << 20))
        }
    }
}

pub fn apply(img: &DynamicImage, v: &Values) -> Result<DynamicImage> {
    let t = target_size(img.width(), img.height(), v);
    apply_with_target(img, v, t)
}

fn apply_with_target(img: &DynamicImage, v: &Values, (tw, th): (u32, u32)) -> Result<DynamicImage> {
    anyhow::ensure!((tw as u64) * (th as u64) <= 400_000_000, "{}", castor_core::tr("image.err.too_big"));
    let (w, h) = (img.width(), img.height());
    Ok(match v.str("mode") {
        "fill" => {
            // Scale to cover, then crop around the focus point.
            let s = (tw as f64 / w as f64).max(th as f64 / h as f64);
            let (sw, sh) = (((w as f64 * s).ceil() as u32).max(tw), ((h as f64 * s).ceil() as u32).max(th));
            let (fx, fy) = if v.bool("auto_focus") {
                ops::saliency_center(img)
            } else {
                let (gx, gy) = v.anchor("focus").grid();
                (gx as f32 / 2.0, gy as f32 / 2.0)
            };
            let scaled = ops::resize_dyn(img, sw, sh);
            let x = ((fx * sw as f32 - tw as f32 / 2.0).round().max(0.0) as u32).min(sw - tw);
            let y = ((fy * sh as f32 - th as f32 / 2.0).round().max(0.0) as u32).min(sh - th);
            scaled.crop_imm(x, y, tw, th)
        }
        "pad" => {
            let (fw, fh) = ops::fit_size(w, h, tw, th);
            let (fw, fh) = if !v.bool("enlarge") && w <= tw && h <= th { (w, h) } else { (fw, fh) };
            let small = ops::resize_rgba(&img.to_rgba8(), fw, fh);
            let c = v.color("background");
            let mut canvas = RgbaImage::from_pixel(tw, th, Rgba(c));
            ops::overlay(&mut canvas, &small, ((tw - fw) / 2) as i64, ((th - fh) / 2) as i64, 1.0);
            DynamicImage::ImageRgba8(canvas)
        }
        _ => ops::resize_dyn(img, tw, th),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use castor_core::Value;
    fn vals(pairs: &[(&str, Value)]) -> Values {
        let mut v = Values::defaults(&Resize::new().options());
        for (k, x) in pairs {
            v.set(k, x.clone());
        }
        v
    }
    #[test]
    fn fit_never_enlarges_by_default() {
        let v = vals(&[("size", Value::Size(4000, 4000))]);
        assert_eq!(target_size(800, 600, &v), (800, 600));
    }
    #[test]
    fn fit_keeps_ratio() {
        let v = vals(&[("size", Value::Size(1000, 1000))]);
        assert_eq!(target_size(4000, 2000, &v), (1000, 500));
    }
    #[test]
    fn preset_fill() {
        let v = vals(&[("preset", Value::Text("instagram-portrait".into())), ("mode", Value::Text("fill".into()))]);
        let img = DynamicImage::ImageRgb8(image::RgbImage::new(3000, 2000));
        let out = apply(&img, &v).unwrap();
        assert_eq!((out.width(), out.height()), (1080, 1350));
    }
}
