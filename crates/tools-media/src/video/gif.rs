//! `video.to-gif`: two-stage palette (palettegen + paletteuse) for clean colours.

use castor_core::options::{Choice, Cond, OptionSpec, Values};
use castor_core::{Family, FileInfo, InputSpec, Job, Output, Preview, Result, RunCx, Tool, ToolMeta, tr_args, units};
use engine_media::Ffmpeg;

use crate::common::*;

pub struct ToGif {
    meta: ToolMeta,
}

impl ToGif {
    pub fn new() -> Self {
        ToGif {
            meta: meta(
                "video.to-gif",
                Family::Video,
                InputSpec::kinds(VIDEO),
                "GIF",
                &["gif", "animated", "animation", "meme", "loop"],
                "",
                true,
            ),
        }
    }
}

#[derive(Clone, Debug)]
pub struct GifSettings {
    pub fps: u32,
    pub width: u32,
    pub colors: u32,
    pub dither: String,
    pub diff: bool,
    pub loop_forever: bool,
}

impl GifSettings {
    pub fn preset(p: &str) -> GifSettings {
        match p {
            "quality" => GifSettings { fps: 20, width: 640, colors: 256, dither: "sierra2_4a".into(), diff: false, loop_forever: true },
            "small" => GifSettings { fps: 10, width: 320, colors: 96, dither: "bayer".into(), diff: true, loop_forever: true },
            _ => GifSettings { fps: 15, width: 480, colors: 192, dither: "sierra2_4a".into(), diff: true, loop_forever: true },
        }
    }
    fn from_values(v: &Values) -> GifSettings {
        let mut s = if v.str("preset") == "custom" {
            GifSettings {
                fps: v.int("fps").clamp(1, 50) as u32,
                width: v.int("width").clamp(32, 3840) as u32,
                colors: v.int("colors").clamp(2, 256) as u32,
                dither: v.str("dither").to_string(),
                diff: true,
                loop_forever: true,
            }
        } else {
            GifSettings::preset(v.str("preset"))
        };
        s.loop_forever = v.get("loop").is_none() || v.bool("loop");
        s
    }
    /// The palette filter graph (one input, one output). `src_fps` 0 = single picture (preview):
    /// no frame-rate conversion.
    pub fn graph(&self, src_fps: f64) -> String {
        let fps = if src_fps > 0.0 { format!("fps={:.3},", (self.fps as f64).min(src_fps)) } else { String::new() };
        let dither = match self.dither.as_str() {
            "bayer" => "bayer:bayer_scale=3".to_string(),
            "none" => "none".to_string(),
            "floyd_steinberg" => "floyd_steinberg".to_string(),
            _ => "sierra2_4a".to_string(),
        };
        format!(
            "{fps}scale=w='min({w},iw)':h=-2:flags=lanczos,split[a][b];[a]palettegen=max_colors={c}:stats_mode={sm}[p];[b][p]paletteuse=dither={dither}{dm}",
            w = self.width,
            c = self.colors,
            sm = if self.diff { "diff" } else { "full" },
            dm = if self.diff { ":diff_mode=rectangle" } else { "" },
        )
    }
}

/// Make a GIF from `input`, optionally from `start` for `len` seconds.
pub fn make_gif(input: &FileInfo, s: &GifSettings, start: Option<f64>, len: Option<f64>, cx: &RunCx) -> Result<Vec<Output>> {
    let p = probe(input)?;
    need_video(&p)?;
    let start = start.unwrap_or(0.0);
    anyhow::ensure!(p.duration <= 0.0 || start < p.duration, castor_core::tr("media.err.empty_range"));
    let len = len.unwrap_or((p.duration - start).max(0.1));
    let out = Partial::new(cx.output_path(Some(input), "gif"));
    let mut ff = Ffmpeg::new();
    if start > 0.0 {
        ff = ff.args(["-ss", &format!("{start:.3}")]);
    }
    ff.input(&input.path)
        .args(["-t", &format!("{len:.3}"), "-map", &format!("0:{}", p.video_index.unwrap_or(0)), "-an", "-sn"])
        .args(["-vf", &s.graph(p.fps)])
        .args(["-loop", if s.loop_forever { "0" } else { "-1" }])
        .arg(out.path())
        .duration(len)
        .run(cx)?;
    let size = size_of(out.path());
    let path = out.keep();
    let (dw, dh) = p.display_size();
    let w = s.width.min(dw.max(1));
    let h = ((dh as f64) * (w as f64) / (dw.max(1) as f64)).round() as u32;
    let note = tr_args(
        "media.note.gif",
        &[("w", &w.to_string()), ("h", &h.to_string()), ("fps", &s.fps.to_string()), ("size", &units::bytes(size))],
    );
    Ok(vec![Output::new(path).with_note(note)])
}

impl Tool for ToGif {
    fn meta(&self) -> &ToolMeta {
        &self.meta
    }
    fn options(&self) -> Vec<OptionSpec> {
        let custom = || Cond::Eq("preset", "custom");
        vec![
            OptionSpec::choice(
                "preset",
                vec![Choice::key("quality"), Choice::key("balanced"), Choice::key("small"), Choice::key("custom")],
                "balanced",
            ),
            OptionSpec::int("fps", 1, 50, 15).slider().unit("fps").when(custom()),
            OptionSpec::int("width", 32, 3840, 480).unit("px").when(custom()),
            OptionSpec::int("colors", 2, 256, 192).slider().when(custom()),
            OptionSpec::choice(
                "dither",
                vec![Choice::key("sierra2_4a"), Choice::key("floyd_steinberg"), Choice::key("bayer"), Choice::key("none")],
                "sierra2_4a",
            )
            .when(custom()),
            OptionSpec::text("start", "").placeholder("media.ph.time"),
            OptionSpec::text("length", "").placeholder("media.ph.time_all"),
            OptionSpec::toggle("loop", true),
        ]
    }
    fn unavailable(&self) -> Option<String> {
        need_ffmpeg()
    }
    fn run(&self, job: &Job, v: &Values, cx: &RunCx) -> Result<Vec<Output>> {
        let input = job.input()?;
        make_gif(input, &GifSettings::from_values(v), time_opt(v, "start")?, time_opt(v, "length")?, cx)
    }
    fn preview(&self, input: Option<&FileInfo>, v: &Values, max_side: u32) -> Result<Option<Preview>> {
        let Some(input) = input else { return Ok(None) };
        let p = probe(input)?;
        if !p.has_video {
            return Ok(None);
        }
        let s = GifSettings::from_values(v);
        let at = time_opt(v, "start").ok().flatten().unwrap_or(p.duration * 0.1);
        let png = engine_media::frame_png(&input.path, at, max_side, Some(&s.graph(0.0)))?;
        let (dw, dh) = p.display_size();
        let w = s.width.min(dw.max(1));
        let h = ((dh as f64) * (w as f64) / (dw.max(1) as f64)).round() as u32;
        let caption = tr_args(
            "video.to-gif.caption",
            &[("w", &w.to_string()), ("h", &h.to_string()), ("fps", &s.fps.to_string()), ("colors", &s.colors.to_string())],
        );
        Ok(Some(Preview { png, caption: Some(caption) }))
    }
}
