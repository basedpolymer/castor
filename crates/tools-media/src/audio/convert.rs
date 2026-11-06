//! `audio.convert`: MP3, M4A, WAV, FLAC, OGG, Opus, AIFF, ALAC; bit depth kept for lossless
//! targets, cover art and tags carried over where the format allows.

use castor_core::options::{Choice, Cond, OptionSpec, Values};
use castor_core::{Family, InputSpec, Job, Output, Result, RunCx, Tool, ToolMeta, units};
use engine_media::{Ffmpeg, Probe, StreamKind};

use crate::common::*;
use crate::video::extract_audio::{encode_args, format_ext};

pub struct AudioConvert {
    meta: ToolMeta,
}

impl AudioConvert {
    pub fn new() -> Self {
        AudioConvert {
            meta: meta(
                "audio.convert",
                Family::Audio,
                InputSpec::kinds(AUDIO),
                "MP3 · M4A · WAV · FLAC · OGG · OPUS · AIFF",
                &["mp3", "wav", "flac", "m4a", "aac", "ogg", "opus", "aiff", "alac", "format", "transcode"],
                "",
                false,
            ),
        }
    }
}

pub fn format_choices() -> Vec<Choice> {
    vec![
        Choice::lit("mp3", "MP3"),
        Choice::lit("m4a", "M4A (AAC)"),
        Choice::lit("wav", "WAV"),
        Choice::lit("flac", "FLAC"),
        Choice::lit("ogg", "OGG (Vorbis)"),
        Choice::lit("opus", "Opus"),
        Choice::lit("aiff", "AIFF"),
        Choice::lit("alac", "ALAC (M4A)"),
    ]
}

/// Common audio-encode arguments: stream map with cover art, tags, rate, channels, codec.
pub fn audio_encode(p: &Probe, fmt: &str, kbps: u32, rate: Option<u32>, channels: Option<u32>) -> Vec<String> {
    let mut a: Vec<String> = vec!["-map".into(), "0:a:0".into()];
    let ext = format_ext(fmt);
    let cover_ok = matches!(ext, "mp3" | "m4a" | "flac");
    if let Some(pic) = p.streams.iter().find(|s| s.kind == StreamKind::Picture).filter(|_| cover_ok) {
        a.extend(["-map".into(), format!("0:{}", pic.index), "-c:v".into(), "copy".into(), "-disposition:v:0".into(), "attached_pic".into()]);
    } else {
        a.push("-vn".into());
    }
    a.extend(["-sn".into(), "-dn".into(), "-map_metadata".into(), "0".into()]);
    let ch = channels.unwrap_or(p.channels);
    if let Some(c) = channels {
        a.extend(["-ac".into(), c.to_string()]);
    }
    // Opus only takes 48 kHz-family rates; MP3 tops out at 48 kHz.
    let rate = match (fmt, rate) {
        ("opus", _) => None,
        ("mp3", None) if p.sample_rate > 48000 => Some(48000),
        (_, r) => r,
    };
    if let Some(r) = rate {
        a.extend(["-ar".into(), r.to_string()]);
    }
    let bits = if engine_media_lossless(p) { p.audio_bits.max(16) } else { 16 };
    a.extend(encode_args(fmt, kbps, bits, ch));
    if ext == "mp3" {
        a.extend(["-id3v2_version".into(), "3".into()]);
    }
    if ext == "m4a" {
        a.extend(["-movflags".into(), "+faststart".into()]);
    }
    a
}

fn engine_media_lossless(p: &Probe) -> bool {
    let c = p.audio_codec.as_deref().unwrap_or("");
    c.starts_with("pcm_") || matches!(c, "flac" | "alac" | "wavpack" | "ape" | "tta")
}

impl Tool for AudioConvert {
    fn meta(&self) -> &ToolMeta {
        &self.meta
    }
    fn options(&self) -> Vec<OptionSpec> {
        let lossy = || Cond::Ne("format", "wav");
        vec![
            OptionSpec::choice("format", format_choices(), "mp3").common(),
            OptionSpec::choice(
                "bitrate",
                vec![
                    Choice::lit("96", "96 kbps"),
                    Choice::lit("128", "128 kbps"),
                    Choice::lit("192", "192 kbps"),
                    Choice::lit("256", "256 kbps"),
                    Choice::lit("320", "320 kbps"),
                ],
                "192",
            )
            .when(lossy()),
            OptionSpec::choice(
                "rate",
                vec![Choice::key("keep"), Choice::lit("44100", "44.1 kHz"), Choice::lit("48000", "48 kHz"), Choice::lit("96000", "96 kHz")],
                "keep",
            )
            .advanced(),
            OptionSpec::choice("channels", vec![Choice::key("keep"), Choice::key("mono"), Choice::key("stereo")], "keep").advanced(),
        ]
    }
    fn unavailable(&self) -> Option<String> {
        need_ffmpeg()
    }
    fn run(&self, job: &Job, v: &Values, cx: &RunCx) -> Result<Vec<Output>> {
        let input = job.input()?;
        let p = probe(input)?;
        need_audio(&p)?;
        let fmt = v.str("format");
        let kbps: u32 = v.str("bitrate").parse().unwrap_or(192);
        let rate = v.str("rate").parse::<u32>().ok();
        let channels = match v.str("channels") {
            "mono" => Some(1),
            "stereo" => Some(2),
            _ => None,
        };
        let ext = format_ext(fmt);
        let out = Partial::new(cx.output_path(Some(input), ext));
        Ffmpeg::new().input(&input.path).args(audio_encode(&p, fmt, kbps, rate, channels)).arg(out.path()).duration(p.duration).run(cx)?;
        let q = engine_media::probe(out.path()).ok();
        let size = size_of(out.path());
        let detail = q
            .map(|q| {
                let bits = if q.audio_bits > 0 { format!(" · {}-bit", q.audio_bits) } else { String::new() };
                format!("{} · {} Hz{bits}", q.audio_codec.unwrap_or_default(), q.sample_rate)
            })
            .unwrap_or_default();
        Ok(vec![Output::new(out.keep()).with_note(format!("{} · {detail}", units::bytes(size)))])
    }
}
