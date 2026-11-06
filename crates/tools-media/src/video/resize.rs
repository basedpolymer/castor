//! `video.resize`: standard heights, exact size (fit / pad / fill / stretch) or percentage.

use castor_core::options::{Choice, Cond, OptionSpec, Values};
use castor_core::{Family, InputSpec, Job, Output, Result, RunCx, Tool, ToolMeta, tr_args};
use engine_media::{Ffmpeg, Probe};

use crate::common::*;

pub struct Resize {
    meta: ToolMeta,
}

impl Resize {
    pub fn new() -> Self {
        Resize {
            meta: meta(
                "video.resize",
                Family::Video,
                InputSpec::kinds(VIDEO),
                "MP4 · MOV · MKV · WEBM",
                &["scale", "resolution", "1080p", "720p", "4k", "downscale", "upscale", "dimensions"],
                "resized",
                true,
            ),
        }
    }
}

/// The scale filter chain for the options; `None` = nothing to do.
pub fn resize_filter(p: &Probe, v: &Values) -> anyhow::Result<Option<String>> {
    let (w, h) = p.display_size();
    Ok(match v.str("preset") {
        "percent" => {
            let pc = v.int("percent").clamp(5, 800) as f64 / 100.0;
            if (pc - 1.0).abs() < 1e-6 {
                None
            } else {
                let nw = ((w as f64 * pc) / 2.0).round() as u32 * 2;
                let nh = ((h as f64 * pc) / 2.0).round() as u32 * 2;
                Some(format!("scale={}:{}:flags=lanczos", nw.max(2), nh.max(2)))
            }
        }
        "custom" => {
            let (tw, th) = v.size("size");
            anyhow::ensure!(tw >= 2 && th >= 2, castor_core::tr("media.err.bad_size"));
            let (tw, th) = (tw / 2 * 2, th / 2 * 2);
            Some(match v.str("fit") {
                "pad" => format!(
                    "scale={tw}:{th}:force_original_aspect_ratio=decrease:flags=lanczos,pad={tw}:{th}:(ow-iw)/2:(oh-ih)/2:color={},setsar=1",
                    pad_color(v)
                ),
                "fill" => format!("scale={tw}:{th}:force_original_aspect_ratio=increase:flags=lanczos,crop={tw}:{th},setsar=1"),
                "stretch" => format!("scale={tw}:{th}:flags=lanczos,setsar=1"),
                _ => format!("scale={tw}:{th}:force_original_aspect_ratio=decrease:force_divisible_by=2:flags=lanczos"),
            })
        }
        r => {
            let target: u32 = r.parse().unwrap_or(1080);
            let short = w.min(h);
            if short == target || (short < target && !v.bool("upscale")) {
                None
            } else if w >= h {
                Some(format!("scale=-2:{target}:flags=lanczos"))
            } else {
                Some(format!("scale={target}:-2:flags=lanczos"))
            }
        }
    })
}

fn pad_color(v: &Values) -> String {
    let [r, g, b, _] = v.color("pad_color");
    format!("0x{r:02x}{g:02x}{b:02x}")
}

impl Tool for Resize {
    fn meta(&self) -> &ToolMeta {
        &self.meta
    }
    fn options(&self) -> Vec<OptionSpec> {
        vec![
            OptionSpec::choice(
                "preset",
                vec![
                    Choice::lit("2160", "4K (2160p)"),
                    Choice::lit("1440", "1440p"),
                    Choice::lit("1080", "1080p"),
                    Choice::lit("720", "720p"),
                    Choice::lit("480", "480p"),
                    Choice::lit("360", "360p"),
                    Choice::key("percent"),
                    Choice::key("custom"),
                ],
                "720",
            ),
            OptionSpec::int("percent", 5, 800, 50).slider().unit("%").when(Cond::Eq("preset", "percent")),
            OptionSpec::size("size", 1080, 1920).when(Cond::Eq("preset", "custom")),
            OptionSpec::choice("fit", vec![Choice::key("fit"), Choice::key("pad"), Choice::key("fill"), Choice::key("stretch")], "fit")
                .when(Cond::Eq("preset", "custom")),
            OptionSpec::color("pad_color", "#000000").when(Cond::Eq("fit", "pad")),
            OptionSpec::toggle("upscale", false).advanced(),
            quality_option("balanced"),
            hardware_option(),
        ]
    }
    fn unavailable(&self) -> Option<String> {
        need_ffmpeg()
    }
    fn run(&self, job: &Job, v: &Values, cx: &RunCx) -> Result<Vec<Output>> {
        let input = job.input()?;
        let p = probe(input)?;
        need_video(&p)?;
        let Some(filter) = resize_filter(&p, v)? else {
            let (w, h) = p.display_size();
            let mut o = super::compress::keep_original(input, cx)?;
            o.note = Some(tr_args("media.note.size_unchanged", &[("w", &w.to_string()), ("h", &h.to_string())]));
            return Ok(vec![o]);
        };
        reencode_with_filter(input, &p, v, cx, vec![filter], level(v))
    }
}

/// Re-encode the video with extra filters, same container and codec family where possible.
pub fn reencode_with_filter(
    input: &castor_core::FileInfo,
    p: &Probe,
    v: &Values,
    cx: &RunCx,
    filters: Vec<String>,
    level: usize,
) -> Result<Vec<Output>> {
    let ext = ext_of(input);
    let mut enc = edit_encoder(p, &ext);
    enc.level = level;
    enc.hardware = v.bool("hardware");
    let out = Partial::new(cx.output_path(Some(input), &output_ext(&ext, enc.codec)));
    let ext = output_ext(&ext, enc.codec);
    encode_with_fallback(cx, &enc, |e| {
        let mut vf = filters.clone();
        finish_filters(p, e, &mut vf);
        let (va, _) = video_args(p, e);
        Ffmpeg::new()
            .input(&input.path)
            .args(stream_map(p, &ext, true, true))
            .args(["-vf", &vf.join(",")])
            .args(va)
            .args(edit_audio(p, &ext, false))
            .args(mux_args(&ext))
            .arg(out.path())
            .duration(p.duration)
    })?;
    let size = size_of(out.path());
    let path = out.keep();
    let dims = engine_media::probe(&path).map(|q| {
        let (w, h) = q.display_size();
        format!("{w} × {h}")
    });
    let note = match dims {
        Ok(d) => format!("{d} · {}", castor_core::units::bytes(size)),
        Err(_) => castor_core::units::bytes(size),
    };
    Ok(vec![Output::new(path).with_note(note)])
}
