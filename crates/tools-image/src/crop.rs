//! image.crop — aspect-ratio presets (with focus), margins in px or %, auto-trim borders.

use anyhow::Result;
use castor_core::options::{Anchor, Choice, Cond, OptionSpec, Values};
use castor_core::{FileInfo, Job, Output, Preview, RunCx, Tool, ToolMeta, ToolStatus};
use image::DynamicImage;

use crate::codec::{DecodeOpts, Fmt};
use crate::{common, ops};

pub struct Crop {
    meta: ToolMeta,
}

pub const RATIOS: &[(&str, &str, f64)] = &[
    ("1-1", "1:1", 1.0),
    ("4-5", "4:5", 4.0 / 5.0),
    ("5-4", "5:4", 5.0 / 4.0),
    ("3-2", "3:2", 3.0 / 2.0),
    ("2-3", "2:3", 2.0 / 3.0),
    ("4-3", "4:3", 4.0 / 3.0),
    ("3-4", "3:4", 3.0 / 4.0),
    ("16-9", "16:9", 16.0 / 9.0),
    ("9-16", "9:16", 9.0 / 16.0),
    ("21-9", "21:9", 21.0 / 9.0),
    ("a4", "A4", 210.0 / 297.0),
    ("a4-landscape", "A4 ↔", 297.0 / 210.0),
];

impl Crop {
    pub fn new() -> Self {
        Crop {
            meta: common::meta(
                "image.crop",
                "JPG · PNG · WEBP",
                &["crop", "trim", "cut", "aspect", "ratio", "square", "borders", "margins"],
                "cropped",
                ToolStatus::Ready,
                &[],
                false,
            ),
        }
    }
}

impl Tool for Crop {
    fn meta(&self) -> &ToolMeta {
        &self.meta
    }

    fn options(&self) -> Vec<OptionSpec> {
        vec![
            OptionSpec::choice("mode", vec![Choice::key("ratio"), Choice::key("margins"), Choice::key("trim")], "ratio"),
            OptionSpec::choice("ratio", RATIOS.iter().map(|(v, l, _)| Choice::lit(v, l)).collect(), "1-1").when(Cond::Eq("mode", "ratio")),
            OptionSpec::toggle("auto_focus", true).when(Cond::Eq("mode", "ratio")),
            OptionSpec::anchor("focus", Anchor::Center).when(Cond::Eq("mode", "ratio")),
            OptionSpec::choice("unit", vec![Choice::lit("px", "px"), Choice::lit("percent", "%")], "px").when(Cond::Eq("mode", "margins")),
            OptionSpec::int("top", 0, 100_000, 0).when(Cond::Eq("mode", "margins")),
            OptionSpec::int("right", 0, 100_000, 0).when(Cond::Eq("mode", "margins")),
            OptionSpec::int("bottom", 0, 100_000, 0).when(Cond::Eq("mode", "margins")),
            OptionSpec::int("left", 0, 100_000, 0).when(Cond::Eq("mode", "margins")),
            OptionSpec::int("tolerance", 0, 128, 12).slider().when(Cond::Eq("mode", "trim")),
            OptionSpec::int("padding", 0, 1000, 0).unit("px").when(Cond::Eq("mode", "trim")),
        ]
    }

    fn run(&self, job: &Job, v: &Values, cx: &RunCx) -> Result<Vec<Output>> {
        let input = job.input()?;
        let d = common::load(input, DecodeOpts::default())?;
        cx.progress(0.3);
        let (w0, h0) = (d.image.width(), d.image.height());
        let (x, y, w, h) = rect(&d.image, v)?;
        let img = d.image.crop_imm(x, y, w, h);
        cx.progress(0.6);
        let alpha = crate::codec::uses_alpha(&img);
        let fmt = common::keep_format(d.format, alpha);
        let q = if d.format == Fmt::Jpeg { None } else { Some(92) };
        let o = common::resave_opts(&d, &input.path, q, true);
        let out = common::save(&img, fmt, &o, Some(input), cx)?;
        Ok(vec![Output::new(out).with_note(format!("{} → {}", common::dims(w0, h0), common::dims(w, h)))])
    }

    fn preview(&self, input: Option<&FileInfo>, v: &Values, max_side: u32) -> Result<Option<Preview>> {
        let Some(f) = input else { return Ok(None) };
        let img = common::load_preview(f, max_side)?;
        // Show the kept area bright and the rest dimmed: the user sees what goes.
        let (x, y, w, h) = rect(&img, v)?;
        let mut rgba = img.to_rgba8();
        for (px, py, p) in rgba.enumerate_pixels_mut() {
            if px < x || py < y || px >= x + w || py >= y + h {
                for c in 0..3 {
                    p[c] = (p[c] as u16 * 35 / 100) as u8;
                }
            }
        }
        common::preview(&rgba, max_side, None)
    }
}

/// The crop rectangle (x, y, w, h) for this image and these options.
pub fn rect(img: &DynamicImage, v: &Values) -> Result<(u32, u32, u32, u32)> {
    let (w, h) = (img.width(), img.height());
    let r = match v.str("mode") {
        "trim" => {
            let tol = v.int("tolerance").clamp(0, 255) as u8;
            let pad = v.int("padding").max(0) as u32;
            let rgba = img.to_rgba8();
            match ops::content_bounds(&rgba, tol) {
                Some((x, y, cw, ch)) => {
                    let x0 = x.saturating_sub(pad);
                    let y0 = y.saturating_sub(pad);
                    let x1 = (x + cw + pad).min(w);
                    let y1 = (y + ch + pad).min(h);
                    (x0, y0, x1 - x0, y1 - y0)
                }
                None => (0, 0, w, h),
            }
        }
        "margins" => {
            let pct = v.str("unit") == "percent";
            let m = |k: &str, total: u32| -> u32 {
                let n = v.int(k).max(0) as f64;
                if pct { (total as f64 * n.min(100.0) / 100.0).round() as u32 } else { n as u32 }
            };
            let (t, r, b, l) = (m("top", h), m("right", w), m("bottom", h), m("left", w));
            anyhow::ensure!(l + r < w && t + b < h, "{}", castor_core::tr("image.crop.too_much"));
            (l, t, w - l - r, h - t - b)
        }
        _ => {
            let ratio = RATIOS.iter().find(|(id, _, _)| *id == v.str("ratio")).map(|r| r.2).unwrap_or(1.0);
            let (cw, ch) = if w as f64 / h as f64 > ratio {
                (((h as f64 * ratio).round() as u32).clamp(1, w), h)
            } else {
                (w, ((w as f64 / ratio).round() as u32).clamp(1, h))
            };
            let (fx, fy) = if v.bool("auto_focus") {
                ops::saliency_center(img)
            } else {
                let (gx, gy) = v.anchor("focus").grid();
                (gx as f32 / 2.0, gy as f32 / 2.0)
            };
            let x = ((fx * w as f32 - cw as f32 / 2.0).round().max(0.0) as u32).min(w - cw);
            let y = ((fy * h as f32 - ch as f32 / 2.0).round().max(0.0) as u32).min(h - ch);
            (x, y, cw, ch)
        }
    };
    Ok(r)
}
