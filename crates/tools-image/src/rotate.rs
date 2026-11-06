//! image.rotate — 90 / 180 / 270, flips, "auto" (apply EXIF orientation), straighten by any angle.

use anyhow::Result;
use castor_core::options::{Choice, OptionSpec, Values};
use castor_core::{FileInfo, Job, Output, Preview, RunCx, Tool, ToolMeta, ToolStatus};
use image::DynamicImage;

use crate::codec::{DecodeOpts, Fmt};
use crate::{common, ops};

pub struct Rotate {
    meta: ToolMeta,
}

impl Rotate {
    pub fn new() -> Self {
        Rotate {
            meta: common::meta(
                "image.rotate",
                "JPG · PNG · WEBP",
                &["rotate", "flip", "mirror", "turn", "orientation", "straighten", "upright", "sideways"],
                "rotated",
                ToolStatus::Ready,
                &[],
                false,
            ),
        }
    }
}

impl Tool for Rotate {
    fn meta(&self) -> &ToolMeta {
        &self.meta
    }

    fn options(&self) -> Vec<OptionSpec> {
        vec![
            OptionSpec::choice(
                "angle",
                vec![Choice::lit("90", "90°"), Choice::lit("180", "180°"), Choice::lit("270", "270°"), Choice::key("auto"), Choice::key("none")],
                "90",
            ),
            OptionSpec::choice("flip", vec![Choice::key("none"), Choice::key("horizontal"), Choice::key("vertical")], "none"),
            OptionSpec::float("straighten", -45.0, 45.0, 0.0).slider().unit("°"),
            OptionSpec::choice("edges", vec![Choice::key("crop"), Choice::key("expand")], "crop"),
            OptionSpec::color("background", "#ffffff00").advanced(),
        ]
    }

    fn run(&self, job: &Job, v: &Values, cx: &RunCx) -> Result<Vec<Output>> {
        let input = job.input()?;
        let d = common::load(input, DecodeOpts::default())?;
        cx.progress(0.3);
        let img = apply(d.image.clone(), v);
        cx.progress(0.7);
        let alpha = crate::codec::uses_alpha(&img);
        let fmt = common::keep_format(d.format, alpha);
        let q = if d.format == Fmt::Jpeg { None } else { Some(92) };
        // Pixels are upright now: the EXIF orientation is reset to 1.
        let o = common::resave_opts(&d, &input.path, q, true);
        let out = common::save(&img, fmt, &o, Some(input), cx)?;
        Ok(vec![Output::new(out)])
    }

    fn preview(&self, input: Option<&FileInfo>, v: &Values, max_side: u32) -> Result<Option<Preview>> {
        let Some(f) = input else { return Ok(None) };
        let img = common::load_preview(f, max_side)?;
        let out = apply(img, v);
        common::preview(&out.to_rgba8(), max_side, None)
    }
}

pub fn apply(img: DynamicImage, v: &Values) -> DynamicImage {
    let mut img = match v.str("angle") {
        "90" => img.rotate90(),
        "180" => img.rotate180(),
        "270" => img.rotate270(),
        _ => img,
    };
    img = match v.str("flip") {
        "horizontal" => img.fliph(),
        "vertical" => img.flipv(),
        _ => img,
    };
    let a = v.float("straighten") as f32;
    if a.abs() > 0.01 {
        let bg = v.color("background");
        let expand = v.str("edges") == "expand";
        let r = ops::rotate_any(&img.to_rgba8(), a, bg, expand);
        img = DynamicImage::ImageRgba8(r);
    }
    img
}
