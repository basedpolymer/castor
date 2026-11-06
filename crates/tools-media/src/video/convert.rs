//! `video.convert`: change container and/or codec; remux without re-encoding when possible.

use castor_core::options::{Choice, Cond, OptionSpec, Values};
use castor_core::{Family, InputSpec, Job, Output, Result, RunCx, Tool, ToolMeta, tr};
use engine_media::Ffmpeg;

use crate::common::*;

pub struct Convert {
    meta: ToolMeta,
}

impl Convert {
    pub fn new() -> Self {
        Convert {
            meta: meta(
                "video.convert",
                Family::Video,
                InputSpec::kinds(VIDEO),
                "MP4 · MOV · MKV · WEBM · AVI · GIF",
                &["format", "mp4", "mov", "mkv", "webm", "avi", "remux", "transcode", "h264", "hevc", "prores"],
                "",
                true,
            ),
        }
    }
}

impl Tool for Convert {
    fn meta(&self) -> &ToolMeta {
        &self.meta
    }
    fn options(&self) -> Vec<OptionSpec> {
        vec![
            OptionSpec::choice(
                "format",
                vec![
                    Choice::lit("mp4", "MP4"),
                    Choice::lit("mov", "MOV"),
                    Choice::lit("mkv", "MKV"),
                    Choice::lit("webm", "WebM"),
                    Choice::lit("avi", "AVI"),
                    Choice::lit("gif", "GIF"),
                ],
                "mp4",
            )
            .common(),
            OptionSpec::choice(
                "codec",
                vec![
                    Choice::key("auto"),
                    Choice::lit("h264", "H.264"),
                    Choice::lit("h265", "H.265 (HEVC)"),
                    Choice::lit("av1", "AV1"),
                    Choice::lit("vp9", "VP9"),
                    Choice::lit("prores", "ProRes 422"),
                ],
                "auto",
            )
            .when(Cond::Ne("format", "gif")),
            OptionSpec::toggle("remux", true).when(Cond::Ne("format", "gif")),
            quality_option("best").when(Cond::Ne("format", "gif")),
            hardware_option().when(Cond::Ne("format", "gif")),
        ]
    }
    fn unavailable(&self) -> Option<String> {
        need_ffmpeg()
    }
    fn run(&self, job: &Job, v: &Values, cx: &RunCx) -> Result<Vec<Output>> {
        let input = job.input()?;
        let fmt = v.str("format").to_string();
        if fmt == "gif" {
            return super::gif::make_gif(input, &super::gif::GifSettings::preset("balanced"), None, None, cx);
        }
        let p = probe(input)?;
        need_video(&p)?;
        let src = p.video_codec.as_deref().unwrap_or("");
        let codec = match v.str("codec") {
            "auto" => match VCodec::from_probe(src) {
                Some(c) if container_ok(&fmt, c) => c,
                _ if fmt == "webm" => VCodec::Vp9,
                _ => VCodec::H264,
            },
            c => VCodec::parse(c),
        };
        anyhow::ensure!(container_ok(&fmt, codec), tr("media.err.codec_container"));
        let can_remux = v.bool("remux")
            && src == codec.ffprobe_name()
            && (!p.has_audio || p.audio_streams().all(|s| audio_copy_ok(&fmt, &s.codec)))
            && !(fmt == "avi" && src != "h264");
        let out = Partial::new(cx.output_path(Some(input), &fmt));
        let mut note = None;
        if can_remux {
            Ffmpeg::new()
                .input(&input.path)
                .args(stream_map(&p, &fmt, true, true))
                .args(["-c:v", "copy", "-c:a", "copy"])
                .args(if codec == VCodec::Hevc && fmt != "mkv" { vec!["-tag:v", "hvc1"] } else { vec![] })
                .args(mux_args(&fmt))
                .arg(out.path())
                .duration(p.duration)
                .run(cx)?;
            note = Some(tr("media.note.remuxed"));
        } else {
            let enc = VideoEnc { hardware: v.bool("hardware"), ..VideoEnc::quality(codec, level(v)) };
            let audio = if p.has_audio && p.audio_streams().all(|s| audio_copy_ok(&fmt, &s.codec)) && fmt != "avi" {
                vec!["-c:a".to_string(), "copy".to_string()]
            } else {
                audio_args(&fmt, 192, &p)
            };
            encode_with_fallback(cx, &enc, |e| {
                let mut vf = Vec::new();
                finish_filters(&p, e, &mut vf);
                let (va, _) = video_args(&p, e);
                Ffmpeg::new()
                    .input(&input.path)
                    .args(stream_map(&p, &fmt, true, true))
                    .args(["-vf", &vf.join(",")])
                    .args(va)
                    .args(audio.clone())
                    .args(mux_args(&fmt))
                    .arg(out.path())
                    .duration(p.duration)
            })?;
        }
        let size = size_of(out.path());
        let path = out.keep();
        let sizes = size_note(input.size, size);
        Ok(vec![Output::new(path).with_note(match note {
            Some(n) => format!("{n} · {sizes}"),
            None => sizes,
        })])
    }
}
