//! `ffprobe` → [`Probe`].

use std::ffi::OsStr;
use std::path::Path;

use castor_core::{CancelToken, tr};
use serde_json::Value;

use crate::ff::{ffprobe, run_simple};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StreamKind {
    Video,
    Audio,
    Subtitle,
    /// Cover art / attached picture.
    Picture,
    Data,
}

/// One stream of a media file.
#[derive(Clone, Debug)]
pub struct Stream {
    /// Absolute index in the file (`-map 0:{index}`).
    pub index: usize,
    pub kind: StreamKind,
    pub codec: String,
    pub language: Option<String>,
    pub title: Option<String>,
    pub channels: u32,
    pub sample_rate: u32,
    pub bitrate: u64,
    /// Bits per sample of the decoded audio/video when known (16, 24, 10…).
    pub bits: u32,
    pub sample_fmt: String,
    pub default: bool,
}

impl Stream {
    /// Text subtitles (convertible to mov_text / SRT / ASS); bitmap ones (PGS, DVB) are not.
    pub fn is_text_subtitle(&self) -> bool {
        matches!(self.codec.as_str(), "subrip" | "srt" | "ass" | "ssa" | "mov_text" | "webvtt" | "text")
    }
}

/// What is inside a media file.
#[derive(Clone, Debug, Default)]
pub struct Probe {
    /// Seconds (0 if unknown).
    pub duration: f64,
    /// Coded width/height of the main video stream (before rotation).
    pub width: u32,
    pub height: u32,
    /// Average frame rate.
    pub fps: f64,
    /// Display rotation in degrees counter-clockwise, normalised to 0 / 90 / 180 / 270.
    pub rotation: i32,
    pub video_codec: Option<String>,
    pub audio_codec: Option<String>,
    pub audio_tracks: usize,
    pub subtitle_tracks: usize,
    pub has_video: bool,
    pub has_audio: bool,
    /// Overall bitrate in bits/s.
    pub bitrate: u64,
    pub video_bitrate: u64,
    pub audio_bitrate: u64,
    pub pix_fmt: String,
    /// Bits per colour component of the main video (8, 10, 12).
    pub bit_depth: u32,
    pub color_transfer: String,
    pub color_primaries: String,
    pub color_space: String,
    pub sample_rate: u32,
    pub channels: u32,
    /// Bits per audio sample when known (16, 24, 32), 0 for lossy codecs.
    pub audio_bits: u32,
    pub audio_sample_fmt: String,
    /// Container short names, e.g. "mov,mp4,m4a,3gp,3g2,mj2".
    pub format: String,
    /// r_frame_rate differs from avg_frame_rate noticeably: probably variable frame rate.
    pub vfr: bool,
    /// Index of the main video stream (not cover art).
    pub video_index: Option<usize>,
    pub streams: Vec<Stream>,
}

impl Probe {
    /// Width × height as displayed (rotation applied).
    pub fn display_size(&self) -> (u32, u32) {
        if self.rotation == 90 || self.rotation == 270 { (self.height, self.width) } else { (self.width, self.height) }
    }
    /// HDR transfer (PQ or HLG).
    pub fn is_hdr(&self) -> bool {
        matches!(self.color_transfer.as_str(), "smpte2084" | "arib-std-b67")
    }
    pub fn audio_streams(&self) -> impl Iterator<Item = &Stream> {
        self.streams.iter().filter(|s| s.kind == StreamKind::Audio)
    }
    pub fn subtitle_streams(&self) -> impl Iterator<Item = &Stream> {
        self.streams.iter().filter(|s| s.kind == StreamKind::Subtitle)
    }
    /// True if the container is MP4/MOV-like.
    pub fn is_mp4_family(&self) -> bool {
        self.format.contains("mp4") || self.format.contains("mov")
    }
}

fn num(v: &Value) -> Option<f64> {
    match v {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
}

fn ratio(s: &str) -> f64 {
    match s.split_once('/') {
        Some((a, b)) => {
            let (a, b): (f64, f64) = (a.parse().unwrap_or(0.0), b.parse().unwrap_or(0.0));
            if b > 0.0 { a / b } else { 0.0 }
        }
        None => s.parse().unwrap_or(0.0),
    }
}

/// Probe a media file with ffprobe. Errors are user-facing ("not a media file").
pub fn probe(path: &Path) -> anyhow::Result<Probe> {
    probe_cancel(path, &CancelToken::new())
}

pub(crate) fn probe_cancel(path: &Path, cancel: &CancelToken) -> anyhow::Result<Probe> {
    anyhow::ensure!(path.is_file(), tr("media.err.unreadable"));
    let fp = ffprobe()?;
    let out = run_simple(
        &fp,
        &[
            OsStr::new("-v"),
            OsStr::new("error"),
            OsStr::new("-show_format"),
            OsStr::new("-show_streams"),
            OsStr::new("-of"),
            OsStr::new("json"),
            path.as_os_str(),
        ],
        cancel,
    )
    .map_err(|e| {
        if e.is::<castor_core::job::Cancelled>() { e } else { anyhow::Error::msg(format!("{e}")).context(tr("media.err.unreadable")) }
    })?;
    let mut p = parse(&out)?;
    anyhow::ensure!(p.has_video || p.has_audio, tr("media.err.no_streams"));
    // Some muxers leave colour tags out of the container while the bitstream carries them:
    // for >8-bit video with unknown transfer, ask the first decoded frame (HDR detection).
    if p.has_video && p.bit_depth > 8 && matches!(p.color_transfer.as_str(), "" | "unknown") {
        if let Ok(fr) = run_simple(
            &fp,
            &[
                OsStr::new("-v"),
                OsStr::new("error"),
                OsStr::new("-select_streams"),
                OsStr::new("v:0"),
                OsStr::new("-read_intervals"),
                OsStr::new("%+#1"),
                OsStr::new("-show_entries"),
                OsStr::new("frame=color_transfer,color_primaries,color_space"),
                OsStr::new("-of"),
                OsStr::new("json"),
                path.as_os_str(),
            ],
            cancel,
        ) {
            if let Ok(v) = serde_json::from_str::<Value>(&fr) {
                let f = &v["frames"][0];
                let get = |k: &str| f[k].as_str().filter(|s| *s != "unknown").map(str::to_string);
                if let Some(t) = get("color_transfer") {
                    p.color_transfer = t;
                }
                if let Some(t) = get("color_primaries").filter(|_| matches!(p.color_primaries.as_str(), "" | "unknown")) {
                    p.color_primaries = t;
                }
                if let Some(t) = get("color_space").filter(|_| matches!(p.color_space.as_str(), "" | "unknown")) {
                    p.color_space = t;
                }
            }
        }
    }
    Ok(p)
}

/// Parse ffprobe JSON (`-show_format -show_streams -of json`).
pub fn parse(json: &str) -> anyhow::Result<Probe> {
    let v: Value = serde_json::from_str(json)?;
    let mut p = Probe::default();
    let fmt = &v["format"];
    p.format = fmt["format_name"].as_str().unwrap_or("").to_string();
    p.duration = num(&fmt["duration"]).unwrap_or(0.0);
    p.bitrate = num(&fmt["bit_rate"]).unwrap_or(0.0) as u64;
    let empty = Vec::new();
    for s in v["streams"].as_array().unwrap_or(&empty) {
        let codec = s["codec_name"].as_str().unwrap_or("").to_string();
        let attached = s["disposition"]["attached_pic"].as_i64() == Some(1);
        let kind = match s["codec_type"].as_str().unwrap_or("") {
            "video" if attached => StreamKind::Picture,
            // MJPEG/PNG "video" streams of 1 frame inside audio files are cover art too.
            "video" if matches!(codec.as_str(), "mjpeg" | "png" | "bmp") && s["nb_frames"].as_str() == Some("1") => {
                StreamKind::Picture
            }
            "video" => StreamKind::Video,
            "audio" => StreamKind::Audio,
            "subtitle" => StreamKind::Subtitle,
            _ => StreamKind::Data,
        };
        let tags = &s["tags"];
        let st = Stream {
            index: s["index"].as_u64().unwrap_or(0) as usize,
            kind,
            codec: codec.clone(),
            language: tags["language"].as_str().filter(|l| *l != "und").map(str::to_string),
            title: tags["title"].as_str().map(str::to_string),
            channels: s["channels"].as_u64().unwrap_or(0) as u32,
            sample_rate: num(&s["sample_rate"]).unwrap_or(0.0) as u32,
            bitrate: num(&s["bit_rate"]).or_else(|| num(&tags["BPS"])).unwrap_or(0.0) as u64,
            bits: num(&s["bits_per_raw_sample"]).or_else(|| num(&s["bits_per_sample"])).unwrap_or(0.0) as u32,
            sample_fmt: s["sample_fmt"].as_str().unwrap_or("").to_string(),
            default: s["disposition"]["default"].as_i64() == Some(1),
        };
        match kind {
            StreamKind::Video if p.video_index.is_none() => {
                p.has_video = true;
                p.video_index = Some(st.index);
                p.video_codec = Some(codec);
                p.width = s["width"].as_u64().unwrap_or(0) as u32;
                p.height = s["height"].as_u64().unwrap_or(0) as u32;
                let avg = ratio(s["avg_frame_rate"].as_str().unwrap_or("0/0"));
                let r = ratio(s["r_frame_rate"].as_str().unwrap_or("0/0"));
                p.fps = if avg > 0.0 { avg } else { r };
                p.vfr = avg > 0.0 && r > 0.0 && (avg - r).abs() / r > 0.02;
                p.pix_fmt = s["pix_fmt"].as_str().unwrap_or("").to_string();
                p.bit_depth = pix_depth(&p.pix_fmt).max(st.bits).max(8);
                p.color_transfer = s["color_transfer"].as_str().unwrap_or("").to_string();
                p.color_primaries = s["color_primaries"].as_str().unwrap_or("").to_string();
                p.color_space = s["color_space"].as_str().unwrap_or("").to_string();
                p.video_bitrate = st.bitrate;
                let mut rot = 0.0;
                if let Some(list) = s["side_data_list"].as_array() {
                    for sd in list {
                        if let Some(r) = num(&sd["rotation"]) {
                            rot = r;
                        }
                    }
                }
                if rot == 0.0 {
                    rot = num(&tags["rotate"]).map(|r| -r).unwrap_or(0.0);
                }
                p.rotation = (((rot.round() as i32) % 360) + 360) % 360;
                if p.duration <= 0.0 {
                    p.duration = num(&s["duration"]).unwrap_or(0.0);
                }
            }
            StreamKind::Audio => {
                p.audio_tracks += 1;
                if !p.has_audio {
                    p.has_audio = true;
                    p.audio_codec = Some(codec);
                    p.sample_rate = st.sample_rate;
                    p.channels = st.channels;
                    p.audio_bitrate = st.bitrate;
                    p.audio_sample_fmt = st.sample_fmt.clone();
                    p.audio_bits = if is_lossless_audio(p.audio_codec.as_deref().unwrap_or("")) {
                        if st.bits > 0 { st.bits } else { sample_fmt_bits(&st.sample_fmt) }
                    } else {
                        0
                    };
                    if p.duration <= 0.0 {
                        p.duration = num(&s["duration"]).unwrap_or(0.0);
                    }
                }
            }
            StreamKind::Subtitle => p.subtitle_tracks += 1,
            _ => {}
        }
        p.streams.push(st);
    }
    Ok(p)
}

fn pix_depth(pix: &str) -> u32 {
    if pix.contains("p16") || pix.contains("48") || pix.contains("64") {
        16
    } else if pix.contains("p12") {
        12
    } else if pix.contains("p10") || pix.contains("x2rgb10") || pix == "v210" {
        10
    } else {
        8
    }
}

/// Lossless audio codecs (PCM, FLAC, ALAC, WavPack…).
pub fn is_lossless_audio(codec: &str) -> bool {
    codec.starts_with("pcm_") || matches!(codec, "flac" | "alac" | "wavpack" | "ape" | "tta" | "truehd" | "mlp")
}

fn sample_fmt_bits(f: &str) -> u32 {
    match f.trim_end_matches('p') {
        "u8" => 8,
        "s16" => 16,
        "s32" | "flt" => 32,
        "dbl" | "s64" => 64,
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_rotation_and_streams() {
        let j = r#"{"streams":[{"index":0,"codec_name":"h264","codec_type":"video","width":320,"height":240,
            "r_frame_rate":"30/1","avg_frame_rate":"30/1","pix_fmt":"yuv420p","disposition":{"attached_pic":0},
            "side_data_list":[{"side_data_type":"Display Matrix","rotation":-90}]},
            {"index":1,"codec_name":"aac","codec_type":"audio","channels":2,"sample_rate":"48000","sample_fmt":"fltp"},
            {"index":2,"codec_name":"flac","codec_type":"audio","channels":6,"sample_rate":"48000","sample_fmt":"s32","bits_per_raw_sample":"24"},
            {"index":3,"codec_name":"subrip","codec_type":"subtitle"}],
            "format":{"format_name":"matroska,webm","duration":"6.000","bit_rate":"1000"}}"#;
        let p = parse(j).unwrap();
        assert_eq!(p.rotation, 270);
        assert_eq!(p.display_size(), (240, 320));
        assert_eq!(p.audio_tracks, 2);
        assert_eq!(p.subtitle_tracks, 1);
        assert_eq!(p.audio_bits, 0);
        assert_eq!(p.streams[2].bits, 24);
        assert!((p.duration - 6.0).abs() < 1e-9);
        assert!(!p.is_mp4_family());
    }
}
