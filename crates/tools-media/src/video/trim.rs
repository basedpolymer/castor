//! `video.trim` / `audio.trim`: keep a range, cut fast at keyframes or exact with re-encoding.

use castor_core::options::{Choice, Cond, OptionSpec, Values};
use castor_core::{Family, InputSpec, Job, Output, Result, RunCx, Tool, ToolMeta, tr, tr_args, units};
use engine_media::Ffmpeg;

use crate::common::*;

pub struct Trim {
    meta: ToolMeta,
    audio: bool,
}

impl Trim {
    pub fn video() -> Self {
        Trim {
            meta: meta(
                "video.trim",
                Family::Video,
                InputSpec::kinds(VIDEO),
                "MP4 · MOV · MKV · WEBM",
                &["cut", "clip", "shorten", "start", "end", "range"],
                "trimmed",
                true,
            ),
            audio: false,
        }
    }
    pub fn audio() -> Self {
        Trim {
            meta: meta(
                "audio.trim",
                Family::Audio,
                InputSpec::kinds(AUDIO),
                "MP3 · M4A · WAV · FLAC",
                &["cut", "clip", "shorten", "start", "end", "range", "ringtone"],
                "trimmed",
                false,
            ),
            audio: true,
        }
    }
}

/// Resolve start/end options against a duration. Errors are user-facing.
pub fn resolve_range(v: &Values, duration: f64) -> anyhow::Result<(f64, f64)> {
    let start = time_opt(v, "start")?.unwrap_or(0.0);
    let end = if v.str("until") == "duration" {
        time_opt(v, "length")?.map(|d| start + d)
    } else {
        time_opt(v, "end")?
    };
    let end = end.unwrap_or(duration);
    let end = if duration > 0.0 { end.min(duration) } else { end };
    if duration > 0.0 && start >= duration {
        anyhow::bail!(tr_args("media.err.start_after_end_of_file", &[("start", &units::timecode(start)), ("duration", &units::timecode(duration))]));
    }
    anyhow::ensure!(end > start + 0.01, tr("media.err.empty_range"));
    Ok((start, end))
}

pub fn range_options() -> Vec<OptionSpec> {
    vec![
        OptionSpec::text("start", "0:00").placeholder("media.ph.time"),
        OptionSpec::choice("until", vec![Choice::key("end"), Choice::key("duration")], "end"),
        OptionSpec::text("end", "").placeholder("media.ph.time_end").when(Cond::Eq("until", "end")),
        OptionSpec::text("length", "").placeholder("media.ph.time").when(Cond::Eq("until", "duration")),
    ]
}

impl Tool for Trim {
    fn meta(&self) -> &ToolMeta {
        &self.meta
    }
    fn options(&self) -> Vec<OptionSpec> {
        let mut o = range_options();
        o.push(cut_option());
        o
    }
    fn unavailable(&self) -> Option<String> {
        need_ffmpeg()
    }
    fn run(&self, job: &Job, v: &Values, cx: &RunCx) -> Result<Vec<Output>> {
        let input = job.input()?;
        let p = probe(input)?;
        if self.audio {
            need_audio(&p)?;
        } else {
            need_video(&p)?;
        }
        let (start, end) = resolve_range(v, p.duration)?;
        let len = end - start;
        let fast = v.str("cut") == "fast";
        let ext = if fast { ext_of(input) } else { reencode_ext(&ext_of(input), !self.audio && p.has_video) };
        let out = Partial::new(cx.output_path(Some(input), &ext));
        let mut ff = Ffmpeg::new().args(["-ss", &format!("{start:.3}")]).input(&input.path).args(["-t", &format!("{len:.3}")]);
        if fast {
            ff = ff.args(["-map", "0:v?", "-map", "0:a?", "-map", "0:s?", "-c", "copy", "-avoid_negative_ts", "make_zero"]);
            ff = ff.args(["-map_metadata", "0", "-map_chapters", "-1"]);
        } else if self.audio || !p.has_video {
            ff = ff.args(["-map", "0:a", "-vn", "-map_metadata", "0", "-map_chapters", "-1"]).args(audio_reencode(&p, &ext));
        } else {
            let enc = edit_encoder(&p, &ext);
            let mut vf = Vec::new();
            finish_filters(&p, &enc, &mut vf);
            let (va, _) = video_args(&p, &enc);
            ff = ff.args(stream_map(&p, &ext, true, true)).args(["-vf", &vf.join(",")]).args(va).args(edit_audio(&p, &ext, false));
        }
        ff.args(mux_args(&ext)).arg(out.path()).duration(len).run(cx)?;
        let size = size_of(out.path());
        let out = out.keep();
        let note = tr_args("media.note.trimmed", &[("from", &units::timecode(start)), ("to", &units::timecode(end)), ("size", &units::bytes(size))]);
        Ok(vec![Output::new(out).with_note(note)])
    }
}

/// Re-encode an audio stream into the same format family at high quality (exact cuts).
pub fn audio_reencode(p: &engine_media::Probe, ext: &str) -> Vec<String> {
    let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
    let src_kbps = (p.audio_bitrate / 1000).max(96) as u32;
    let kbps = src_kbps.clamp(128, 320);
    match ext {
        "mp3" => s(&["-c:a", "libmp3lame", "-b:a", &format!("{kbps}k")]),
        "wav" => s(&["-c:a", pcm_codec(p.audio_bits)]),
        "aif" | "aiff" => s(&["-c:a", &pcm_codec(p.audio_bits).replace("le", "be")]),
        "flac" => {
            let mut a = s(&["-c:a", "flac"]);
            if p.audio_bits > 16 {
                a.extend(s(&["-sample_fmt", "s32"]));
            }
            a
        }
        "ogg" | "oga" => s(&["-c:a", "libvorbis", "-q:a", "6"]),
        "opus" => s(&["-c:a", "libopus", "-b:a", &format!("{}k", kbps.min(256))]),
        "webm" => s(&["-c:a", "libopus", "-b:a", &format!("{}k", kbps.min(256))]),
        _ => {
            if p.audio_codec.as_deref() == Some("alac") {
                s(&["-c:a", "alac"])
            } else {
                s(&["-c:a", "aac", "-b:a", &format!("{kbps}k")])
            }
        }
    }
}

/// PCM codec keeping the source bit depth (16 / 24 / 32).
pub fn pcm_codec(bits: u32) -> &'static str {
    match bits {
        b if b > 24 => "pcm_s32le",
        b if b > 16 => "pcm_s24le",
        _ => "pcm_s16le",
    }
}
