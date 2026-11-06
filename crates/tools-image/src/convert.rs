//! image.convert — any image in, JPG / PNG / WebP / AVIF / GIF / TIFF / BMP / ICO (+ HEIC on macOS).

use anyhow::Result;
use castor_core::options::{Choice, Cond, OptionSpec, Values};
use castor_core::{Job, Output, RunCx, Tool, ToolMeta, ToolStatus};
use image::DynamicImage;

use crate::codec::{self, DecodeOpts, EncodeOpts, Fmt};
use crate::common;

pub struct Convert {
    meta: ToolMeta,
}

impl Convert {
    pub fn new() -> Self {
        Convert {
            meta: common::meta(
                "image.convert",
                "JPG · PNG · WEBP · AVIF",
                &["heic", "jpg", "jpeg", "png", "webp", "avif", "raw", "svg", "format", "change", "transform", "gif", "tiff", "ico"],
                "",
                ToolStatus::Ready,
                &[],
                false,
            ),
        }
    }
}

fn formats() -> Vec<Choice> {
    let mut v = vec![
        Choice::lit("jpg", "JPG"),
        Choice::lit("png", "PNG"),
        Choice::lit("webp", "WebP"),
        Choice::lit("avif", "AVIF"),
        Choice::lit("gif", "GIF"),
        Choice::lit("tiff", "TIFF"),
        Choice::lit("bmp", "BMP"),
        Choice::lit("ico", "ICO"),
    ];
    if cfg!(target_os = "macos") {
        v.push(Choice::lit("heic", "HEIC"));
    }
    v
}

impl Tool for Convert {
    fn meta(&self) -> &ToolMeta {
        &self.meta
    }

    fn options(&self) -> Vec<OptionSpec> {
        vec![
            OptionSpec::choice("format", formats(), "jpg"),
            common::quality_option(88),
            OptionSpec::toggle("lossless", false).when(Cond::Eq("format", "webp")),
            OptionSpec::color("background", "#ffffff"),
            OptionSpec::toggle("keep_metadata", true),
            OptionSpec::choice("color", vec![Choice::key("srgb"), Choice::key("keep")], "srgb").advanced(),
            OptionSpec::toggle("animation", true).advanced(),
            OptionSpec::int("svg_size", 0, 16_384, 0).unit("px").advanced(),
        ]
    }

    fn run(&self, job: &Job, v: &Values, cx: &RunCx) -> Result<Vec<Output>> {
        let input = job.input()?;
        let target = Fmt::from_ext(v.str("format")).unwrap_or(Fmt::Jpeg);
        let animated_target = matches!(target, Fmt::Gif | Fmt::Webp);
        let d = common::load(
            input,
            DecodeOpts { orient: true, frames: v.bool("animation") && animated_target, svg_side: v.int("svg_size").max(0) as u32 },
        )?;
        cx.check_cancel()?;
        cx.progress(0.3);
        let bg = v.color("background");
        let keep_meta = v.bool("keep_metadata");
        let mut icc = d.icc.clone();
        let mut img = d.image;
        if v.str("color") != "keep" {
            if let Some(conv) = crate::color::to_srgb(&img, icc.as_deref()) {
                img = conv;
                icc = None;
            }
        }
        let o = EncodeOpts {
            quality: v.int("quality").clamp(1, 100) as u8,
            lossless: v.bool("lossless"),
            background: [bg[0], bg[1], bg[2]],
            icc: if target == Fmt::Jpeg || target == Fmt::Png || target == Fmt::Webp { icc } else { None },
            exif: if keep_meta { d.exif.as_deref().map(crate::meta::reset_orientation) } else { None },
            xmp: if keep_meta { d.xmp.clone() } else { None },
            ..Default::default()
        };
        let bytes = match (&d.frames, target) {
            (Some(frames), Fmt::Gif) => codec::encode_gifski(frames.clone(), o.quality.max(70), 0, None, cx.cancel_token())?,
            (Some(frames), Fmt::Webp) => codec::encode_webp_frames(frames, &o)?,
            _ => {
                let img = if target == Fmt::Jpeg && matches!(img, DynamicImage::ImageRgb16(_) | DynamicImage::ImageRgba16(_)) {
                    DynamicImage::ImageRgb8(img.to_rgb8())
                } else {
                    img
                };
                codec::encode(&img, target, &o)?
            }
        };
        cx.check_cancel()?;
        let out = cx.output_path(Some(input), target.ext());
        common::write(&out, &bytes)?;
        Ok(vec![Output::new(out).with_note(common::size_note(input.size, bytes.len() as u64))])
    }
}
