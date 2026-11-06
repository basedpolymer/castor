//! `create.audiogram`: audio → video for social media: an animated waveform (bars, line or
//! circle), a title, a cover image, captions (from speech with whisper, or from an SRT/VTT
//! file) and a progress line, in 1:1, 9:16 or 16:9. Frames are drawn here (tiny-skia) and
//! encoded by ffmpeg with the original sound.

use std::path::{Path, PathBuf};

use anyhow::{Result, anyhow};
use castor_core::download::Asset;
use castor_core::options::{Choice, Cond, OptionSpec, Values};
use castor_core::{FileInfo, FileKind, InputSpec, Job, Output, Preview, Requirement, RunCx, Tool, ToolMeta, tr};
use engine_media::{subs, whisper};
use resvg::tiny_skia::{self, FillRule, Paint, PathBuilder, Pixmap, PixmapPaint, Stroke, Transform};

use crate::common::{self, xml};
use crate::render::{png, raster, style, video};

pub struct AudiogramTool {
    meta: ToolMeta,
}

impl AudiogramTool {
    pub fn new() -> Self {
        AudiogramTool {
            meta: common::serial(common::with_requires(
                common::meta(
                    "create.audiogram",
                    InputSpec::kinds(&[FileKind::Audio, FileKind::Video]),
                    "MP4",
                    &["audiogram", "podcast", "waveform", "audio to video", "social", "captions", "clip", "visualizer"],
                    "audiogram",
                ),
                &[Requirement::Ffmpeg],
            )),
        }
    }
}

const ASPECTS: &[(&str, u32, u32)] = &[("square", 1080, 1080), ("portrait", 1080, 1920), ("landscape", 1920, 1080), ("feed", 1080, 1350)];

impl Tool for AudiogramTool {
    fn meta(&self) -> &ToolMeta {
        &self.meta
    }

    fn options(&self) -> Vec<OptionSpec> {
        vec![
            OptionSpec::choice("style", ["bars", "line", "circle"].map(Choice::key).to_vec(), "bars"),
            OptionSpec::choice("aspect", ASPECTS.iter().map(|a| Choice::key(a.0)).collect(), "square"),
            OptionSpec::text("title", ""),
            OptionSpec::text("subtitle", ""),
            OptionSpec::file("cover", &["png", "jpg", "jpeg", "webp", "svg"]),
            OptionSpec::choice("scene", raster::SCENES.iter().filter(|s| **s != "transparent").map(|s| Choice::key(s)).collect(), "ink"),
            OptionSpec::color("scene_color", "#1b1616").when(Cond::Eq("scene", "custom")),
            OptionSpec::color("wave_color", "#ffeca4"),
            OptionSpec::color("text_color", "#fcfbf9"),
            OptionSpec::choice("captions", ["none", "speech", "file"].map(Choice::key).to_vec(), "none"),
            OptionSpec::file("subtitles", &["srt", "vtt", "ass"]).when(Cond::Eq("captions", "file")),
            OptionSpec::choice("model", whisper::MODELS.iter().map(|m| Choice::key(m)).collect(), "base").when(Cond::Eq("captions", "speech")),
            OptionSpec::text("start", "0"),
            OptionSpec::text("length", "60"),
            OptionSpec::toggle("progress", true),
            OptionSpec::int("fps", 15, 60, 30).unit("fps").advanced(),
        ]
    }

    fn downloads(&self, v: &Values) -> Vec<Asset> {
        if v.str("captions") != "speech" {
            return Vec::new();
        }
        whisper::assets(v.str("model"), "q8", false, true).into_iter().filter(|a| !a.is_present()).collect()
    }

    fn unavailable(&self) -> Option<String> {
        if engine_media::ffmpeg().is_err() { Some(tr("create.audiogram.err.ffmpeg")) } else { None }
    }

    fn preview(&self, input: Option<&FileInfo>, v: &Values, max_side: u32) -> Result<Option<Preview>> {
        let (w, h) = aspect(v);
        let k = (max_side as f32 / w.max(h) as f32).min(1.0);
        let (pw, ph) = (((w as f32 * k) as u32) & !1, ((h as f32 * k) as u32) & !1);
        let title = default_title(v, input.map(|f| f.path.as_path()));
        let bg = background(v, pw, ph, &title)?;
        // A plausible fake signal so the style can be judged before running.
        let mut levels = vec![0f32; BANDS];
        for (i, l) in levels.iter_mut().enumerate() {
            *l = (0.35 + 0.6 * ((i as f32 * 0.7).sin() * 0.5 + 0.5) * (1.0 - i as f32 / BANDS as f32 * 0.6)).min(1.0);
        }
        let wave: Vec<f32> = (0..512).map(|i| (i as f32 * 0.09).sin() * 0.5 * (i as f32 * 0.013).sin()).collect();
        let mut pm = bg.clone();
        draw_frame(&mut pm, v, &levels, &wave, 0.35, None);
        Ok(Some(common::preview(pm.encode_png().map_err(|e| anyhow!("{e}"))?, common::figure(&common::px(w, h)))))
    }

    fn run(&self, job: &Job, v: &Values, cx: &RunCx) -> Result<Vec<Output>> {
        let input = job.input()?;
        let start = engine_media::time::parse_time(v.str("start"))?.unwrap_or(0.0).max(0.0);
        let probe = engine_media::probe(&input.path)?;
        anyhow::ensure!(probe.has_audio, "{}", tr("create.audiogram.err.no_audio"));
        let max_len = (probe.duration - start).max(0.0);
        anyhow::ensure!(max_len > 0.2, "{}", tr("create.audiogram.err.start"));
        let len = engine_media::time::parse_time(v.str("length"))?.unwrap_or(max_len).clamp(0.5, max_len.min(30.0 * 60.0));
        // 1. The clip's sound, as WAV (muxed in) and as mono samples (analysed).
        cx.message(tr("create.audiogram.msg.audio"));
        let wav = cx.tmp_dir().join("clip.wav");
        engine_media::Ffmpeg::new()
            .args(["-y", "-ss", &format!("{start:.3}"), "-t", &format!("{len:.3}")])
            .input(&input.path)
            .args(["-vn", "-ac", "2", "-ar", "48000", "-c:a", "pcm_s16le"])
            .arg(&wav)
            .duration(len)
            .range(0.0, 0.05)
            .run(cx)?;
        let samples = read_mono(&wav, RATE)?;
        // 2. Captions.
        let cues: Vec<subs::Cue> = match v.str("captions") {
            "speech" => {
                cx.message(tr("create.audiogram.msg.transcribing"));
                let assets = whisper::assets(v.str("model"), "q8", false, true);
                let mut paths = Vec::new();
                for a in &assets {
                    paths.push(castor_core::download::ensure(a, &cx.cancel_token(), &|_, _| {})?);
                }
                let opts = whisper::Options {
                    model: paths[0].clone(),
                    language: "auto".into(),
                    translate: false,
                    vad: paths.get(1).cloned(),
                    beam: false,
                    prompt: None,
                    threads: 0,
                };
                let t = whisper::transcribe(&wav, &opts, cx, (0.05, 0.35))?;
                subs::cues_from_transcript(&t, subs::Layout { max_chars: 28, max_lines: 2, max_secs: 4.0, gap: 0.6 })
            }
            "file" => {
                let p = v.path("subtitles").ok_or_else(|| anyhow!("{}", tr("create.audiogram.err.subtitles")))?;
                let text = std::fs::read_to_string(p)?;
                subs::parse(&text)
                    .into_iter()
                    .filter(|c| c.end > start && c.start < start + len)
                    .map(|mut c| {
                        c.start -= start;
                        c.end -= start;
                        c.text = subs::wrap(&c.text.replace('\n', " "), 28, 2);
                        c
                    })
                    .collect()
            }
            _ => Vec::new(),
        };
        // 3. Frames.
        let (w, h) = aspect(v);
        let fps = v.int("fps").clamp(15, 60) as u32;
        let frames = (len * fps as f64).ceil() as usize;
        let title = default_title(v, Some(&input.path));
        let bg = background(v, w, h, &title)?;
        let caption_imgs: Vec<Option<Pixmap>> = cues.iter().map(|c| caption_pixmap(&c.text, w, h, v).ok()).collect();
        let out = cx.output_path(Some(input), "mp4");
        let mut sink = video::FrameSink::start(&out, w, h, fps, video::Codec::Mp4, Some(&wav), cx)?;
        let mut an = Analyzer::new();
        let p0 = if cues.is_empty() && v.str("captions") == "none" { 0.05 } else { 0.35 };
        for i in 0..frames {
            let t = i as f64 / fps as f64;
            let center = (t * RATE as f64) as usize;
            let (levels, wave) = an.frame(&samples, center);
            let mut pm = bg.clone();
            let cap = cues.iter().position(|c| t >= c.start && t < c.end).and_then(|k| caption_imgs[k].as_ref());
            draw_frame(&mut pm, v, &levels, &wave, (t / len) as f32, cap);
            sink.push(&video::pixmap_rgb(&pm))?;
            if i % 10 == 0 {
                cx.progress(p0 + (1.0 - p0) * i as f32 / frames as f32);
            }
        }
        sink.finish()?;
        Ok(vec![Output::new(out).with_note(format!("{} · {}", common::px(w, h), castor_core::units::duration(len)))])
    }
}

const RATE: u32 = 22_050;
const FFT: usize = 2048;
const BANDS: usize = 48;

fn aspect(v: &Values) -> (u32, u32) {
    ASPECTS.iter().find(|a| a.0 == v.str("aspect")).map(|a| (a.1, a.2)).unwrap_or((1080, 1080))
}

fn default_title(v: &Values, input: Option<&Path>) -> String {
    let t = v.str("title").trim();
    if !t.is_empty() {
        return t.to_string();
    }
    input.and_then(|p| p.file_stem()).map(|s| s.to_string_lossy().replace(['_', '-'], " ")).unwrap_or_default()
}

/// Mono f32 samples at `rate` from a media file, via ffmpeg.
fn read_mono(p: &Path, rate: u32) -> Result<Vec<f32>> {
    let ff = engine_media::ffmpeg()?;
    let mut cmd = castor_core::process::command(&ff);
    cmd.args(["-hide_banner", "-loglevel", "error", "-i"]).arg(p).args(["-ac", "1", "-ar", &rate.to_string(), "-f", "f32le", "-"]);
    let out = cmd.output().map_err(|e| anyhow!("ffmpeg: {e}"))?;
    anyhow::ensure!(out.status.success(), "ffmpeg: {}", String::from_utf8_lossy(&out.stderr));
    Ok(out.stdout.chunks_exact(4).map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]])).collect())
}

/// Spectrum bands with smoothing (fast attack, slow release) and a short oscilloscope window.
struct Analyzer {
    smooth: Vec<f32>,
    peak: f32,
}

impl Analyzer {
    fn new() -> Self {
        Analyzer { smooth: vec![0.0; BANDS], peak: 1e-3 }
    }

    fn frame(&mut self, s: &[f32], center: usize) -> (Vec<f32>, Vec<f32>) {
        let start = center.saturating_sub(FFT / 2);
        let mut re = vec![0f32; FFT];
        let mut im = vec![0f32; FFT];
        for i in 0..FFT {
            let x = *s.get(start + i).unwrap_or(&0.0);
            let win = 0.5 - 0.5 * (std::f32::consts::TAU * i as f32 / (FFT - 1) as f32).cos();
            re[i] = x * win;
        }
        fft(&mut re, &mut im);
        // Log-spaced bands from 60 Hz to 8 kHz.
        let bin_hz = RATE as f32 / FFT as f32;
        let mut levels = vec![0f32; BANDS];
        for (b, l) in levels.iter_mut().enumerate() {
            let f0 = 60.0 * (8000.0f32 / 60.0).powf(b as f32 / BANDS as f32);
            let f1 = 60.0 * (8000.0f32 / 60.0).powf((b + 1) as f32 / BANDS as f32);
            let (i0, i1) = ((f0 / bin_hz) as usize, ((f1 / bin_hz) as usize).max((f0 / bin_hz) as usize + 1));
            let mut e = 0f32;
            for k in i0..i1.min(FFT / 2) {
                e = e.max((re[k] * re[k] + im[k] * im[k]).sqrt());
            }
            // Perceptual compression.
            *l = (1.0 + e).ln();
        }
        let mx = levels.iter().cloned().fold(0.0, f32::max);
        self.peak = (self.peak * 0.995).max(mx).max(1e-3);
        for (b, l) in levels.iter_mut().enumerate() {
            let v = (*l / self.peak).clamp(0.0, 1.0);
            let s = &mut self.smooth[b];
            *s = if v > *s { *s + (v - *s) * 0.6 } else { *s + (v - *s) * 0.18 };
            *l = *s;
        }
        let wave: Vec<f32> = (0..512).map(|i| *s.get(center.saturating_sub(512) + i * 2).unwrap_or(&0.0)).collect();
        (levels, wave)
    }
}

/// In-place iterative radix-2 FFT.
fn fft(re: &mut [f32], im: &mut [f32]) {
    let n = re.len();
    let mut j = 0;
    for i in 1..n {
        let mut bit = n >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j |= bit;
        if i < j {
            re.swap(i, j);
            im.swap(i, j);
        }
    }
    let mut len = 2;
    while len <= n {
        let ang = -std::f32::consts::TAU / len as f32;
        let (wr, wi) = (ang.cos(), ang.sin());
        for i in (0..n).step_by(len) {
            let (mut cr, mut ci) = (1.0f32, 0.0f32);
            for k in 0..len / 2 {
                let (ar, ai) = (re[i + k], im[i + k]);
                let (br, bi) = (re[i + k + len / 2] * cr - im[i + k + len / 2] * ci, re[i + k + len / 2] * ci + im[i + k + len / 2] * cr);
                re[i + k] = ar + br;
                im[i + k] = ai + bi;
                re[i + k + len / 2] = ar - br;
                im[i + k + len / 2] = ai - bi;
                let nr = cr * wr - ci * wi;
                ci = cr * wi + ci * wr;
                cr = nr;
            }
        }
        len <<= 1;
    }
}

/// Where things go for this aspect: (wave rect x, y, w, h) and caption baseline y.
fn layout(w: f32, h: f32, style: &str) -> ([f32; 4], f32, [f32; 3]) {
    let portrait = h > w * 1.2;
    let landscape = w > h * 1.2;
    let cover = if style == "circle" {
        [w / 2.0, h * if portrait { 0.42 } else { 0.47 }, w.min(h) * if portrait { 0.18 } else { 0.15 }]
    } else if landscape {
        [w * 0.2, h * 0.38, h * 0.2]
    } else {
        [w / 2.0, h * if portrait { 0.36 } else { 0.38 }, w.min(h) * 0.13]
    };
    let wave = if landscape { [w * 0.42, h * 0.3, w * 0.5, h * 0.22] } else { [w * 0.08, h * if portrait { 0.56 } else { 0.6 }, w * 0.84, h * if portrait { 0.12 } else { 0.15 }] };
    let cap_y = if portrait { h * 0.76 } else if landscape { h * 0.78 } else { h * 0.86 };
    (wave, cap_y, cover)
}

/// Static background: scene, cover, title, subtitle, ornaments.
fn background(v: &Values, w: u32, h: u32, title: &str) -> Result<Pixmap> {
    let scene = raster::scene(v.str("scene"), w, h, 5, v.color("scene_color"));
    let (wf, hf) = (w as f32, h as f32);
    let (_, _, cover) = layout(wf, hf, v.str("style"));
    let (tc, _) = style::svg_paint(v.color("text_color"));
    let (wc, _) = style::svg_paint(v.color("wave_color"));
    let u = wf.min(hf) / 100.0;
    let mut svg = format!("<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 {w} {h}\" width=\"{w}\" height=\"{h}\">");
    let landscape = wf > hf * 1.2;
    if let Some(p) = v.path("cover") {
        let img = png::load_image(p, 1200)?;
        let data = common::base64(&png::encode_rgba(&img)?);
        let r = cover[2];
        let round = if v.str("style") == "circle" { r } else { r * 0.12 };
        svg.push_str(&format!(
            "<defs><clipPath id=\"c\"><rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" rx=\"{round}\"/></clipPath></defs><image x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" preserveAspectRatio=\"xMidYMid slice\" clip-path=\"url(#c)\" href=\"data:image/png;base64,{data}\"/>",
            cover[0] - r, cover[1] - r, 2.0 * r, 2.0 * r, cover[0] - r, cover[1] - r, 2.0 * r, 2.0 * r
        ));
    } else {
        // The diamond motif instead of a cover.
        let r = cover[2] * 0.45;
        svg.push_str(&format!(
            "<path d=\"M{cx} {a}L{b} {cy}L{cx} {c}L{d} {cy}Z\" fill=\"none\" stroke=\"{wc}\" stroke-width=\"{sw}\"/>",
            cx = cover[0],
            cy = cover[1],
            a = cover[1] - r,
            b = cover[0] + r,
            c = cover[1] + r,
            d = cover[0] - r,
            sw = u * 0.6
        ));
    }
    let (tx, ty, anchor) = if landscape { (wf * 0.42, hf * 0.18, "start") } else { (wf / 2.0, hf * 0.1, "middle") };
    if !title.is_empty() {
        let fs = if landscape { 5.5 * u } else { 5.2 * u };
        let t = subs::wrap(title, if landscape { 34 } else { 26 }, 2);
        for (i, line) in t.lines().enumerate() {
            svg.push_str(&format!(
                "<text x=\"{tx}\" y=\"{}\" text-anchor=\"{anchor}\" font-family=\"Latin Modern Sans\" font-weight=\"bold\" font-size=\"{fs}\" fill=\"{tc}\">{}</text>",
                ty + i as f32 * fs * 1.15,
                xml(line)
            ));
        }
    }
    let sub = v.str("subtitle").trim();
    if !sub.is_empty() {
        let lines = subs::wrap(title, if landscape { 34 } else { 26 }, 2).lines().count().max(1) as f32;
        svg.push_str(&format!(
            "<text x=\"{tx}\" y=\"{}\" text-anchor=\"{anchor}\" font-family=\"Latin Modern Roman\" font-style=\"italic\" font-size=\"{}\" fill=\"{tc}\" fill-opacity=\"0.75\">{}</text>",
            ty + lines * 5.6 * u + 1.0 * u,
            3.4 * u,
            xml(sub)
        ));
    }
    svg.push_str("</svg>");
    let overlay = png::pixmap_to_rgba(&png::render_svg(svg.as_bytes(), w, h, false)?);
    let mut img = scene;
    raster::over(&mut img, &overlay, 0, 0);
    let mut pm = Pixmap::new(w, h).ok_or_else(|| anyhow!("frame too large"))?;
    for (d, s) in pm.pixels_mut().iter_mut().zip(img.pixels()) {
        *d = tiny_skia::ColorU8::from_rgba(s[0], s[1], s[2], 255).premultiply();
    }
    Ok(pm)
}

fn caption_pixmap(text: &str, w: u32, h: u32, v: &Values) -> Result<Pixmap> {
    let u = w.min(h) as f32 / 100.0;
    let fs = 4.6 * u;
    let lines: Vec<&str> = text.lines().collect();
    let ch = (lines.len() as f32 * fs * 1.25 + fs * 0.8) as u32;
    let (tc, _) = style::svg_paint(v.color("text_color"));
    let mut svg = format!("<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 {w} {ch}\" width=\"{w}\" height=\"{ch}\">");
    for (i, l) in lines.iter().enumerate() {
        svg.push_str(&format!(
            "<text x=\"{}\" y=\"{}\" text-anchor=\"middle\" font-family=\"Latin Modern Sans\" font-weight=\"bold\" font-size=\"{fs}\" fill=\"{tc}\" stroke=\"#000\" stroke-opacity=\"0.35\" stroke-width=\"{}\" paint-order=\"stroke\">{}</text>",
            w as f32 / 2.0,
            fs * 1.05 + i as f32 * fs * 1.25,
            fs * 0.12,
            xml(l)
        ));
    }
    svg.push_str("</svg>");
    png::render_svg(svg.as_bytes(), w, ch, false)
}

fn paint(c: [u8; 4], alpha: f32) -> Paint<'static> {
    let mut p = Paint::default();
    p.set_color_rgba8(c[0], c[1], c[2], (c[3] as f32 * alpha) as u8);
    p.anti_alias = true;
    p
}

fn draw_frame(pm: &mut Pixmap, v: &Values, levels: &[f32], wave: &[f32], progress: f32, caption: Option<&Pixmap>) {
    let (w, h) = (pm.width() as f32, pm.height() as f32);
    let (r, cap_y, cover) = layout(w, h, v.str("style"));
    let col = v.color("wave_color");
    let u = w.min(h) / 100.0;
    match v.str("style") {
        "line" => {
            let mut pb = PathBuilder::new();
            let n = wave.len();
            for (i, s) in wave.iter().enumerate() {
                let x = r[0] + r[2] * i as f32 / (n - 1) as f32;
                // Taper the ends so the line starts and ends flat.
                let taper = (std::f32::consts::PI * i as f32 / (n - 1) as f32).sin();
                let y = r[1] + r[3] / 2.0 - s.clamp(-1.0, 1.0) * r[3] * 0.9 * taper;
                if i == 0 { pb.move_to(x, y) } else { pb.line_to(x, y) }
            }
            if let Some(p) = pb.finish() {
                let st = Stroke { width: (0.5 * u).max(1.5), line_cap: tiny_skia::LineCap::Round, line_join: tiny_skia::LineJoin::Round, ..Default::default() };
                pm.stroke_path(&p, &paint(col, 1.0), &st, Transform::identity(), None);
            }
        }
        "circle" => {
            let (cx, cy, rad) = (cover[0], cover[1], cover[2] * 1.12);
            let n = levels.len() * 2;
            let mut pb = PathBuilder::new();
            for i in 0..n {
                // Mirror the spectrum around the circle.
                let l = levels[if i < levels.len() { i } else { n - 1 - i }];
                let a = i as f32 / n as f32 * std::f32::consts::TAU - std::f32::consts::FRAC_PI_2;
                let len = rad * (0.06 + 0.55 * l);
                let (c, s) = (a.cos(), a.sin());
                pb.move_to(cx + c * rad, cy + s * rad);
                pb.line_to(cx + c * (rad + len), cy + s * (rad + len));
            }
            if let Some(p) = pb.finish() {
                let st = Stroke { width: (rad * std::f32::consts::TAU / n as f32 * 0.55).max(1.5), line_cap: tiny_skia::LineCap::Round, ..Default::default() };
                pm.stroke_path(&p, &paint(col, 1.0), &st, Transform::identity(), None);
            }
        }
        _ => {
            let n = levels.len();
            let bw = r[2] / n as f32;
            let mut pb = PathBuilder::new();
            for (i, l) in levels.iter().enumerate() {
                let bh = (r[3] * (0.04 + 0.96 * l)).max(bw * 0.6);
                let x = r[0] + i as f32 * bw + bw * 0.18;
                let y = r[1] + (r[3] - bh) / 2.0;
                if let Some(rect) = tiny_skia::Rect::from_xywh(x, y, bw * 0.64, bh) {
                    pb.push_rect(rect);
                }
            }
            if let Some(p) = pb.finish() {
                pm.fill_path(&p, &paint(col, 1.0), FillRule::Winding, Transform::identity(), None);
            }
        }
    }
    if v.bool("progress") {
        let y = h - 3.0 * u;
        let (x0, x1) = (w * 0.08, w * 0.92);
        if let Some(rect) = tiny_skia::Rect::from_xywh(x0, y, x1 - x0, (0.25 * u).max(1.0)) {
            pm.fill_rect(rect, &paint(v.color("text_color"), 0.25), Transform::identity(), None);
        }
        if let Some(rect) = tiny_skia::Rect::from_xywh(x0, y, (x1 - x0) * progress.clamp(0.0, 1.0), (0.25 * u).max(1.0)) {
            pm.fill_rect(rect, &paint(col, 1.0), Transform::identity(), None);
        }
    }
    if let Some(c) = caption {
        let y = cap_y - c.height() as f32 / 2.0;
        pm.draw_pixmap(0, y as i32, c.as_ref(), &PixmapPaint::default(), Transform::identity(), None);
    }
}

#[allow(dead_code)]
fn unused(_: PathBuf) {}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fft_finds_a_tone() {
        let mut re: Vec<f32> = (0..FFT).map(|i| (std::f32::consts::TAU * 64.0 * i as f32 / FFT as f32).sin()).collect();
        let mut im = vec![0f32; FFT];
        fft(&mut re, &mut im);
        let mags: Vec<f32> = (0..FFT / 2).map(|k| re[k].hypot(im[k])).collect();
        let best = mags.iter().enumerate().max_by(|a, b| a.1.partial_cmp(b.1).unwrap()).unwrap().0;
        assert_eq!(best, 64);
    }
    #[test]
    fn every_style_draws() {
        let t = AudiogramTool::new();
        for s in ["bars", "line", "circle"] {
            let mut v = Values::defaults(&t.options());
            v.set("style", castor_core::Value::Text(s.into()));
            let mut pm = background(&v, 200, 200, "Title").unwrap();
            draw_frame(&mut pm, &v, &[0.5; BANDS], &[0.2; 512], 0.5, None);
        }
    }
}
