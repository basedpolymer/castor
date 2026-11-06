//! `video.compress`: constant quality presets, target size (two-pass) or codec-preserving
//! re-encode. Never returns a file larger than the original.

use castor_core::options::{Choice, Cond, OptionSpec, Values};
use castor_core::{Family, FileInfo, InputSpec, Job, Output, Result, RunCx, Tool, ToolMeta, tr, tr_args};
use engine_media::{Ffmpeg, Probe};

use crate::common::*;

pub struct Compress {
    meta: ToolMeta,
}

impl Compress {
    pub fn new() -> Self {
        Compress {
            meta: meta(
                "video.compress",
                Family::Video,
                InputSpec::kinds(VIDEO),
                "MP4 · MKV · MOV · WEBM",
                &["shrink", "smaller", "reduce", "size", "email", "discord", "whatsapp", "h264", "hevc", "av1", "handbrake"],
                "compressed",
                true,
            ),
        }
    }
}

/// Scale filter limiting the short side of the displayed picture to `limit` pixels.
pub fn limit_filter(p: &Probe, limit: u32) -> Option<String> {
    let (w, h) = p.display_size();
    if limit == 0 || w.min(h) <= limit {
        return None;
    }
    Some(if w >= h { format!("scale=-2:{limit}:flags=lanczos") } else { format!("scale={limit}:-2:flags=lanczos") })
}

/// Copy `src` to a unique output path (the "kept the original" case).
pub fn keep_original(input: &FileInfo, cx: &RunCx) -> Result<Output> {
    let dest = cx.output_path(Some(input), &ext_of(input));
    std::fs::copy(&input.path, &dest)?;
    Ok(Output::new(dest).with_note(kept_original_note(input.size)))
}

impl Tool for Compress {
    fn meta(&self) -> &ToolMeta {
        &self.meta
    }
    fn options(&self) -> Vec<OptionSpec> {
        vec![
            OptionSpec::choice("mode", vec![Choice::key("quality"), Choice::key("size"), Choice::key("keep")], "quality"),
            quality_option("balanced").when(Cond::Ne("mode", "size")),
            OptionSpec::float("target", 1.0, 100_000.0, 25.0).unit("MB").when(Cond::Eq("mode", "size")),
            codec_option("h264").when(Cond::Ne("mode", "keep")),
            OptionSpec::choice(
                "max_res",
                vec![
                    Choice::key("original"),
                    Choice::lit("2160", "4K (2160p)"),
                    Choice::lit("1440", "1440p"),
                    Choice::lit("1080", "1080p"),
                    Choice::lit("720", "720p"),
                    Choice::lit("480", "480p"),
                ],
                "original",
            ),
            OptionSpec::choice(
                "audio",
                vec![Choice::lit("64", "64 kbps"), Choice::lit("96", "96 kbps"), Choice::lit("128", "128 kbps"), Choice::lit("160", "160 kbps")],
                "128",
            )
            .advanced(),
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
        let mode = v.str("mode");
        let in_ext = ext_of(input);
        let codec = if mode == "keep" {
            p.video_codec.as_deref().and_then(VCodec::from_probe).filter(|c| *c != VCodec::ProRes).unwrap_or(VCodec::H264)
        } else {
            VCodec::parse(v.str("codec"))
        };
        let ext = output_ext(&in_ext, codec);
        let kbps: u32 = v.str("audio").parse().unwrap_or(128);
        let dur = p.duration.max(0.1);
        let level = if mode == "keep" { 1 } else { level(v) };
        let mut enc = VideoEnc { hardware: v.bool("hardware"), ..VideoEnc::quality(codec, level) };

        // Target size: budget the video bitrate after audio and container overhead.
        if mode == "size" {
            let target = (v.float("target") * 1_000_000.0) as u64;
            if input.size <= target {
                return Ok(vec![keep_original(input, cx)?.with_note(tr("media.note.already_under_target"))]);
            }
            let tracks = p.audio_tracks.max(if p.has_audio { 1 } else { 0 }) as f64;
            let audio_bits = kbps as f64 * 1000.0 * dur * tracks;
            let total_bits = target as f64 * 8.0 * 0.96;
            let vb = ((total_bits - audio_bits) / dur) as i64;
            if vb < 60_000 {
                let min_mb = ((audio_bits + 60_000.0 * dur) / 8.0 / 0.96 / 1e6).ceil();
                anyhow::bail!(tr_args("media.err.target_too_small", &[("min", &format!("{min_mb:.0} MB"))]));
            }
            enc.bitrate = Some(vb as u64);
        }

        let mut vf: Vec<String> = Vec::new();
        if let Ok(limit) = v.str("max_res").parse::<u32>() {
            vf.extend(limit_filter(&p, limit));
        }
        finish_filters(&p, &enc, &mut vf);
        let vf = vf.join(",");

        let audio: Vec<String> = if !p.has_audio {
            vec![]
        } else if p.audio_streams().all(|s| audio_copy_ok(&ext, &s.codec) && s.bitrate > 0 && s.bitrate <= kbps as u64 * 1100 * (s.channels.max(2) as u64) / 2) {
            vec!["-c:a".into(), "copy".into()]
        } else {
            audio_args(&ext, kbps, &p)
        };

        let out = Partial::new(cx.output_path(Some(input), &ext));
        let tmp = std::path::absolute(cx.tmp_dir())?;
        let src = std::path::absolute(&input.path)?;
        let dst = std::path::absolute(out.path())?;
        let two_pass = enc.bitrate.is_some() && !enc.hardware && matches!(codec, VCodec::H264 | VCodec::Hevc | VCodec::Vp9);
        if two_pass {
            cx.message(tr("media.msg.pass1"));
            let e1 = VideoEnc { pass: Some((1, "passlog".into())), ..enc.clone() };
            let (va, _) = video_args(&p, &e1);
            Ffmpeg::new()
                .input(&src)
                .args(["-map", &format!("0:{}", p.video_index.unwrap_or(0)), "-an", "-sn", "-vf", &vf])
                .args(va)
                .args(["-f", "null", "-"])
                .cwd(&tmp)
                .duration(p.duration)
                .range(0.0, 0.45)
                .run(cx)?;
            cx.message(tr("media.msg.pass2"));
            let e2 = VideoEnc { pass: Some((2, "passlog".into())), ..enc.clone() };
            let (va, _) = video_args(&p, &e2);
            Ffmpeg::new()
                .input(&src)
                .args(stream_map(&p, &ext, true, true))
                .args(["-vf", &vf])
                .args(va)
                .args(audio.clone())
                .args(mux_args(&ext))
                .arg(&dst)
                .cwd(&tmp)
                .duration(p.duration)
                .range(0.45, 1.0)
                .run(cx)?;
        } else {
            encode_with_fallback(cx, &enc, |e| {
                let (va, _) = video_args(&p, e);
                Ffmpeg::new()
                    .input(&src)
                    .args(stream_map(&p, &ext, true, true))
                    .args(["-vf", &vf])
                    .args(va)
                    .args(audio.clone())
                    .args(mux_args(&ext))
                    .arg(&dst)
                    .duration(p.duration)
            })?;
        }
        finish_compressed(input, out, cx)
    }
}

/// Keep the result only if it is smaller than the original; otherwise copy the original.
pub fn finish_compressed(input: &FileInfo, out: Partial, cx: &RunCx) -> Result<Vec<Output>> {
    let size = size_of(out.path());
    if size == 0 || size >= input.size {
        drop(out);
        return Ok(vec![keep_original(input, cx)?]);
    }
    let path = out.keep();
    Ok(vec![Output::new(path).with_note(size_note(input.size, size))])
}
