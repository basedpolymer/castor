//! image.watermark — text or logo, anchored or tiled, opacity, scale, angle.

use anyhow::{Context, Result};
use castor_core::options::{Anchor, Choice, Cond, OptionSpec, Values};
use castor_core::{FileInfo, Job, Output, Preview, RunCx, Tool, ToolMeta, ToolStatus};
use image::{DynamicImage, RgbaImage};

use crate::codec::{DecodeOpts, Fmt};
use crate::{common, ops};

pub struct Watermark {
    meta: ToolMeta,
}

impl Watermark {
    pub fn new() -> Self {
        Watermark {
            meta: common::meta(
                "image.watermark",
                "JPG · PNG · WEBP",
                &["watermark", "logo", "copyright", "stamp", "protect", "brand", "signature", "overlay"],
                "watermarked",
                ToolStatus::Ready,
                &[],
                false,
            ),
        }
    }
}

impl Tool for Watermark {
    fn meta(&self) -> &ToolMeta {
        &self.meta
    }

    fn options(&self) -> Vec<OptionSpec> {
        let year = time::OffsetDateTime::now_utc().year();
        vec![
            OptionSpec::choice("kind", vec![Choice::key("text"), Choice::key("logo")], "text"),
            OptionSpec::text("text", &format!("© {year}")).when(Cond::Eq("kind", "text")),
            OptionSpec::font("font", "").when(Cond::Eq("kind", "text")),
            OptionSpec::color("color", "#ffffff").when(Cond::Eq("kind", "text")),
            OptionSpec::file("logo", &["png", "svg", "webp", "jpg", "jpeg"]).when(Cond::Eq("kind", "logo")),
            OptionSpec::choice("layout", vec![Choice::key("single"), Choice::key("tiled")], "single"),
            OptionSpec::anchor("position", Anchor::BottomRight).when(Cond::Eq("layout", "single")),
            OptionSpec::int("size", 2, 100, 18).slider().unit("%"),
            OptionSpec::int("opacity", 5, 100, 60).slider().unit("%"),
            OptionSpec::float("angle", -90.0, 90.0, 0.0).slider().unit("°"),
            OptionSpec::int("margin", 0, 30, 3).unit("%").when(Cond::Eq("layout", "single")),
            OptionSpec::int("spacing", 0, 300, 60).unit("%").when(Cond::Eq("layout", "tiled")),
            OptionSpec::toggle("shadow", true),
            common::format_option("keep"),
        ]
    }

    fn run(&self, job: &Job, v: &Values, cx: &RunCx) -> Result<Vec<Output>> {
        let input = job.input()?;
        let d = common::load(input, DecodeOpts::default())?;
        cx.progress(0.3);
        let mark = mark_image(v)?;
        let img = apply(d.image.to_rgba8(), &mark, v);
        cx.progress(0.8);
        let fmt = common::out_format(v, d.format, crate::codec::uses_alpha(&d.image));
        let q = if fmt == d.format && d.format == Fmt::Jpeg { None } else { Some(90) };
        let o = common::resave_opts(&d, &input.path, q, true);
        let out = common::save(&DynamicImage::ImageRgba8(img), fmt, &o, Some(input), cx)?;
        Ok(vec![Output::new(out)])
    }

    fn preview(&self, input: Option<&FileInfo>, v: &Values, max_side: u32) -> Result<Option<Preview>> {
        let base = match input {
            Some(f) => common::load_preview(f, max_side)?.to_rgba8(),
            None => sample_photo(max_side),
        };
        let mark = mark_image(v)?;
        common::preview(&apply(base, &mark, v), max_side, None)
    }
}

/// A neutral gradient stand-in when no file is selected.
fn sample_photo(side: u32) -> RgbaImage {
    let (w, h) = (side, side * 2 / 3);
    RgbaImage::from_fn(w, h, |x, y| {
        let t = x as f32 / w as f32;
        let u = y as f32 / h as f32;
        image::Rgba([(70.0 + 120.0 * t) as u8, (90.0 + 60.0 * u) as u8, (140.0 - 40.0 * t) as u8, 255])
    })
}

/// The watermark at a large reference size (scaled per image later).
fn mark_image(v: &Values) -> Result<RgbaImage> {
    if v.str("kind") == "logo" {
        let p = v.path("logo").context(castor_core::tr("image.watermark.no_logo"))?;
        let d = crate::codec::decode(p, DecodeOpts { svg_side: 1600, ..Default::default() })?;
        return Ok(d.image.to_rgba8());
    }
    let c = v.color("color");
    let text = v.str("text");
    ops::render_text(if text.trim().is_empty() { "©" } else { text }, v.str("font"), 160.0, c, true)
        .context(castor_core::tr("image.watermark.no_text"))
}

pub fn apply(mut img: RgbaImage, mark: &RgbaImage, v: &Values) -> RgbaImage {
    let (w, h) = img.dimensions();
    let size = v.int("size").clamp(1, 100) as f32 / 100.0;
    let opacity = v.int("opacity").clamp(0, 100) as f32 / 100.0;
    // Size relative to the shorter side for single marks (consistent across portrait/landscape).
    let target_w = ((w.min(h) as f32 * size).max(8.0)) as u32;
    let (mw, mh) = ops::fit_size(mark.width(), mark.height(), target_w, (h as f32 * 0.9) as u32);
    let mut m = ops::resize_rgba(mark, mw, mh);
    let angle = v.float("angle") as f32;
    if angle.abs() > 0.1 {
        m = ops::rotate_any(&m, angle, [0, 0, 0, 0], true);
    }
    let shadow = if v.bool("shadow") {
        let blur = (m.width().max(m.height()) as f32 / 60.0).max(1.5);
        let pad = (blur * 3.0) as u32;
        Some((ops::shadow(&m, blur, [0, 0, 0, 150], pad), pad))
    } else {
        None
    };
    let mut place = |x: i64, y: i64| {
        if let Some((s, pad)) = &shadow {
            let off = (m.height() as i64 / 40).max(1);
            ops::overlay(&mut img, s, x - *pad as i64 + off, y - *pad as i64 + off, opacity * 0.8);
        }
        ops::overlay(&mut img, &m, x, y, opacity);
    };
    if v.str("layout") == "tiled" {
        let gap = v.int("spacing").max(0) as f32 / 100.0;
        let sx = (m.width() as f32 * (1.0 + gap)).max(4.0) as i64;
        let sy = (m.height() as f32 * (1.0 + gap * 1.5)).max(4.0) as i64;
        let mut row = 0;
        let mut y = -(m.height() as i64) / 2;
        while y < h as i64 {
            let shift = if row % 2 == 1 { sx / 2 } else { 0 };
            let mut x = -shift;
            while x < w as i64 {
                place(x, y);
                x += sx;
            }
            y += sy;
            row += 1;
        }
    } else {
        let margin = (w.min(h) as f32 * v.int("margin").max(0) as f32 / 100.0) as i64;
        let (gx, gy) = v.anchor("position").grid();
        let x = match gx {
            0 => margin,
            1 => (w as i64 - m.width() as i64) / 2,
            _ => w as i64 - m.width() as i64 - margin,
        };
        let y = match gy {
            0 => margin,
            1 => (h as i64 - m.height() as i64) / 2,
            _ => h as i64 - m.height() as i64 - margin,
        };
        place(x, y);
    }
    img
}
