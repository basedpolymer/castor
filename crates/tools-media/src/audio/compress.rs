//! `audio.compress`: smaller audio files, same format family when possible, never larger.

use castor_core::options::{Choice, OptionSpec, Values};
use castor_core::{Family, InputSpec, Job, Output, Result, RunCx, Tool, ToolMeta};
use engine_media::Ffmpeg;

use super::convert::audio_encode;
use crate::common::*;
use crate::video::compress::finish_compressed;

pub struct AudioCompress {
    meta: ToolMeta,
}

impl AudioCompress {
    pub fn new() -> Self {
        AudioCompress {
            meta: meta(
                "audio.compress",
                Family::Audio,
                InputSpec::kinds(AUDIO),
                "MP3 · M4A · OPUS",
                &["shrink", "smaller", "reduce", "bitrate", "podcast", "voice memo", "email"],
                "compressed",
                false,
            ),
        }
    }
}

/// Target format for "keep": the same family for lossy inputs, AAC for lossless ones.
pub fn keep_format(ext: &str, codec: &str) -> &'static str {
    match (ext, codec) {
        ("mp3", _) | (_, "mp3") => "mp3",
        ("opus", _) | (_, "opus") => "opus",
        ("ogg" | "oga", _) | (_, "vorbis") => "ogg",
        _ => "m4a",
    }
}

impl Tool for AudioCompress {
    fn meta(&self) -> &ToolMeta {
        &self.meta
    }
    fn options(&self) -> Vec<OptionSpec> {
        vec![
            OptionSpec::choice("content", vec![Choice::key("voice"), Choice::key("music")], "music"),
            OptionSpec::choice("quality", vec![Choice::key("high"), Choice::key("balanced"), Choice::key("small")], "balanced"),
            OptionSpec::choice(
                "format",
                vec![Choice::key("keep"), Choice::lit("mp3", "MP3"), Choice::lit("m4a", "M4A (AAC)"), Choice::lit("opus", "Opus")],
                "keep",
            ),
        ]
    }
    fn unavailable(&self) -> Option<String> {
        need_ffmpeg()
    }
    fn run(&self, job: &Job, v: &Values, cx: &RunCx) -> Result<Vec<Output>> {
        let input = job.input()?;
        let p = probe(input)?;
        need_audio(&p)?;
        let fmt = match v.str("format") {
            "keep" => keep_format(&input.extension(), p.audio_codec.as_deref().unwrap_or("")),
            "mp3" => "mp3",
            "opus" => "opus",
            _ => "m4a",
        };
        let voice = v.str("content") == "voice";
        let q = match v.str("quality") {
            "high" => 0,
            "small" => 2,
            _ => 1,
        };
        // Voice is mono and needs far fewer bits; Opus is efficient at low rates.
        let kbps: u32 = match (voice, fmt == "opus") {
            (true, true) => [48, 32, 24][q],
            (true, false) => [96, 64, 48][q],
            (false, true) => [128, 96, 64][q],
            (false, false) => [192, 128, 96][q],
        };
        let channels = if voice { Some(1) } else { None };
        let rate = if voice && fmt != "opus" { Some(if q == 2 { 22050 } else { 44100 }) } else { None };
        let ext = crate::video::extract_audio::format_ext(fmt);
        let out = Partial::new(cx.output_path(Some(input), ext));
        Ffmpeg::new().input(&input.path).args(audio_encode(&p, fmt, kbps, rate, channels)).arg(out.path()).duration(p.duration).run(cx)?;
        finish_compressed(input, out, cx)
    }
}
