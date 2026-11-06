//! `video.join`: concatenate in queue order. Stream copy when every file matches, else a
//! normalising re-encode (size, frame rate, audio layout) at an explicit quality.

use castor_core::options::{Choice, Cond, OptionSpec, Values};
use castor_core::{Family, FileInfo, InputSpec, Job, Output, Result, RunCx, Tool, ToolMeta, tr, tr_args, units};
use engine_media::{Ffmpeg, Probe};

use crate::common::*;

pub struct Join {
    meta: ToolMeta,
}

impl Join {
    pub fn new() -> Self {
        Join {
            meta: meta(
                "video.join",
                Family::Video,
                InputSpec::kinds(VIDEO).min(2),
                "MP4 · MKV · MOV",
                &["merge", "combine", "concatenate", "append", "stitch", "concat"],
                "joined",
                true,
            ),
        }
    }
}

/// Can these files be joined with the concat demuxer and stream copy?
pub fn copy_compatible(ps: &[Probe], exts: &[String]) -> bool {
    let a = &ps[0];
    ps.iter().zip(exts).all(|(p, e)| {
        e == &exts[0]
            && p.has_video
            && p.video_codec == a.video_codec
            && p.width == a.width
            && p.height == a.height
            && p.pix_fmt == a.pix_fmt
            && p.rotation == a.rotation
            && (p.fps - a.fps).abs() < 0.01
            && p.has_audio == a.has_audio
            && p.audio_tracks == a.audio_tracks
            && p.audio_codec == a.audio_codec
            && p.sample_rate == a.sample_rate
            && p.channels == a.channels
    })
}

/// Concat-demuxer list line for a path (single quotes escaped).
pub fn concat_line(p: &std::path::Path) -> String {
    let s = p.display().to_string().replace('\\', "/").replace('\'', "'\\''");
    format!("file '{s}'\n")
}

impl Tool for Join {
    fn meta(&self) -> &ToolMeta {
        &self.meta
    }
    fn options(&self) -> Vec<OptionSpec> {
        vec![
            OptionSpec::choice("mode", vec![Choice::key("auto"), Choice::key("reencode")], "auto"),
            OptionSpec::choice("size", vec![Choice::key("first"), Choice::key("largest")], "first").when(Cond::Eq("mode", "reencode")),
            quality_option("best"),
            codec_option("h264").advanced(),
        ]
    }
    fn plan(&self, inputs: &[FileInfo], _v: &Values) -> Result<Vec<Job>> {
        anyhow::ensure!(inputs.len() >= 2, tr("media.err.need_two"));
        Ok(vec![Job::many(inputs.to_vec())])
    }
    fn unavailable(&self) -> Option<String> {
        need_ffmpeg()
    }
    fn run(&self, job: &Job, v: &Values, cx: &RunCx) -> Result<Vec<Output>> {
        anyhow::ensure!(job.inputs.len() >= 2, tr("media.err.need_two"));
        let mut ps = Vec::new();
        for f in &job.inputs {
            let p = probe(f).map_err(|e| anyhow::anyhow!("{}: {e}", f.name()))?;
            need_video(&p).map_err(|e| anyhow::anyhow!("{}: {e}", f.name()))?;
            ps.push(p);
        }
        let exts: Vec<String> = job.inputs.iter().map(ext_of).collect();
        let total: f64 = ps.iter().map(|p| p.duration).sum();
        let first = &job.inputs[0];
        if v.str("mode") == "auto" && copy_compatible(&ps, &exts) {
            let ext = exts[0].clone();
            let list = cx.tmp_dir().join("list.txt");
            let mut s = String::from("ffconcat version 1.0\n");
            for f in &job.inputs {
                s.push_str(&concat_line(&std::path::absolute(&f.path)?));
            }
            std::fs::write(&list, s)?;
            let out = Partial::new(cx.output_path(Some(first), &ext));
            Ffmpeg::new()
                .args(["-f", "concat", "-safe", "0"])
                .input(&list)
                .args(["-map", "0:v:0", "-map", "0:a?", "-c", "copy"])
                .args(mux_args(&ext))
                .arg(out.path())
                .duration(total)
                .run(cx)?;
            let size = size_of(out.path());
            let note = tr_args("media.note.joined_copy", &[("count", &job.inputs.len().to_string()), ("duration", &units::duration(total))]);
            return Ok(vec![Output::new(out.keep()).with_note(format!("{note} · {}", units::bytes(size)))]);
        }
        // Re-encode: normalise every input to one size / frame rate / audio layout.
        let pick = if v.str("size") == "largest" {
            ps.iter().max_by_key(|p| p.display_size().0 as u64 * p.display_size().1 as u64).unwrap()
        } else {
            &ps[0]
        };
        let (w, h) = pick.display_size();
        let (w, h) = (w / 2 * 2, h / 2 * 2);
        let fps = if pick.fps > 0.0 && pick.fps <= 120.0 { pick.fps } else { 30.0 };
        let codec = VCodec::parse(v.str("codec"));
        let ext = output_ext(&exts[0], codec);
        let enc = VideoEnc::quality(codec, level(v));
        let any_audio = ps.iter().any(|p| p.has_audio);
        let mut fc = String::new();
        let mut cat = String::new();
        for (i, p) in ps.iter().enumerate() {
            fc.push_str(&format!(
                "[{i}:v:0]scale={w}:{h}:force_original_aspect_ratio=decrease:flags=lanczos,pad={w}:{h}:(ow-iw)/2:(oh-ih)/2:color=black,setsar=1,fps={fps:.4},format=yuv420p[v{i}];"
            ));
            if any_audio {
                if p.has_audio {
                    fc.push_str(&format!("[{i}:a:0]aformat=sample_rates=48000:channel_layouts=stereo,aresample=async=1[a{i}];"));
                } else {
                    fc.push_str(&format!("anullsrc=r=48000:cl=stereo,atrim=duration={:.3}[a{i}];", p.duration.max(0.04)));
                }
                cat.push_str(&format!("[v{i}][a{i}]"));
            } else {
                cat.push_str(&format!("[v{i}]"));
            }
        }
        let n = ps.len();
        if any_audio {
            fc.push_str(&format!("{cat}concat=n={n}:v=1:a=1[vc][ac];[vc]"));
        } else {
            fc.push_str(&format!("{cat}concat=n={n}:v=1:a=0[vc];[vc]"));
        }
        let mut vf = Vec::new();
        finish_filters(pick, &enc, &mut vf);
        fc.push_str(&vf.join(","));
        fc.push_str("[vout]");
        let script = cx.tmp_dir().join("graph.txt");
        std::fs::write(&script, &fc)?;
        let out = Partial::new(cx.output_path(Some(first), &ext));
        let mut ff = Ffmpeg::new();
        for f in &job.inputs {
            ff = ff.input(&f.path);
        }
        let (va, _) = video_args(pick, &enc);
        ff = ff.arg("-/filter_complex").arg(&script).args(["-map", "[vout]"]);
        if any_audio {
            ff = ff.args(["-map", "[ac]"]).args(audio_args(&ext, 192, pick));
        }
        ff.args(va).args(mux_args(&ext)).arg(out.path()).duration(total).run(cx)?;
        let size = size_of(out.path());
        let note = tr_args("media.note.joined_reencode", &[("count", &n.to_string()), ("w", &w.to_string()), ("h", &h.to_string())]);
        Ok(vec![Output::new(out.keep()).with_note(format!("{note} · {}", units::bytes(size)))])
    }
}
