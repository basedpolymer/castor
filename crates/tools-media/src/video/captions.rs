//! `video.captions`: captions from speech (whisper) or an SRT/VTT/ASS file, burnt in with a
//! style (classic, box, karaoke word highlight, minimal) or added as a soft subtitle track.

use castor_core::download::Asset;
use castor_core::options::{Choice, Cond, OptionSpec, Values};
use castor_core::{Family, InputSpec, Job, Output, Requirement, Result, RunCx, Tool, ToolMeta, ToolStatus, tr, tr_args};
use engine_media::subs::{self, Cue, Layout};
use engine_media::{Ffmpeg, Probe};

use crate::audio::transcribe::{prepare, whisper_downloads, whisper_options, whisper_unavailable};
use crate::common::*;

pub struct Captions {
    meta: ToolMeta,
}

impl Captions {
    pub fn new() -> Self {
        Captions {
            meta: ToolMeta {
                id: "video.captions",
                family: Family::Video,
                inputs: InputSpec::kinds(VIDEO),
                output: "MP4 · MKV · SRT",
                keywords: &["subtitles", "subtitle", "srt", "burn", "karaoke", "reels", "tiktok", "accessibility", "whisper"],
                status: ToolStatus::Ready,
                requires: &[Requirement::Ffmpeg, Requirement::Whisper],
                suffix: "captioned",
                in_place: false,
                serial: true,
            },
        }
    }
}

/// "#rrggbb[aa]" → ASS "&HAABBGGRR" (ASS alpha: 00 = opaque).
pub fn ass_color(c: [u8; 4], extra_alpha: u8) -> String {
    let a = 255 - ((c[3] as u16 * (255 - extra_alpha as u16)) / 255) as u8;
    format!("&H{a:02X}{:02X}{:02X}{:02X}", c[2], c[1], c[0])
}

fn ass_time(t: f64) -> String {
    let cs = (t.max(0.0) * 100.0).round() as u64;
    format!("{}:{:02}:{:02}.{:02}", cs / 360_000, (cs / 6000) % 60, (cs / 100) % 60, cs % 100)
}

fn ass_text(s: &str) -> String {
    s.replace('\\', "").replace('{', "(").replace('}', ")").replace('\n', "\\N")
}

/// Settings of the burnt-in look.
pub struct Look {
    pub style: String,
    pub family: String,
    pub size_pct: f64,
    pub position: String,
    pub color: [u8; 4],
    pub accent: [u8; 4],
}

/// Build an ASS script for `cues` on a `w × h` picture.
pub fn build_ass(cues: &[Cue], w: u32, h: u32, look: &Look) -> String {
    let mut fs = (h.min(w) as f64 * look.size_pct / 100.0).max(10.0);
    // Portrait video: size from the width so lines still fit.
    if h > w {
        fs = (w as f64 * look.size_pct / 100.0 * 1.15).max(10.0);
    }
    let align = match look.position.as_str() {
        "top" => 8,
        "middle" => 5,
        _ => 2,
    };
    let margin_v = (h as f64 * 0.07).round();
    let margin_h = (w as f64 * 0.06).round();
    let text = ass_color(look.color, 0);
    let (bold, border_style, outline, shadow, outline_c, back_c, fs) = match look.style.as_str() {
        "box" => (-1, 3, (fs * 0.22).round(), 0.0, "&H40000000".to_string(), "&H40000000".to_string(), fs),
        "minimal" => (0, 1, 0.0, (fs * 0.05).max(1.0), "&H00000000".to_string(), "&H80000000".to_string(), fs * 0.85),
        "karaoke" => (-1, 1, (fs * 0.08).max(1.5), (fs * 0.03).max(1.0), "&H00000000".to_string(), "&H80000000".to_string(), fs * 1.05),
        _ => (-1, 1, (fs * 0.07).max(1.5), (fs * 0.025).max(1.0), "&H00000000".to_string(), "&H90000000".to_string(), fs),
    };
    let mut s = format!(
        "[Script Info]\nScriptType: v4.00+\nTitle: Castor captions\nPlayResX: {w}\nPlayResY: {h}\nWrapStyle: 0\nScaledBorderAndShadow: yes\nYCbCr Matrix: TV.709\n\n\
[V4+ Styles]\nFormat: Name, Fontname, Fontsize, PrimaryColour, SecondaryColour, OutlineColour, BackColour, Bold, Italic, Underline, StrikeOut, ScaleX, ScaleY, Spacing, Angle, BorderStyle, Outline, Shadow, Alignment, MarginL, MarginR, MarginV, Encoding\n\
Style: Default,{fam},{fs:.0},{text},{text},{outline_c},{back_c},{bold},0,0,0,100,100,0,0,{border_style},{outline:.1},{shadow:.1},{align},{margin_h},{margin_h},{margin_v},1\n\n\
[Events]\nFormat: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\n",
        fam = look.family
    );
    let accent = ass_color(look.accent, 0);
    for c in cues {
        if look.style == "karaoke" && !c.words.is_empty() {
            // One event per word: the spoken word in the accent colour, line breaks kept.
            let lines: Vec<Vec<&str>> = c.text.lines().map(|l| l.split_whitespace().collect()).collect();
            let total: usize = lines.iter().map(|l| l.len()).sum();
            if total != c.words.len() {
                s.push_str(&format!("Dialogue: 0,{},{},Default,,0,0,0,,{}\n", ass_time(c.start), ass_time(c.end), ass_text(&c.text)));
                continue;
            }
            if c.words[0].start > c.start + 0.01 {
                s.push_str(&format!("Dialogue: 0,{},{},Default,,0,0,0,,{}\n", ass_time(c.start), ass_time(c.words[0].start), ass_text(&c.text)));
            }
            for (i, wd) in c.words.iter().enumerate() {
                let end = c.words.get(i + 1).map(|n| n.start).unwrap_or(c.end).max(wd.start + 0.01);
                let mut k = 0;
                let mut body = Vec::new();
                for l in &lines {
                    let mut lw = Vec::new();
                    for word in l {
                        let t = ass_text(word);
                        lw.push(if k == i { format!("{{\\c{accent}}}{t}{{\\c{text}}}") } else { t });
                        k += 1;
                    }
                    body.push(lw.join(" "));
                }
                s.push_str(&format!("Dialogue: 0,{},{},Default,,0,0,0,,{}\n", ass_time(wd.start), ass_time(end), body.join("\\N")));
            }
        } else {
            s.push_str(&format!("Dialogue: 0,{},{},Default,,0,0,0,,{}\n", ass_time(c.start), ass_time(c.end), ass_text(&c.text)));
        }
    }
    s
}

impl Tool for Captions {
    fn meta(&self) -> &ToolMeta {
        &self.meta
    }
    fn options(&self) -> Vec<OptionSpec> {
        let burn = || Cond::Eq("mode", "burn");
        let mut o = vec![
            OptionSpec::choice("source", vec![Choice::key("speech"), Choice::key("file")], "speech"),
            OptionSpec::file("subtitles", &["srt", "vtt", "ass", "ssa"]).when(Cond::Eq("source", "file")),
            OptionSpec::choice("mode", vec![Choice::key("burn"), Choice::key("soft")], "burn"),
            OptionSpec::choice("style", vec![Choice::key("classic"), Choice::key("box"), Choice::key("karaoke"), Choice::key("minimal")], "classic")
                .when(burn()),
            OptionSpec::font("font", crate::fonts::DEFAULT_FAMILY).when(burn()),
            OptionSpec::int("size", 2, 15, 6).slider().unit("%").when(burn()),
            OptionSpec::choice("position", vec![Choice::key("bottom"), Choice::key("middle"), Choice::key("top")], "bottom").when(burn()),
            OptionSpec::color("color", "#ffffff").when(burn()).common(),
            OptionSpec::color("accent", "#ffd400").when(Cond::Eq("style", "karaoke")),
            OptionSpec::toggle("export_srt", true),
        ];
        o.extend(whisper_options(true).into_iter().map(|s| if s.visible_if.is_none() { s.when(Cond::Eq("source", "speech")) } else { s }));
        o.push(quality_option("best").advanced());
        o
    }
    fn downloads(&self, v: &Values) -> Vec<Asset> {
        if v.str("source") == "file" { Vec::new() } else { whisper_downloads(v) }
    }
    fn unavailable(&self) -> Option<String> {
        need_ffmpeg()
    }
    fn run(&self, job: &Job, v: &Values, cx: &RunCx) -> Result<Vec<Output>> {
        let input = job.input()?;
        let p = probe(input)?;
        need_video(&p)?;
        let karaoke = v.str("mode") == "burn" && v.str("style") == "karaoke";
        let (cues, language, base) = if v.str("source") == "file" {
            let f = v.path("subtitles").ok_or_else(|| anyhow::anyhow!(tr("media.err.no_subtitles")))?;
            let raw = std::fs::read(f).map_err(|_| anyhow::anyhow!(tr("media.err.no_subtitles")))?;
            let cues = subs::parse(&String::from_utf8_lossy(&raw));
            anyhow::ensure!(!cues.is_empty(), tr("media.err.empty_subtitles"));
            (cues, None, 0.0)
        } else {
            if let Some(e) = whisper_unavailable() {
                anyhow::bail!(e);
            }
            anyhow::ensure!(p.has_audio, tr("media.err.no_audio"));
            let opts = prepare(v, cx, 0.05)?;
            let t = engine_media::whisper::transcribe(&input.path, &opts, cx, (0.05, 0.6))?;
            anyhow::ensure!(!t.segments.is_empty(), tr("media.err.no_speech"));
            let layout = if karaoke { Layout { max_chars: 28, max_lines: 2, max_secs: 4.0, gap: 0.6 } } else { Layout::default() };
            (subs::cues_from_transcript(&t, layout), Some(t.language.clone()), 0.6)
        };
        let mut outs = Vec::new();
        if v.bool("export_srt") {
            let srt = cx.output_path(Some(input), "srt");
            std::fs::write(&srt, subs::to_srt(&cues))?;
            outs.push(Output::new(srt).with_note(tr_args("media.note.cues", &[("count", &cues.len().to_string())])));
        }
        let video = if v.str("mode") == "soft" { soft(input, &p, &cues, language.as_deref(), cx, base)? } else { burn(input, &p, &cues, v, cx, base)? };
        outs.insert(0, video);
        Ok(outs)
    }
}

fn soft(input: &castor_core::FileInfo, p: &Probe, cues: &[Cue], lang: Option<&str>, cx: &RunCx, base: f32) -> Result<Output> {
    let ext = match ext_of(input).as_str() {
        e @ ("mp4" | "mov" | "m4v" | "mkv" | "webm") => e.to_string(),
        _ => "mkv".to_string(),
    };
    let srt = cx.tmp_dir().join("captions.srt");
    std::fs::write(&srt, subs::to_srt(cues))?;
    let codec = match ext.as_str() {
        "mkv" => "srt",
        "webm" => "webvtt",
        _ => "mov_text",
    };
    let n_subs = p.subtitle_tracks;
    let out = Partial::new(cx.output_path(Some(input), &ext));
    let mut ff = Ffmpeg::new()
        .input(&input.path)
        .input(&srt)
        .args(["-map", "0:v?", "-map", "0:a?", "-map", "1:0", "-c", "copy", &format!("-c:s:{n_subs}"), codec]);
    if let Some(l) = lang.and_then(iso639_2) {
        ff = ff.args([&format!("-metadata:s:s:{n_subs}"), &format!("language={l}")]);
    }
    ff.args(["-map_metadata", "0"]).args(mux_args(&ext)).arg(out.path()).duration(p.duration).range(base, 1.0).run(cx)?;
    Ok(Output::new(out.keep()).with_note(tr("media.note.soft_subs")))
}

fn burn(input: &castor_core::FileInfo, p: &Probe, cues: &[Cue], v: &Values, cx: &RunCx, base: f32) -> Result<Output> {
    anyhow::ensure!(engine_media::has_filter("subtitles"), tr("media.err.no_libass"));
    let tmp = std::path::absolute(cx.tmp_dir())?;
    let style = v.str("style").to_string();
    let bold = style != "minimal";
    let font = crate::fonts::prepare(v.str("font"), bold, &tmp)?;
    let (w, h) = p.display_size();
    let look = Look {
        style,
        family: font.family.clone(),
        size_pct: v.int("size").clamp(2, 15) as f64,
        position: v.str("position").to_string(),
        color: v.color("color"),
        accent: v.color("accent"),
    };
    std::fs::write(tmp.join("captions.ass"), build_ass(cues, w, h, &look))?;
    let ext = ext_of(input);
    let enc = VideoEnc::quality(edit_encoder(p, &ext).codec, level(v));
    let ext = output_ext(&ext, enc.codec);
    let mut vf = vec!["subtitles=captions.ass:fontsdir=.".to_string()];
    finish_filters(p, &enc, &mut vf);
    let (va, _) = video_args(p, &enc);
    let out = Partial::new(cx.output_path(Some(input), &ext));
    let dst = std::path::absolute(out.path())?;
    Ffmpeg::new()
        .input(std::path::absolute(&input.path)?)
        .args(["-map", &format!("0:{}", p.video_index.unwrap_or(0)), "-map", "0:a?"])
        .args(["-vf", &vf.join(",")])
        .args(va)
        .args(edit_audio(p, &ext, false))
        .args(["-map_metadata", "0"])
        .args(mux_args(&ext))
        .arg(&dst)
        .cwd(&tmp)
        .duration(p.duration)
        .range(base, 1.0)
        .run(cx)?;
    Ok(Output::new(out.keep()).with_note(tr_args("media.note.cues", &[("count", &cues.len().to_string())])))
}

/// ISO 639-1 → 639-2/B for container language tags.
fn iso639_2(code: &str) -> Option<&'static str> {
    Some(match code {
        "en" => "eng",
        "fr" => "fre",
        "de" => "ger",
        "es" => "spa",
        "it" => "ita",
        "pt" => "por",
        "nl" => "dut",
        "ja" => "jpn",
        "zh" => "chi",
        "ko" => "kor",
        "ru" => "rus",
        "uk" => "ukr",
        "pl" => "pol",
        "tr" => "tur",
        "ar" => "ara",
        "hi" => "hin",
        "sv" => "swe",
        "da" => "dan",
        "no" => "nor",
        "fi" => "fin",
        "cs" => "cze",
        "el" => "gre",
        "he" => "heb",
        "ro" => "rum",
        "hu" => "hun",
        "ca" => "cat",
        "vi" => "vie",
        "id" => "ind",
        "th" => "tha",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_media::whisper::Word;
    #[test]
    fn colors() {
        assert_eq!(ass_color([255, 212, 0, 255], 0), "&H0000D4FF");
        assert_eq!(ass_time(3723.456), "1:02:03.46");
    }
    #[test]
    fn karaoke_events() {
        let w = |s: f64, e: f64, t: &str| Word { start: s, end: e, text: t.into(), p: 1.0 };
        let c = Cue { start: 1.0, end: 3.0, text: "Hello\nworld".into(), words: vec![w(1.0, 1.5, "Hello"), w(2.0, 2.8, "world")] };
        let look = Look { style: "karaoke".into(), family: "LM Sans 10".into(), size_pct: 6.0, position: "bottom".into(), color: [255, 255, 255, 255], accent: [255, 212, 0, 255] };
        let a = build_ass(&[c], 1920, 1080, &look);
        let ev: Vec<&str> = a.lines().filter(|l| l.starts_with("Dialogue")).collect();
        assert_eq!(ev.len(), 2);
        assert!(ev[0].contains("{\\c&H0000D4FF}Hello{\\c&H00FFFFFF}\\Nworld"));
        assert!(ev[1].starts_with("Dialogue: 0,0:00:02.00,0:00:03.00"));
    }
}
