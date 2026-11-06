//! image.remove-background and image.upscale (local ONNX models through engine-ml).

use anyhow::Result;
use castor_core::download::Asset;
use castor_core::options::{Choice, Cond, OptionSpec, Values};
use castor_core::{FileInfo, Job, Output, Preview, Requirement, RunCx, Tool, ToolMeta, ToolStatus};
use image::{DynamicImage, GrayImage, RgbaImage};

use crate::codec::{DecodeOpts, EncodeOpts, Fmt};
use crate::{common, ops};

// ------------------------------------------------------------------ remove background

pub struct RemoveBackground {
    meta: ToolMeta,
}

impl RemoveBackground {
    pub fn new() -> Self {
        RemoveBackground {
            meta: common::meta(
                "image.remove-background",
                "PNG · WEBP",
                &["background", "remove", "cutout", "transparent", "remove.bg", "subject", "isolate", "product", "erase"],
                "nobg",
                ToolStatus::Ready,
                &[Requirement::Model("birefnet-lite")],
                true,
            ),
        }
    }
}

fn bg_model(v: &Values) -> &'static str {
    match v.str("model") {
        "fast" => "isnet-general-use",
        _ => "birefnet-lite",
    }
}

impl Tool for RemoveBackground {
    fn meta(&self) -> &ToolMeta {
        &self.meta
    }

    fn options(&self) -> Vec<OptionSpec> {
        vec![
            OptionSpec::choice("model", vec![Choice::key("best"), Choice::key("fast")], "best"),
            OptionSpec::choice("background", vec![Choice::key("transparent"), Choice::key("color"), Choice::key("blur")], "transparent"),
            OptionSpec::color("color", "#ffffff").when(Cond::Eq("background", "color")),
            OptionSpec::int("blur", 2, 100, 20).slider().when(Cond::Eq("background", "blur")),
            OptionSpec::choice("format", vec![Choice::lit("png", "PNG"), Choice::lit("webp", "WebP"), Choice::lit("jpg", "JPG")], "png"),
            OptionSpec::toggle("crop", false),
            OptionSpec::int("padding", 0, 2000, 0).unit("px").when(Cond::True("crop")),
            OptionSpec::toggle("decontaminate", true).advanced(),
        ]
    }

    fn downloads(&self, v: &Values) -> Vec<Asset> {
        engine_ml::models::asset(bg_model(v)).into_iter().collect()
    }

    fn run(&self, job: &Job, v: &Values, cx: &RunCx) -> Result<Vec<Output>> {
        let input = job.input()?;
        let d = common::load(input, DecodeOpts::default())?;
        cx.check_cancel()?;
        let rgba = d.image.to_rgba8();
        drop(d.image);
        let sub = sub_cx(cx, 0.0, 0.85);
        let matte = engine_ml::remove_background(&rgba, bg_model(v), &sub)?;
        cx.check_cancel()?;
        let out_img = compose(rgba, &matte, v);
        cx.progress(0.92);
        let fmt = match v.str("format") {
            "webp" => Fmt::Webp,
            "jpg" => Fmt::Jpeg,
            _ => Fmt::Png,
        };
        let o = EncodeOpts {
            quality: 92,
            lossless: false,
            background: if v.str("background") == "color" { let c = v.color("color"); [c[0], c[1], c[2]] } else { [255, 255, 255] },
            icc: d.icc.clone(),
            ..Default::default()
        };
        let out = common::save(&DynamicImage::ImageRgba8(out_img), fmt, &o, Some(input), cx)?;
        Ok(vec![Output::new(out)])
    }

    fn preview(&self, input: Option<&FileInfo>, v: &Values, max_side: u32) -> Result<Option<Preview>> {
        // Only once the model is on disk: a preview never downloads anything.
        let Some(f) = input else { return Ok(None) };
        if !engine_ml::models::asset(bg_model(v)).is_some_and(|a| a.is_present()) {
            return Ok(None);
        }
        let img = common::load_preview(f, max_side)?.to_rgba8();
        let cx = RunCx::new(std::env::temp_dir(), std::env::temp_dir(), "", castor_core::CancelToken::new());
        let matte = engine_ml::remove_background(&img, bg_model(v), &cx)?;
        let mut out = compose(img, &matte, v);
        if v.str("background") == "transparent" {
            out = checkerboard_under(&out);
        }
        common::preview(&out, max_side, None)
    }
}

/// A RunCx whose progress maps into [a, b] of the parent's.
fn sub_cx(cx: &RunCx, a: f32, b: f32) -> RunCx {
    let parent = cx.clone();
    let p2 = cx.clone();
    RunCx::new(cx.out_dir().to_path_buf(), cx.tmp_dir().to_path_buf(), cx.suffix(), cx.cancel_token())
        .on_progress(move |f| parent.progress(a + (b - a) * f))
        .on_message(move |m| p2.message(m))
}

/// Apply the matte: transparent / colour / blurred background, optional crop to the subject.
fn compose(mut rgba: RgbaImage, matte: &GrayImage, v: &Values) -> RgbaImage {
    if v.bool("decontaminate") {
        engine_ml::matting::decontaminate(&mut rgba, matte);
    }
    let original = if v.str("background") == "blur" { Some(rgba.clone()) } else { None };
    for (p, a) in rgba.pixels_mut().zip(matte.pixels()) {
        p[3] = ((p[3] as u32 * a[0] as u32) / 255) as u8;
    }
    let mut out = match v.str("background") {
        "color" => {
            let c = v.color("color");
            let mut bg = RgbaImage::from_pixel(rgba.width(), rgba.height(), image::Rgba(c));
            ops::overlay(&mut bg, &rgba, 0, 0, 1.0);
            bg
        }
        "blur" => {
            let src = original.unwrap_or_else(|| rgba.clone());
            let side = src.width().max(src.height()) as f32;
            let sigma = v.int("blur").max(1) as f32 / 1000.0 * side;
            let mut bg = ops::blur_rgba(&src, sigma);
            ops::overlay(&mut bg, &rgba, 0, 0, 1.0);
            bg
        }
        _ => rgba,
    };
    if v.bool("crop") {
        let pad = v.int("padding").max(0) as u32;
        // Bounds of the subject from the matte (works for every background mode).
        let mut b = (u32::MAX, u32::MAX, 0u32, 0u32);
        for (x, y, a) in matte.enumerate_pixels() {
            if a[0] > 8 {
                b = (b.0.min(x), b.1.min(y), b.2.max(x), b.3.max(y));
            }
        }
        if b.0 <= b.2 {
            let (w, h) = out.dimensions();
            let x0 = b.0.saturating_sub(pad);
            let y0 = b.1.saturating_sub(pad);
            let x1 = (b.2 + 1 + pad).min(w);
            let y1 = (b.3 + 1 + pad).min(h);
            out = image::imageops::crop_imm(&out, x0, y0, x1 - x0, y1 - y0).to_image();
        }
    }
    out
}

/// Grey checkerboard behind transparent pixels (preview only).
pub fn checkerboard_under(img: &RgbaImage) -> RgbaImage {
    let (w, h) = img.dimensions();
    let cell = (w.max(h) / 40).max(6);
    let mut bg = RgbaImage::from_fn(w, h, |x, y| {
        let c = if ((x / cell) + (y / cell)) % 2 == 0 { 236 } else { 206 };
        image::Rgba([c, c, c, 255])
    });
    ops::overlay(&mut bg, img, 0, 0, 1.0);
    bg
}

// ------------------------------------------------------------------ upscale

pub struct Upscale {
    meta: ToolMeta,
}

impl Upscale {
    pub fn new() -> Self {
        Upscale {
            meta: common::meta(
                "image.upscale",
                "PNG · JPG · WEBP",
                &["upscale", "enlarge", "super resolution", "ai", "enhance", "bigger", "sharpen", "esrgan", "4x", "2x"],
                "upscaled",
                ToolStatus::Ready,
                &[Requirement::Model("realesr-general-x4v3")],
                true,
            ),
        }
    }
}

fn up_model(v: &Values) -> &'static str {
    match (v.str("model"), v.str("scale")) {
        ("art", _) => "realesrgan-x4plus-anime6b",
        ("fast", _) => "realesr-general-x4v3",
        (_, "2") => "realesrgan-x2plus",
        _ => "realesrgan-x4plus",
    }
}

impl Tool for Upscale {
    fn meta(&self) -> &ToolMeta {
        &self.meta
    }

    fn options(&self) -> Vec<OptionSpec> {
        vec![
            OptionSpec::choice("scale", vec![Choice::lit("2", "×2"), Choice::lit("3", "×3"), Choice::lit("4", "×4")], "2"),
            OptionSpec::choice("model", vec![Choice::key("fast"), Choice::key("photo"), Choice::key("art")], "fast"),
            common::format_option("keep"),
            common::quality_option(92),
        ]
    }

    fn downloads(&self, v: &Values) -> Vec<Asset> {
        engine_ml::models::asset(up_model(v)).into_iter().collect()
    }

    fn run(&self, job: &Job, v: &Values, cx: &RunCx) -> Result<Vec<Output>> {
        let input = job.input()?;
        let d = common::load(input, DecodeOpts::default())?;
        let scale: u32 = v.str("scale").parse().unwrap_or(2);
        let (w0, h0) = (d.image.width(), d.image.height());
        let alpha = crate::codec::uses_alpha(&d.image);
        let alpha_plane = if alpha { Some(d.image.to_rgba8()) } else { None };
        let rgb = crate::codec::flatten(&d.image, [0, 0, 0]);
        let sub = sub_cx(cx, 0.0, 0.95);
        let up = engine_ml::upscale(&rgb, scale, up_model(v), &sub)?;
        drop(rgb);
        cx.check_cancel()?;
        let img = match alpha_plane {
            Some(a) => {
                let big_a = ops::resize_rgba(&a, up.width(), up.height());
                let mut out = RgbaImage::new(up.width(), up.height());
                for ((o, p), s) in out.pixels_mut().zip(up.pixels()).zip(big_a.pixels()) {
                    o.0 = [p[0], p[1], p[2], s[3]];
                }
                DynamicImage::ImageRgba8(out)
            }
            None => DynamicImage::ImageRgb8(up),
        };
        let fmt = common::out_format(v, d.format, alpha);
        let o = EncodeOpts { quality: v.int("quality").clamp(1, 100) as u8, icc: d.icc.clone(), png_effort: 1, ..Default::default() };
        let out = common::save(&img, fmt, &o, Some(input), cx)?;
        Ok(vec![Output::new(out).with_note(format!("{} → {}", common::dims(w0, h0), common::dims(img.width(), img.height())))])
    }
}
