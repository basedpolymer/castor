//! `video.rotate`: rotate / flip, losslessly (display matrix) when the container allows it.

use castor_core::options::{Choice, OptionSpec, Values};
use castor_core::{Family, InputSpec, Job, Output, Result, RunCx, Tool, ToolMeta, tr};
use engine_media::Ffmpeg;

use crate::common::*;

pub struct Rotate {
    meta: ToolMeta,
}

impl Rotate {
    pub fn new() -> Self {
        Rotate {
            meta: meta(
                "video.rotate",
                Family::Video,
                InputSpec::kinds(VIDEO),
                "MP4 · MOV · MKV · WEBM",
                &["turn", "flip", "mirror", "sideways", "portrait", "landscape", "upside down", "orientation"],
                "rotated",
                true,
            ),
        }
    }
}

/// Clockwise degrees of the `angle` option.
fn clockwise(v: &Values) -> i32 {
    match v.str("angle") {
        "90" => 90,
        "180" => 180,
        "270" => 270,
        _ => 0,
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
                vec![Choice::key("90"), Choice::key("180"), Choice::key("270"), Choice::key("0")],
                "90",
            ),
            OptionSpec::choice("flip", vec![Choice::key("none"), Choice::key("horizontal"), Choice::key("vertical")], "none"),
            OptionSpec::toggle("lossless", true),
            quality_option("best").advanced(),
        ]
    }
    fn unavailable(&self) -> Option<String> {
        need_ffmpeg()
    }
    fn run(&self, job: &Job, v: &Values, cx: &RunCx) -> Result<Vec<Output>> {
        let input = job.input()?;
        let p = probe(input)?;
        need_video(&p)?;
        let cw = clockwise(v);
        let flip = v.str("flip");
        anyhow::ensure!(cw != 0 || flip != "none", tr("media.err.nothing_to_do"));
        let ext = ext_of(input);
        if v.bool("lossless") && matches!(ext.as_str(), "mp4" | "mov" | "m4v") {
            // The display matrix is counter-clockwise; it replaces the existing one.
            let new_rot = ((p.rotation - cw) % 360 + 360) % 360;
            let out = Partial::new(cx.output_path(Some(input), &ext));
            let mut ff = Ffmpeg::new().args(["-display_rotation:v:0", &new_rot.to_string()]);
            if flip == "horizontal" {
                ff = ff.args(["-display_hflip:v:0"]);
            }
            if flip == "vertical" {
                ff = ff.args(["-display_vflip:v:0"]);
            }
            ff.input(&input.path)
                .args(["-map", "0", "-c", "copy", "-map_metadata", "0"])
                .args(mux_args(&ext))
                .arg(out.path())
                .duration(p.duration)
                .run(cx)?;
            let path = out.keep();
            return Ok(vec![Output::new(path).with_note(tr("media.note.lossless_rotate"))]);
        }
        let mut f = Vec::new();
        match cw {
            90 => f.push("transpose=clock".to_string()),
            180 => f.push("hflip,vflip".to_string()),
            270 => f.push("transpose=cclock".to_string()),
            _ => {}
        }
        match flip {
            "horizontal" => f.push("hflip".into()),
            "vertical" => f.push("vflip".into()),
            _ => {}
        }
        super::resize::reencode_with_filter(input, &p, v, cx, f, level(v))
    }
}
