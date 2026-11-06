//! `video.extract-audio`: the soundtrack as an audio file, losslessly when possible.

use castor_core::options::{Choice, Cond, OptionSpec, Values};
use castor_core::{Family, InputSpec, Job, Output, Result, RunCx, Tool, ToolMeta, tr_args};
use engine_media::{Ffmpeg, Probe, Stream};

use crate::common::*;

pub struct ExtractAudio {
    meta: ToolMeta,
}

impl ExtractAudio {
    pub fn new() -> Self {
        ExtractAudio {
            meta: meta(
                "video.extract-audio",
                Family::Video,
                InputSpec::kinds(VIDEO),
                "MP3 · M4A · WAV · FLAC · OPUS",
                &["soundtrack", "mp3", "audio", "rip", "sound", "music"],
                "audio",
                false,
            ),
        }
    }
}

/// Container that holds a codec untouched: (extension, copy?).
pub fn natural_container(codec: &str) -> (&'static str, bool) {
    match codec {
        "aac" | "alac" => ("m4a", true),
        "mp3" => ("mp3", true),
        "opus" => ("opus", true),
        "vorbis" => ("ogg", true),
        "flac" => ("flac", true),
        c if c.starts_with("pcm_") => ("wav", true),
        "ac3" => ("ac3", true),
        "eac3" => ("eac3", true),
        "" => ("m4a", false),
        _ => ("mka", true),
    }
}

/// Encoder args for an audio format (`bitrate` kbps for lossy formats).
pub fn encode_args(fmt: &str, kbps: u32, p_bits: u32, channels: u32) -> Vec<String> {
    let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
    match fmt {
        "mp3" => s(&["-c:a", "libmp3lame", "-b:a", &format!("{kbps}k")]),
        "wav" => s(&["-c:a", super::trim::pcm_codec(p_bits)]),
        "aiff" => s(&["-c:a", &super::trim::pcm_codec(p_bits).replace("le", "be")]),
        "flac" => {
            let mut a = s(&["-c:a", "flac", "-compression_level", "8"]);
            if p_bits > 16 {
                a.extend(s(&["-sample_fmt", "s32"]));
            }
            a
        }
        "alac" => {
            let mut a = s(&["-c:a", "alac"]);
            if p_bits > 16 {
                a.extend(s(&["-sample_fmt", "s32p"]));
            }
            a
        }
        "opus" => {
            let mut a = s(&["-c:a", "libopus", "-b:a", &format!("{}k", kbps.min(256))]);
            if channels > 2 {
                a.extend(s(&["-mapping_family", "1"]));
            }
            a
        }
        "ogg" => s(&["-c:a", "libvorbis", "-b:a", &format!("{kbps}k")]),
        _ => s(&["-c:a", "aac", "-b:a", &format!("{kbps}k")]),
    }
}

/// File extension of an audio format option value.
pub fn format_ext(fmt: &str) -> &str {
    match fmt {
        "aac" | "alac" => "m4a",
        "aiff" => "aiff",
        f => f,
    }
}

impl Tool for ExtractAudio {
    fn meta(&self) -> &ToolMeta {
        &self.meta
    }
    fn options(&self) -> Vec<OptionSpec> {
        vec![
            OptionSpec::choice(
                "format",
                vec![
                    Choice::key("auto"),
                    Choice::lit("mp3", "MP3"),
                    Choice::lit("m4a", "M4A (AAC)"),
                    Choice::lit("wav", "WAV"),
                    Choice::lit("flac", "FLAC"),
                    Choice::lit("opus", "Opus"),
                ],
                "auto",
            ),
            OptionSpec::choice(
                "bitrate",
                vec![Choice::lit("128", "128 kbps"), Choice::lit("192", "192 kbps"), Choice::lit("256", "256 kbps"), Choice::lit("320", "320 kbps")],
                "192",
            )
            .when(Cond::Ne("format", "auto")),
            OptionSpec::choice("tracks", vec![Choice::key("first"), Choice::key("all")], "first"),
        ]
    }
    fn unavailable(&self) -> Option<String> {
        need_ffmpeg()
    }
    fn run(&self, job: &Job, v: &Values, cx: &RunCx) -> Result<Vec<Output>> {
        let input = job.input()?;
        let p = probe(input)?;
        need_audio(&p)?;
        let streams: Vec<&Stream> = if v.str("tracks") == "all" { p.audio_streams().collect() } else { p.audio_streams().take(1).collect() };
        let n = streams.len();
        let mut outs = Vec::new();
        for (i, s) in streams.iter().enumerate() {
            cx.check_cancel()?;
            let (lo, hi) = (i as f32 / n as f32, (i + 1) as f32 / n as f32);
            outs.push(extract_one(input, &p, s, v, cx, (lo, hi), if n > 1 { Some(i + 1) } else { None })?);
        }
        Ok(outs)
    }
}

fn extract_one(
    input: &castor_core::FileInfo,
    p: &Probe,
    s: &Stream,
    v: &Values,
    cx: &RunCx,
    range: (f32, f32),
    n: Option<usize>,
) -> Result<Output> {
    let fmt = v.str("format");
    let kbps: u32 = v.str("bitrate").parse().unwrap_or(192);
    let (ext, args): (String, Vec<String>) = if fmt == "auto" {
        let (e, copy) = natural_container(&s.codec);
        if copy { (e.to_string(), vec!["-c:a".into(), "copy".into()]) } else { ("m4a".into(), encode_args("m4a", 192, 0, s.channels)) }
    } else {
        let bits = if engine_media_lossless(&s.codec) { s.bits.max(16) } else { 16 };
        // Same codec as requested: copy instead of re-encoding (no generation loss).
        let same = matches!((fmt, s.codec.as_str()), ("mp3", "mp3") | ("m4a", "aac") | ("opus", "opus") | ("flac", "flac"));
        if same {
            (format_ext(fmt).to_string(), vec!["-c:a".into(), "copy".into()])
        } else {
            (format_ext(fmt).to_string(), encode_args(fmt, kbps, bits, s.channels))
        }
    };
    let name = match (n, s.language.as_deref()) {
        (Some(i), Some(l)) => format!("{}-{}-{i}-{l}.{ext}", input.stem(), cx.suffix()),
        (Some(i), None) => format!("{}-{}-{i}.{ext}", input.stem(), cx.suffix()),
        _ => format!("{}-{}.{ext}", input.stem(), cx.suffix()),
    };
    let out = Partial::new(cx.output_named(&name));
    let mut ff = Ffmpeg::new()
        .input(&input.path)
        .args(["-map", &format!("0:{}", s.index), "-vn", "-sn", "-dn", "-map_metadata", "0"])
        .args(args);
    if ext == "m4a" {
        ff = ff.args(["-movflags", "+faststart"]);
    }
    if ext == "mp3" {
        ff = ff.args(["-id3v2_version", "3"]);
    }
    ff.arg(out.path()).duration(p.duration).range(range.0, range.1).run(cx)?;
    let size = size_of(out.path());
    let path = out.keep();
    let note = tr_args("media.note.audio_track", &[("codec", &s.codec), ("size", &castor_core::units::bytes(size))]);
    Ok(Output::new(path).with_note(note))
}

fn engine_media_lossless(c: &str) -> bool {
    c.starts_with("pcm_") || matches!(c, "flac" | "alac" | "wavpack" | "truehd")
}
