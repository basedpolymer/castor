//! Shared helpers for the media tools: metadata, output guards, encoder settings, stream maps.

use std::path::{Path, PathBuf};

use castor_core::options::{Choice, OptionSpec, Values};
use castor_core::{Family, FileInfo, FileKind, InputSpec, Requirement, ToolMeta, ToolStatus, tr, tr_args, units};
use engine_media::hw::{self, Codec};
use engine_media::{Ffmpeg, Probe};

pub const VIDEO: &[FileKind] = &[FileKind::Video];
pub const AUDIO: &[FileKind] = &[FileKind::Audio];
pub const AV: &[FileKind] = &[FileKind::Video, FileKind::Audio];
pub const FFMPEG: &[Requirement] = &[Requirement::Ffmpeg];

/// Standard metadata for a media tool.
pub fn meta(
    id: &'static str,
    family: Family,
    inputs: InputSpec,
    output: &'static str,
    keywords: &'static [&'static str],
    suffix: &'static str,
    serial: bool,
) -> ToolMeta {
    ToolMeta { id, family, inputs, output, keywords, status: ToolStatus::Ready, requires: FFMPEG, suffix, in_place: false, serial }
}

/// `Tool::unavailable` for every ffmpeg tool.
pub fn need_ffmpeg() -> Option<String> {
    match engine_media::ffmpeg() {
        Ok(_) => None,
        Err(e) => Some(e.to_string()),
    }
}

/// An output file being written: removed on drop unless [`Partial::keep`] is called, so a
/// failed or cancelled run never leaves a half-written file behind.
pub struct Partial {
    path: PathBuf,
    keep: bool,
}

impl Partial {
    pub fn new(path: PathBuf) -> Partial {
        Partial { path, keep: false }
    }
    pub fn path(&self) -> &Path {
        &self.path
    }
    pub fn keep(mut self) -> PathBuf {
        self.keep = true;
        std::mem::take(&mut self.path)
    }
}

impl Drop for Partial {
    fn drop(&mut self) {
        if !self.keep && !self.path.as_os_str().is_empty() {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

/// Probe an input (user-facing errors).
pub fn probe(f: &FileInfo) -> anyhow::Result<Probe> {
    engine_media::probe(&f.path)
}

pub fn need_video(p: &Probe) -> anyhow::Result<()> {
    anyhow::ensure!(p.has_video, tr("media.err.no_video"));
    Ok(())
}

pub fn need_audio(p: &Probe) -> anyhow::Result<()> {
    anyhow::ensure!(p.has_audio, tr("media.err.no_audio"));
    Ok(())
}

pub fn size_of(p: &Path) -> u64 {
    std::fs::metadata(p).map(|m| m.len()).unwrap_or(0)
}

/// "19 MB → 4.2 MB (−78 %)".
pub fn size_note(before: u64, after: u64) -> String {
    format!("{} → {} ({})", units::bytes(before), units::bytes(after), units::change(before, after))
}

/// A time option (Text) in seconds; empty = None. Errors name the option.
pub fn time_opt(v: &Values, key: &str) -> anyhow::Result<Option<f64>> {
    engine_media::time::parse_time(v.str(key))
}

/// Extension for a re-encoded output: containers we write well are kept, others become MP4
/// (video) or M4A (audio): WMV, FLV, MTS, 3GP, WMA, AMR, CAF…
pub fn reencode_ext(ext: &str, video: bool) -> String {
    let keep = if video {
        matches!(ext, "mp4" | "m4v" | "mov" | "mkv" | "webm" | "avi" | "ts" | "m2ts")
    } else {
        matches!(ext, "mp3" | "wav" | "flac" | "ogg" | "oga" | "opus" | "m4a" | "aiff" | "aif" | "aac" | "mka")
    };
    if keep {
        ext.to_string()
    } else if video {
        "mp4".into()
    } else {
        "m4a".into()
    }
}

pub fn ext_of(f: &FileInfo) -> String {
    let e = f.extension();
    if e.is_empty() { "mp4".into() } else { e }
}

// ---------------------------------------------------------------------------------------------
// Video encoding

/// Video codec chosen by a tool.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VCodec {
    H264,
    Hevc,
    Av1,
    Vp9,
    ProRes,
}

impl VCodec {
    pub fn parse(s: &str) -> VCodec {
        match s {
            "h265" | "hevc" => VCodec::Hevc,
            "av1" => VCodec::Av1,
            "vp9" => VCodec::Vp9,
            "prores" => VCodec::ProRes,
            _ => VCodec::H264,
        }
    }
    /// Codec of a probed stream, if we can encode it again ("keep codec").
    pub fn from_probe(name: &str) -> Option<VCodec> {
        Some(match name {
            "h264" => VCodec::H264,
            "hevc" => VCodec::Hevc,
            "av1" => VCodec::Av1,
            "vp9" => VCodec::Vp9,
            "prores" => VCodec::ProRes,
            _ => return None,
        })
    }
    pub fn ffprobe_name(self) -> &'static str {
        match self {
            VCodec::H264 => "h264",
            VCodec::Hevc => "hevc",
            VCodec::Av1 => "av1",
            VCodec::Vp9 => "vp9",
            VCodec::ProRes => "prores",
        }
    }
    fn hw(self) -> Option<Codec> {
        match self {
            VCodec::H264 => Some(Codec::H264),
            VCodec::Hevc => Some(Codec::Hevc),
            VCodec::Av1 => Some(Codec::Av1),
            _ => None,
        }
    }
}

/// Quality preset, 0 = best … 2 = smallest.
pub fn level_of(s: &str) -> usize {
    match s {
        "best" | "quality" => 0,
        "small" => 2,
        _ => 1,
    }
}

/// Everything needed to encode the video stream.
#[derive(Clone, Debug)]
pub struct VideoEnc {
    pub codec: VCodec,
    pub level: usize,
    pub hardware: bool,
    /// Average bitrate (two-pass target size). None = constant quality.
    pub bitrate: Option<u64>,
    /// Pass number for two-pass (1 or 2) and the log prefix.
    pub pass: Option<(u8, PathBuf)>,
}

impl VideoEnc {
    pub fn quality(codec: VCodec, level: usize) -> VideoEnc {
        VideoEnc { codec, level, hardware: false, bitrate: None, pass: None }
    }
}

/// Container from extension.
pub fn container_ok(ext: &str, v: VCodec) -> bool {
    match ext {
        "webm" => matches!(v, VCodec::Vp9 | VCodec::Av1),
        "avi" => matches!(v, VCodec::H264),
        "mp4" | "m4v" => matches!(v, VCodec::H264 | VCodec::Hevc | VCodec::Av1 | VCodec::Vp9),
        "mov" => matches!(v, VCodec::H264 | VCodec::Hevc | VCodec::Av1 | VCodec::ProRes),
        _ => true,
    }
}

/// Output extension for a re-encode of `input_ext` with `codec`: keep the input container when
/// it can hold the codec, else MP4 (or MOV for ProRes, WebM never chosen implicitly).
pub fn output_ext(input_ext: &str, codec: VCodec) -> String {
    let e = match input_ext {
        "mp4" | "m4v" | "mov" | "mkv" | "webm" | "avi" => input_ext,
        _ => "mp4",
    };
    if container_ok(e, codec) {
        e.to_string()
    } else if codec == VCodec::ProRes {
        "mov".into()
    } else if matches!(codec, VCodec::Vp9) {
        "webm".into()
    } else {
        "mp4".into()
    }
}

/// Filters that make the picture encodable by a 4:2:0 encoder: tone-map HDR to SDR when the
/// target is 8-bit, and force even dimensions.
pub fn finish_filters(p: &Probe, enc: &VideoEnc, vf: &mut Vec<String>) {
    let keep_hdr = keeps_hdr(p, enc);
    if p.is_hdr() && !keep_hdr {
        vf.push(
            "zscale=t=linear:npl=100,format=gbrpf32le,zscale=p=bt709,tonemap=tonemap=hable:desat=0,zscale=t=bt709:m=bt709:r=tv"
                .into(),
        );
    }
    if enc.codec != VCodec::ProRes {
        vf.push("scale=trunc(iw/2)*2:trunc(ih/2)*2".into());
    }
    vf.push(format!("format={}", pix_fmt(p, enc)));
}

fn keeps_hdr(p: &Probe, enc: &VideoEnc) -> bool {
    p.is_hdr() && matches!(enc.codec, VCodec::Hevc | VCodec::Av1 | VCodec::Vp9 | VCodec::ProRes) && !enc.hardware
}

fn pix_fmt(p: &Probe, enc: &VideoEnc) -> &'static str {
    match enc.codec {
        VCodec::ProRes => "yuv422p10le",
        VCodec::Av1 => "yuv420p10le",
        VCodec::Hevc | VCodec::Vp9 if keeps_hdr(p, enc) || (p.bit_depth > 8 && !enc.hardware) => "yuv420p10le",
        _ => "yuv420p",
    }
}

/// Encoder arguments (`-c:v …`) for `enc`, given the probed source.
/// Returns the arguments and the name of the encoder used.
pub fn video_args(p: &Probe, enc: &VideoEnc) -> (Vec<String>, String) {
    let l = enc.level.min(2);
    let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
    if enc.hardware {
        if let Some(h) = enc.codec.hw().and_then(hw::encoder) {
            let fallback = enc.bitrate.unwrap_or_else(|| guess_bitrate(p, l));
            let mut a = s(&["-c:v", h]);
            if let Some(b) = enc.bitrate {
                a.extend(s(&["-b:v", &b.to_string(), "-maxrate", &(b * 3 / 2).to_string(), "-bufsize", &(b * 2).to_string()]));
            } else {
                a.extend(hw::quality_args(h, l, fallback));
            }
            if enc.codec == VCodec::Hevc {
                a.extend(s(&["-tag:v", "hvc1"]));
            }
            return (a, h.to_string());
        }
    }
    let mut a: Vec<String>;
    let name;
    match enc.codec {
        VCodec::H264 => {
            name = "libx264";
            a = s(&["-c:v", name, "-preset", ["slow", "medium", "medium"][l], "-profile:v", "high"]);
            match enc.bitrate {
                Some(b) => a.extend(s(&["-b:v", &b.to_string()])),
                None => a.extend(s(&["-crf", ["20", "23", "28"][l]])),
            }
        }
        VCodec::Hevc => {
            name = "libx265";
            a = s(&["-c:v", name, "-preset", ["slow", "medium", "medium"][l], "-tag:v", "hvc1"]);
            match enc.bitrate {
                Some(b) => a.extend(s(&["-b:v", &b.to_string()])),
                None => a.extend(s(&["-crf", ["22", "26", "30"][l]])),
            }
            let mut params = vec!["log-level=error".to_string()];
            if keeps_hdr(p, enc) {
                params.push(format!(
                    "hdr10-opt=1:repeat-headers=1:colorprim={}:transfer={}:colormatrix={}",
                    or(&p.color_primaries, "bt2020"),
                    or(&p.color_transfer, "smpte2084"),
                    or(&p.color_space, "bt2020nc")
                ));
            }
            if let Some((n, log)) = &enc.pass {
                params.push(format!("pass={n}:stats={}", log.with_extension("x265.log").display()));
            }
            a.extend(s(&["-x265-params", &params.join(":")]));
        }
        VCodec::Av1 => {
            name = "libsvtav1";
            a = s(&["-c:v", name, "-preset", ["5", "7", "8"][l]]);
            match enc.bitrate {
                Some(b) => a.extend(s(&["-b:v", &b.to_string()])),
                None => a.extend(s(&["-crf", ["28", "35", "42"][l]])),
            }
            a.extend(s(&["-svtav1-params", "tune=0"]));
        }
        VCodec::Vp9 => {
            name = "libvpx-vp9";
            a = s(&["-c:v", name, "-row-mt", "1", "-deadline", "good", "-cpu-used", ["2", "3", "4"][l]]);
            match enc.bitrate {
                Some(b) => a.extend(s(&["-b:v", &b.to_string()])),
                None => a.extend(s(&["-crf", ["30", "34", "38"][l], "-b:v", "0"])),
            }
        }
        VCodec::ProRes => {
            name = "prores_ks";
            // 422 HQ / 422 / 422 LT
            a = s(&["-c:v", name, "-profile:v", ["3", "2", "1"][l], "-vendor", "apl0"]);
        }
    }
    if let Some((n, log)) = &enc.pass {
        if enc.codec != VCodec::Hevc {
            a.extend(s(&["-pass", &n.to_string(), "-passlogfile"]));
            a.push(log.display().to_string());
        }
    }
    if keeps_hdr(p, enc) {
        a.extend(s(&[
            "-color_primaries",
            or(&p.color_primaries, "bt2020"),
            "-color_trc",
            or(&p.color_transfer, "smpte2084"),
            "-colorspace",
            or(&p.color_space, "bt2020nc"),
        ]));
    } else if p.is_hdr() {
        a.extend(s(&["-color_primaries", "bt709", "-color_trc", "bt709", "-colorspace", "bt709"]));
    }
    (a, name.to_string())
}

fn or<'a>(s: &'a str, d: &'a str) -> &'a str {
    if s.is_empty() || s == "unknown" { d } else { s }
}

/// A sensible average bitrate for encoders without constant quality (bits/s).
pub fn guess_bitrate(p: &Probe, level: usize) -> u64 {
    let (w, h) = p.display_size();
    let px = (w.max(16) as f64) * (h.max(16) as f64) * p.fps.clamp(10.0, 60.0);
    let bpp = [0.10, 0.07, 0.045][level.min(2)];
    (px * bpp) as u64
}

// ---------------------------------------------------------------------------------------------
// Audio encoding

/// Audio encoder arguments for a container. `kbps` per stereo track.
pub fn audio_args(ext: &str, kbps: u32, p: &Probe) -> Vec<String> {
    let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
    match ext {
        "webm" => {
            let mut a = s(&["-c:a", "libopus", "-b:a", &format!("{kbps}k")]);
            if p.channels > 2 {
                a.extend(s(&["-mapping_family", "1"]));
            }
            a
        }
        "avi" => s(&["-c:a", "libmp3lame", "-b:a", &format!("{}k", kbps.max(128))]),
        _ => {
            let k = if p.channels > 2 { kbps * p.channels / 2 } else { kbps };
            s(&["-c:a", "aac", "-b:a", &format!("{k}k")])
        }
    }
}

/// Can this audio codec go into this container untouched?
pub fn audio_copy_ok(ext: &str, codec: &str) -> bool {
    match ext {
        "mp4" | "m4v" | "mov" => matches!(codec, "aac" | "mp3" | "alac" | "ac3" | "eac3" | "opus" | "flac"),
        "webm" => matches!(codec, "opus" | "vorbis"),
        "avi" => matches!(codec, "mp3" | "ac3" | "pcm_s16le"),
        "mkv" => true,
        _ => false,
    }
}

/// `-map` arguments: main video, every audio track, subtitles the container can hold
/// (converted to the container's text format), chapters and metadata.
pub fn stream_map(p: &Probe, ext: &str, video: bool, audio: bool) -> Vec<String> {
    let mut a = Vec::new();
    if video {
        if let Some(i) = p.video_index {
            a.push("-map".into());
            a.push(format!("0:{i}"));
        }
    }
    if audio && p.has_audio {
        a.push("-map".into());
        a.push("0:a".into());
    }
    let subs: Vec<_> = p.subtitle_streams().collect();
    match ext {
        "mkv" if !subs.is_empty() => {
            a.extend(["-map".into(), "0:s".into(), "-c:s".into(), "copy".into()]);
        }
        "mp4" | "m4v" | "mov" if subs.iter().any(|s| s.is_text_subtitle()) => {
            for s in subs.iter().filter(|s| s.is_text_subtitle()) {
                a.push("-map".into());
                a.push(format!("0:{}", s.index));
            }
            a.extend(["-c:s".into(), "mov_text".into()]);
        }
        "webm" if subs.iter().any(|s| s.is_text_subtitle()) => {
            for s in subs.iter().filter(|s| s.is_text_subtitle()) {
                a.push("-map".into());
                a.push(format!("0:{}", s.index));
            }
            a.extend(["-c:s".into(), "webvtt".into()]);
        }
        _ => {}
    }
    a.extend(["-map_metadata".into(), "0".into(), "-map_chapters".into(), "0".into()]);
    a
}

/// Muxer flags for the output container.
pub fn mux_args(ext: &str) -> Vec<String> {
    match ext {
        "mp4" | "m4v" | "mov" | "m4a" => vec!["-movflags".into(), "+faststart".into()],
        _ => vec![],
    }
}

/// Run an encode; if a hardware encoder fails, retry once in software.
pub fn encode_with_fallback(
    cx: &castor_core::RunCx,
    enc: &VideoEnc,
    build: impl Fn(&VideoEnc) -> Ffmpeg,
) -> anyhow::Result<()> {
    match build(enc).run(cx) {
        Ok(_) => Ok(()),
        Err(e) if enc.hardware && !e.is::<castor_core::job::Cancelled>() => {
            tracing::warn!("hardware encode failed, retrying in software: {e:#}");
            cx.message(tr("media.msg.hw_fallback"));
            let mut soft = enc.clone();
            soft.hardware = false;
            build(&soft).run(cx).map(|_| ())
        }
        Err(e) => Err(e),
    }
}

// ---------------------------------------------------------------------------------------------
// Shared options

pub fn quality_option(default: &'static str) -> OptionSpec {
    OptionSpec::choice("quality", vec![Choice::key("best"), Choice::key("balanced"), Choice::key("small")], default)
}

pub fn codec_option(default: &'static str) -> OptionSpec {
    OptionSpec::choice(
        "codec",
        vec![Choice::lit("h264", "H.264"), Choice::lit("h265", "H.265 (HEVC)"), Choice::lit("av1", "AV1")],
        default,
    )
}

pub fn hardware_option() -> OptionSpec {
    OptionSpec::toggle("hardware", false).advanced()
}

/// Trim-like "fast (keyframes) / exact (re-encode)" option.
pub fn cut_option() -> OptionSpec {
    OptionSpec::choice("cut", vec![Choice::key("exact"), Choice::key("fast")], "exact")
}

/// Note shown when the tool kept the original because the result was not smaller.
pub fn kept_original_note(size: u64) -> String {
    tr_args("media.note.kept_original", &[("size", &units::bytes(size))])
}

/// Re-encode settings that keep quality high for edits (trim exact, crop, rotate…): same codec
/// family as the source when we can encode it, at "best" quality.
pub fn edit_encoder(p: &Probe, ext: &str) -> VideoEnc {
    let src = p.video_codec.as_deref().and_then(VCodec::from_probe).unwrap_or(VCodec::H264);
    let codec = if container_ok(ext, src) && src != VCodec::ProRes { src } else if ext == "webm" { VCodec::Vp9 } else { VCodec::H264 };
    let codec = if codec == VCodec::ProRes { VCodec::H264 } else { codec };
    VideoEnc::quality(codec, 0)
}

/// Audio settings for edits: copy if the container takes it, else a good AAC/Opus encode.
pub fn edit_audio(p: &Probe, ext: &str, filtered: bool) -> Vec<String> {
    let codec = p.audio_codec.as_deref().unwrap_or("");
    if !filtered && audio_copy_ok(ext, codec) {
        vec!["-c:a".into(), "copy".into()]
    } else {
        audio_args(ext, 192, p)
    }
}

/// Values helper: a "level" from the `quality` option.
pub fn level(v: &Values) -> usize {
    level_of(v.str("quality"))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn containers() {
        assert_eq!(output_ext("mkv", VCodec::H264), "mkv");
        assert_eq!(output_ext("webm", VCodec::H264), "mp4");
        assert_eq!(output_ext("avi", VCodec::Hevc), "mp4");
        assert_eq!(output_ext("mp4", VCodec::ProRes), "mov");
        assert_eq!(output_ext("flv", VCodec::H264), "mp4");
        assert!(audio_copy_ok("mp4", "aac") && !audio_copy_ok("webm", "aac"));
    }
}
