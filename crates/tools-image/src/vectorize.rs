//! image.vectorize — raster → SVG with vtracer; presets logo / illustration / photo, or auto.

use anyhow::Result;
use castor_core::options::{Choice, OptionSpec, Values};
use castor_core::{FileInfo, Job, Output, Preview, RunCx, Tool, ToolMeta, ToolStatus};
use image::RgbaImage;

use crate::codec::DecodeOpts;
use crate::{common, ops};

pub struct Vectorize {
    meta: ToolMeta,
}

impl Vectorize {
    pub fn new() -> Self {
        Vectorize {
            meta: common::meta(
                "image.vectorize",
                "SVG",
                &["vectorize", "svg", "trace", "vector", "logo", "potrace", "illustrator", "scalable"],
                "",
                ToolStatus::Ready,
                &[],
                false,
            ),
        }
    }
}

impl Tool for Vectorize {
    fn meta(&self) -> &ToolMeta {
        &self.meta
    }

    fn options(&self) -> Vec<OptionSpec> {
        vec![
            OptionSpec::choice(
                "preset",
                vec![Choice::key("auto"), Choice::key("logo"), Choice::key("bw"), Choice::key("illustration"), Choice::key("photo")],
                "auto",
            ),
            OptionSpec::int("colors", 1, 8, 6).slider(),
            OptionSpec::int("detail", 0, 100, 70).slider().unit("%"),
            OptionSpec::choice("curves", vec![Choice::key("smooth"), Choice::key("sharp")], "smooth"),
            OptionSpec::int("max_size", 256, 8000, 2000).unit("px").advanced(),
        ]
    }

    fn run(&self, job: &Job, v: &Values, cx: &RunCx) -> Result<Vec<Output>> {
        let input = job.input()?;
        let d = common::load(input, DecodeOpts::default())?;
        cx.progress(0.1);
        let img = d.image.to_rgba8();
        let (svg, preset) = trace(&img, v)?;
        cx.check_cancel()?;
        let out = cx.output_path(Some(input), "svg");
        common::write(&out, svg.as_bytes())?;
        let note = castor_core::tr(&format!("image.vectorize.opt.preset.{preset}"));
        Ok(vec![Output::new(out).with_note(note)])
    }

    fn preview(&self, input: Option<&FileInfo>, v: &Values, max_side: u32) -> Result<Option<Preview>> {
        let Some(f) = input else { return Ok(None) };
        let img = common::load_preview(f, max_side.min(512))?.to_rgba8();
        let (svg, _) = trace(&img, v)?;
        let r = crate::codec::render_svg(svg.as_bytes(), max_side, None)?;
        common::preview(&crate::ml_tools::checkerboard_under(&r), max_side, None)
    }
}

/// Distinct colours (quantised to 4 bits/channel) on a small sample.
fn distinct_colors(img: &RgbaImage) -> usize {
    let small = image::imageops::thumbnail(img, 128, 128);
    let mut set = std::collections::HashSet::new();
    for p in small.pixels() {
        if p[3] > 128 {
            set.insert((p[0] >> 4, p[1] >> 4, p[2] >> 4));
        }
    }
    set.len()
}

fn trace(img: &RgbaImage, v: &Values) -> Result<(String, &'static str)> {
    let max = v.int("max_size").clamp(64, 16_000) as u32;
    let img = if img.width().max(img.height()) > max {
        let (w, h) = ops::fit_size(img.width(), img.height(), max, max);
        ops::resize_rgba(img, w, h)
    } else {
        img.clone()
    };
    let preset: &'static str = match v.str("preset") {
        "logo" => "logo",
        "bw" => "bw",
        "illustration" => "illustration",
        "photo" => "photo",
        _ => {
            let n = distinct_colors(&img);
            if n <= 3 {
                "bw"
            } else if n <= 64 {
                "logo"
            } else if n <= 600 {
                "illustration"
            } else {
                "photo"
            }
        }
    };
    let mut cfg = match preset {
        "bw" => vtracer::Config::from_preset(vtracer::Preset::Bw),
        "photo" => vtracer::Config::from_preset(vtracer::Preset::Photo),
        _ => vtracer::Config::from_preset(vtracer::Preset::Poster),
    };
    let detail = v.int("detail").clamp(0, 100) as f64 / 100.0;
    cfg.color_precision = v.int("colors").clamp(1, 8) as i32;
    // Detail: smaller speckles kept, finer colour layers.
    let base_speckle = if preset == "photo" { 16.0 } else { 8.0 };
    cfg.filter_speckle = ((1.0 - detail) * base_speckle + 1.0).round() as usize;
    cfg.layer_difference = if preset == "logo" { 24 } else if preset == "photo" { (64.0 - detail * 48.0) as i32 } else { (40.0 - detail * 28.0) as i32 };
    if v.str("curves") == "sharp" {
        cfg.mode = visioncortex::PathSimplifyMode::Polygon;
    }
    if preset == "logo" {
        cfg.corner_threshold = 50;
    }
    let ci = vtracer::ColorImage { pixels: img.as_raw().clone(), width: img.width() as usize, height: img.height() as usize };
    let svg = vtracer::convert(ci, cfg).map_err(|e| anyhow::anyhow!("{e}"))?;
    let mut s = svg.to_string();
    // Scalable output: viewBox, and a generator comment.
    if !s.contains("viewBox") {
        s = s.replacen(
            &format!("width=\"{}\" height=\"{}\"", img.width(), img.height()),
            &format!("width=\"{w}\" height=\"{h}\" viewBox=\"0 0 {w} {h}\"", w = img.width(), h = img.height()),
            1,
        );
    }
    Ok((s, preset))
}
