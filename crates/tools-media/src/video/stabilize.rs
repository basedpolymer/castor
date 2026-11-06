//! `video.stabilize`: two-pass vid.stab (detect, then smooth the camera path), with a light
//! sharpen; falls back to ffmpeg's `deshake` when vid.stab is not compiled in.

use castor_core::options::{Choice, OptionSpec, Values};
use castor_core::{Family, InputSpec, Job, Output, Result, RunCx, Tool, ToolMeta, tr};
use engine_media::Ffmpeg;

use crate::common::*;

pub struct Stabilize {
    meta: ToolMeta,
}

impl Stabilize {
    pub fn new() -> Self {
        Stabilize {
            meta: meta(
                "video.stabilize",
                Family::Video,
                InputSpec::kinds(VIDEO),
                "MP4 · MOV · MKV · WEBM",
                &["shaky", "steady", "stabilise", "gimbal", "smooth", "handheld", "deshake"],
                "stable",
                true,
            ),
        }
    }
}

impl Tool for Stabilize {
    fn meta(&self) -> &ToolMeta {
        &self.meta
    }
    fn options(&self) -> Vec<OptionSpec> {
        vec![
            OptionSpec::int("strength", 1, 10, 5).slider(),
            OptionSpec::choice("edges", vec![Choice::key("zoom"), Choice::key("black")], "zoom"),
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
        let s = v.int("strength").clamp(1, 10);
        let zoom = v.str("edges") != "black";
        let tmp = std::path::absolute(cx.tmp_dir())?;
        let src = std::path::absolute(&input.path)?;
        let vi = p.video_index.unwrap_or(0);
        let filter = if engine_media::has_filter("vidstabdetect") {
            cx.message(tr("media.msg.analysing_motion"));
            let detect = format!("vidstabdetect=shakiness={}:accuracy=15:stepsize=6:result=transforms.trf", (3 + s / 2).min(10));
            Ffmpeg::new()
                .input(&src)
                .args(["-map", &format!("0:{vi}"), "-an", "-sn", "-vf", &detect, "-f", "null", "-"])
                .cwd(&tmp)
                .duration(p.duration)
                .range(0.0, 0.4)
                .run(cx)?;
            cx.message(tr("media.msg.stabilising"));
            format!(
                "vidstabtransform=input=transforms.trf:smoothing={}:optzoom={}:zoomspeed=0.2:interpol=bicubic:crop={},unsharp=5:5:0.6:3:3:0.3",
                6 + s * 5,
                if zoom { 2 } else { 0 },
                if zoom { "keep" } else { "black" }
            )
        } else {
            "deshake=rx=32:ry=32,unsharp=5:5:0.6:3:3:0.3".to_string()
        };
        let ext = ext_of(input);
        let enc = VideoEnc::quality(edit_encoder(&p, &ext).codec, level(v));
        let ext = output_ext(&ext, enc.codec);
        let mut vf = vec![filter];
        finish_filters(&p, &enc, &mut vf);
        let (va, _) = video_args(&p, &enc);
        let out = Partial::new(cx.output_path(Some(input), &ext));
        let dst = std::path::absolute(out.path())?;
        Ffmpeg::new()
            .input(&src)
            .args(stream_map(&p, &ext, true, true))
            .args(["-vf", &vf.join(",")])
            .args(va)
            .args(edit_audio(&p, &ext, false))
            .args(mux_args(&ext))
            .arg(&dst)
            .cwd(&tmp)
            .duration(p.duration)
            .range(0.4, 1.0)
            .run(cx)?;
        let size = size_of(out.path());
        Ok(vec![Output::new(out.keep()).with_note(castor_core::units::bytes(size))])
    }
}
