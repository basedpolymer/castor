//! `video.frames`: stills every N seconds, N evenly spaced, one per scene, or a contact sheet.

use std::path::Path;

use castor_core::options::{Choice, Cond, OptionSpec, Values};
use castor_core::{Family, InputSpec, Job, Output, Result, RunCx, Tool, ToolMeta, tr, units};
use engine_media::{Ffmpeg, filter_escape};

use crate::common::*;

pub struct Frames {
    meta: ToolMeta,
}

impl Frames {
    pub fn new() -> Self {
        Frames {
            meta: meta(
                "video.frames",
                Family::Video,
                InputSpec::kinds(VIDEO),
                "PNG · JPG · WEBP",
                &["screenshot", "still", "thumbnail", "snapshot", "contact sheet", "storyboard", "image", "export frames"],
                "frame",
                true,
            ),
        }
    }
}

const MAX_FRAMES: usize = 2000;

impl Tool for Frames {
    fn meta(&self) -> &ToolMeta {
        &self.meta
    }
    fn options(&self) -> Vec<OptionSpec> {
        vec![
            OptionSpec::choice("mode", vec![Choice::key("count"), Choice::key("every"), Choice::key("scenes"), Choice::key("sheet")], "count"),
            OptionSpec::int("count", 1, 500, 10).when(Cond::Eq("mode", "count")),
            OptionSpec::text("every", "10").placeholder("media.ph.time").when(Cond::Eq("mode", "every")),
            OptionSpec::int("columns", 1, 12, 4).when(Cond::Eq("mode", "sheet")),
            OptionSpec::int("rows", 1, 30, 4).when(Cond::Eq("mode", "sheet")),
            OptionSpec::toggle("timestamps", true).when(Cond::Eq("mode", "sheet")),
            OptionSpec::choice("format", vec![Choice::lit("png", "PNG"), Choice::lit("jpg", "JPG"), Choice::lit("webp", "WebP")], "jpg").common(),
            OptionSpec::int("width", 0, 7680, 0).unit("px").advanced(),
        ]
    }
    fn unavailable(&self) -> Option<String> {
        need_ffmpeg()
    }
    fn run(&self, job: &Job, v: &Values, cx: &RunCx) -> Result<Vec<Output>> {
        let input = job.input()?;
        let p = probe(input)?;
        need_video(&p)?;
        let dur = p.duration;
        anyhow::ensure!(dur > 0.0, tr("media.err.no_duration"));
        let fmt = v.str("format").to_string();
        let width = v.int("width").max(0) as u32;
        let mode = v.str("mode");
        let (times, base): (Vec<f64>, f32) = match mode {
            "every" => {
                let step = time_opt(v, "every")?.filter(|s| *s > 0.0).ok_or_else(|| anyhow::anyhow!(tr("media.err.bad_length")))?;
                let t: Vec<f64> = (0..).map(|i| i as f64 * step).take_while(|t| *t < dur).take(MAX_FRAMES + 1).collect();
                anyhow::ensure!(t.len() <= MAX_FRAMES, tr("media.err.too_many_frames"));
                (t, 0.0)
            }
            "scenes" => {
                cx.message(tr("media.msg.finding_scenes"));
                let mut t = vec![0.0];
                t.extend(engine_media::analyze::scenes(&input.path, 12.0, 1.0, dur, cx, (0.0, 0.5))?.into_iter().map(|s| s + 0.1));
                t.truncate(MAX_FRAMES);
                (t, 0.5)
            }
            "sheet" => {
                let n = (v.int("columns").clamp(1, 12) * v.int("rows").clamp(1, 30)) as usize;
                ((0..n).map(|i| (i as f64 + 0.5) * dur / n as f64).collect(), 0.0)
            }
            _ => {
                let n = v.int("count").clamp(1, 500) as usize;
                ((0..n).map(|i| (i as f64 + 0.5) * dur / n as f64).collect(), 0.0)
            }
        };
        let tmp = std::path::absolute(cx.tmp_dir())?;
        let src = std::path::absolute(&input.path)?;
        let sheet = mode == "sheet";
        let stamp = sheet && v.bool("timestamps");
        let font = if stamp { Some(crate::fonts::prepare("", true, &tmp)?) } else { None };
        let tile_w = if width > 0 { width } else { 1600 } / v.int("columns").clamp(1, 12) as u32;
        let digits = times.len().to_string().len().max(3);
        let mut outs = Vec::new();
        let mut tiles = Vec::new();
        for (i, t) in times.iter().enumerate() {
            cx.check_cancel()?;
            cx.progress(base + (1.0 - base) * (i as f32 / times.len() as f32) * if sheet { 0.9 } else { 1.0 });
            let mut vf: Vec<String> = Vec::new();
            if sheet {
                vf.push(format!("scale={tile_w}:-2:flags=lanczos"));
            } else if width > 0 {
                vf.push(format!("scale=w='min({width},iw)':h=-2:flags=lanczos"));
            }
            if let Some(f) = &font {
                vf.push(format!(
                    "drawtext=fontfile={}:text='{}':fontsize=h/11:fontcolor=white:box=1:boxcolor=black@0.55:boxborderw=6:x=8:y=h-th-12",
                    f.file,
                    filter_escape(&units::timecode(*t)[..8])
                ));
            }
            let name = if sheet { format!("tile{i:04}.png") } else { format!("f{i:04}.{fmt}") };
            let mut ff = Ffmpeg::new().args(["-ss", &format!("{t:.3}")]).input(&src).args(["-map", &format!("0:{}", p.video_index.unwrap_or(0)), "-frames:v", "1"]);
            if !vf.is_empty() {
                ff = ff.args(["-vf", &vf.join(",")]);
            }
            if !sheet {
                ff = ff.args(quality_args(&fmt));
            }
            ff.args(["-update", "1"]).arg(&name).cwd(&tmp).quiet().run(cx)?;
            if !tmp.join(&name).is_file() {
                continue; // seeking past the last decodable frame
            }
            if sheet {
                tiles.push(name);
            } else {
                let dest = cx.output_named(&format!("{}-{}-{:0digits$}.{fmt}", input.stem(), cx.suffix(), i + 1));
                move_file(&tmp.join(&name), &dest)?;
                outs.push(Output::new(dest).with_note(units::timecode(*t)));
            }
        }
        if sheet {
            anyhow::ensure!(!tiles.is_empty(), tr("media.err.no_frame"));
            let cols = v.int("columns").clamp(1, 12) as usize;
            let rows = tiles.len().div_ceil(cols);
            let list: String = tiles.iter().map(|t| format!("file '{t}'\nduration 1\n")).collect();
            std::fs::write(tmp.join("tiles.txt"), list)?;
            let name = format!("sheet.{fmt}");
            Ffmpeg::new()
                .args(["-f", "concat", "-safe", "0", "-i", "tiles.txt"])
                .args(["-vf", &format!("tile={cols}x{rows}:padding=6:margin=12:color=white")])
                .args(["-frames:v", "1", "-update", "1"])
                .args(quality_args(&fmt))
                .arg(&name)
                .cwd(&tmp)
                .quiet()
                .run(cx)?;
            let dest = cx.output_named(&format!("{}-sheet.{fmt}", input.stem()));
            move_file(&tmp.join(&name), &dest)?;
            outs.push(Output::new(dest).with_note(format!("{cols} × {rows} · {}", units::duration(dur))));
        }
        anyhow::ensure!(!outs.is_empty(), tr("media.err.no_frame"));
        Ok(outs)
    }
}

fn quality_args(fmt: &str) -> Vec<String> {
    match fmt {
        "jpg" => vec!["-q:v".into(), "2".into()],
        "webp" => vec!["-c:v".into(), "libwebp".into(), "-quality".into(), "90".into()],
        _ => vec![],
    }
}

/// Rename, or copy + delete across volumes.
pub fn move_file(from: &Path, to: &Path) -> Result<()> {
    if std::fs::rename(from, to).is_err() {
        std::fs::copy(from, to)?;
        let _ = std::fs::remove_file(from);
    }
    Ok(())
}
