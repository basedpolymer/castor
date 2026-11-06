//! `create.motion`: short motion graphics — lower third, text reveal, logo sting, title card —
//! with a transparent background (ProRes 4444 MOV or VP9 WebM with alpha) or on a colour (MP4).
//! Each frame is an SVG scene (resvg) driven by eased keyframes.

use anyhow::Result;
use castor_core::options::{Choice, Cond, OptionSpec, Values};
use castor_core::{FileInfo, InputSpec, Job, Output, Preview, Requirement, RunCx, Tool, ToolMeta, tr};

use crate::common::{self, xml};
use crate::render::{png, style, video};

pub struct MotionTool {
    meta: ToolMeta,
}

impl MotionTool {
    pub fn new() -> Self {
        MotionTool {
            meta: common::serial(common::status(
                common::with_requires(
                    common::meta(
                        "create.motion",
                        InputSpec::none(),
                        "MOV · WEBM · MP4",
                        &["motion", "lower third", "title", "animation", "logo sting", "intro", "alpha", "overlay", "text animation"],
                        "motion",
                    ),
                    &[Requirement::Ffmpeg],
                ),
                castor_core::ToolStatus::Beta,
            )),
        }
    }
}

pub const PRESETS: &[&str] = &["lower-third", "reveal", "sting", "title"];

impl Tool for MotionTool {
    fn meta(&self) -> &ToolMeta {
        &self.meta
    }

    fn options(&self) -> Vec<OptionSpec> {
        vec![
            OptionSpec::choice("preset", PRESETS.iter().map(|p| Choice::key(p)).collect(), "lower-third"),
            OptionSpec::text("title", "Ada Example"),
            OptionSpec::text("subtitle", "Typographer, Castor"),
            OptionSpec::file("logo", &["png", "svg", "jpg", "jpeg", "webp"]).when(Cond::Eq("preset", "sting")),
            OptionSpec::color("accent", "#ffeca4"),
            OptionSpec::color("color", "#1b1616"),
            OptionSpec::color("background", "#00000000"),
            OptionSpec::float("duration", 1.5, 15.0, 4.0).unit("s"),
            OptionSpec::size("size", 1920, 1080),
            OptionSpec::choice("format", vec![Choice::lit("mov", "MOV (ProRes 4444, alpha)"), Choice::lit("webm", "WebM (VP9, alpha)"), Choice::lit("mp4", "MP4")], "mov"),
            OptionSpec::int("fps", 24, 60, 30).unit("fps").advanced(),
        ]
    }

    fn unavailable(&self) -> Option<String> {
        if video::available() { None } else { Some(tr("create.audiogram.err.ffmpeg")) }
    }

    fn preview(&self, _input: Option<&FileInfo>, v: &Values, max_side: u32) -> Result<Option<Preview>> {
        let (w, h) = v.size("size");
        let (pw, ph) = crate::gradient::fit(w.max(64), h.max(64), max_side);
        let logo = logo_data(v)?;
        // The "hold" moment, on a checkerboard so transparency shows.
        let svg = frame(v, w as f32, h as f32, 0.5, logo.as_deref());
        let pm = png::render_svg(svg.as_bytes(), pw, ph, false)?;
        let mut img = checker(pw, ph);
        image::imageops::overlay(&mut img, &png::pixmap_to_rgba(&pm), 0, 0);
        Ok(Some(common::preview(png::encode_rgba(&img)?, common::figure(&format!("{} · {:.1} s", common::px(w, h), v.float("duration"))))))
    }

    fn run(&self, _job: &Job, v: &Values, cx: &RunCx) -> Result<Vec<Output>> {
        let (w, h) = v.size("size");
        anyhow::ensure!((64..=4096).contains(&w) && (64..=4096).contains(&h), "{}", tr("create.common.err.size"));
        let (w, h) = (w & !1, h & !1);
        let fps = v.int("fps").clamp(24, 60) as u32;
        let n = (v.float("duration").clamp(1.5, 15.0) * fps as f64).round() as u32;
        let fmt = v.str("format");
        let codec = match fmt {
            "webm" => video::Codec::WebmAlpha,
            "mp4" => video::Codec::Mp4,
            _ => video::Codec::ProresAlpha,
        };
        let out = cx.output_path(None, fmt);
        let logo = logo_data(v)?;
        let mut sink = video::FrameSink::start(&out, w, h, fps, codec, None, cx)?;
        let bg = v.color("background");
        for i in 0..n {
            let t = i as f32 / (n - 1).max(1) as f32;
            let svg = frame(v, w as f32, h as f32, t, logo.as_deref());
            let pm = png::render_svg(svg.as_bytes(), w, h, false)?;
            if codec.has_alpha() {
                sink.push(&video::pixmap_rgba(&pm))?;
            } else {
                // MP4 has no alpha: composite on the background colour (white if transparent).
                let base = if bg[3] == 0 { [255, 255, 255] } else { [bg[0], bg[1], bg[2]] };
                sink.push(png::flatten(&png::pixmap_to_rgba(&pm), base).as_raw())?;
            }
            cx.progress((i + 1) as f32 / n as f32);
        }
        sink.finish()?;
        Ok(vec![Output::new(out).with_note(format!("{} · {} frames", common::px(w, h), n))])
    }
}

fn checker(w: u32, h: u32) -> image::RgbaImage {
    image::RgbaImage::from_fn(w, h, |x, y| if (x / 12 + y / 12) % 2 == 0 { image::Rgba([236, 234, 229, 255]) } else { image::Rgba([250, 249, 246, 255]) })
}

fn logo_data(v: &Values) -> Result<Option<String>> {
    match v.path("logo") {
        Some(p) => Ok(Some(common::base64(&png::encode_rgba(&png::load_image(p, 1024)?)?))),
        None => Ok(None),
    }
}

/// Smooth in-out (cubic).
fn ease(x: f32) -> f32 {
    let x = x.clamp(0.0, 1.0);
    if x < 0.5 { 4.0 * x * x * x } else { 1.0 - (-2.0 * x + 2.0).powi(3) / 2.0 }
}

/// Out-back with a gentle overshoot.
fn ease_back(x: f32) -> f32 {
    let x = x.clamp(0.0, 1.0);
    let (c1, c3) = (1.2f32, 2.2f32);
    1.0 + c3 * (x - 1.0).powi(3) + c1 * (x - 1.0).powi(2)
}

/// Progress of a segment [a, b] at time t (0..1 of the clip).
fn seg(t: f32, a: f32, b: f32) -> f32 {
    ((t - a) / (b - a)).clamp(0.0, 1.0)
}

/// In over [0, 0.25], out over [0.8, 1]: combined visibility.
fn inout(t: f32, a_in: f32, b_in: f32) -> f32 {
    ease(seg(t, a_in, b_in)) * (1.0 - ease(seg(t, 0.82, 0.97)))
}

/// One frame as SVG, t in 0..1.
pub fn frame(v: &Values, w: f32, h: f32, t: f32, logo: Option<&str>) -> String {
    let u = h / 100.0;
    let (acc, _) = style::svg_paint(v.color("accent"));
    let (fg, _) = style::svg_paint(v.color("color"));
    let bg = v.color("background");
    let (bgc, bga) = style::svg_paint(bg);
    let title = xml(v.str("title"));
    let sub = xml(v.str("subtitle"));
    let mut s = format!("<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 {w} {h}\" width=\"{w}\" height=\"{h}\">");
    if bg[3] > 0 {
        s.push_str(&format!("<rect width=\"{w}\" height=\"{h}\" fill=\"{bgc}\" fill-opacity=\"{bga}\"/>"));
    }
    match v.str("preset") {
        "reveal" => {
            // Words rise one after another out of a mask line.
            let words: Vec<&str> = v.str("title").split_whitespace().collect();
            let fs = 9.0 * u;
            let y = h * 0.5;
            s.push_str(&format!("<defs><clipPath id=\"m\"><rect x=\"0\" y=\"{}\" width=\"{w}\" height=\"{}\"/></clipPath></defs><g clip-path=\"url(#m)\">", y - fs * 1.05, fs * 1.35));
            let total: f32 = words.iter().map(|w| w.chars().count() as f32 + 1.0).sum::<f32>().max(1.0);
            let mut x = w / 2.0 - total * fs * 0.27;
            let out = 1.0 - ease(seg(t, 0.82, 0.97));
            for (i, word) in words.iter().enumerate() {
                let k = ease_back(seg(t, 0.05 + i as f32 * 0.07, 0.30 + i as f32 * 0.07));
                let dy = (1.0 - k) * fs * 1.2 + (1.0 - out) * -fs * 1.2;
                s.push_str(&format!(
                    "<text x=\"{x}\" y=\"{}\" font-family=\"Latin Modern Sans\" font-weight=\"bold\" font-size=\"{fs}\" fill=\"{fg}\">{}</text>",
                    y + dy,
                    xml(word)
                ));
                x += (word.chars().count() as f32 + 1.0) * fs * 0.54;
            }
            s.push_str("</g>");
            let a = inout(t, 0.35, 0.6);
            s.push_str(&format!(
                "<text x=\"{}\" y=\"{}\" text-anchor=\"middle\" font-family=\"Latin Modern Roman\" font-style=\"italic\" font-size=\"{}\" fill=\"{fg}\" opacity=\"{a}\">{sub}</text>",
                w / 2.0,
                y + 5.5 * u,
                3.6 * u
            ));
            let lw = w * 0.3 * ease(seg(t, 0.0, 0.3)) * out;
            s.push_str(&format!("<rect x=\"{}\" y=\"{}\" width=\"{lw}\" height=\"{}\" fill=\"{acc}\"/>", w / 2.0 - lw / 2.0, y + 1.2 * u, 0.35 * u));
        }
        "sting" => {
            // The diamond draws itself, the logo (or title) scales in, a glint, out.
            let cx = w / 2.0;
            let cy = h / 2.0;
            let r = 18.0 * u;
            let draw = ease(seg(t, 0.0, 0.35));
            let per = 4.0 * r * std::f32::consts::SQRT_2;
            let out = 1.0 - ease(seg(t, 0.85, 1.0));
            s.push_str(&format!(
                "<path d=\"M{cx} {}L{} {cy}L{cx} {}L{} {cy}Z\" fill=\"none\" stroke=\"{acc}\" stroke-width=\"{}\" stroke-dasharray=\"{per}\" stroke-dashoffset=\"{}\" opacity=\"{out}\"/>",
                cy - r,
                cx + r,
                cy + r,
                cx - r,
                0.8 * u,
                per * (1.0 - draw)
            ));
            let k = ease_back(seg(t, 0.25, 0.55)) * out;
            match logo {
                Some(data) => {
                    let sz = r * 1.1 * k.max(0.001);
                    s.push_str(&format!(
                        "<image x=\"{}\" y=\"{}\" width=\"{sz}\" height=\"{sz}\" preserveAspectRatio=\"xMidYMid meet\" href=\"data:image/png;base64,{data}\"/>",
                        cx - sz / 2.0,
                        cy - sz / 2.0
                    ));
                }
                None => {
                    let fs = 7.0 * u * k.max(0.001);
                    s.push_str(&format!(
                        "<text x=\"{cx}\" y=\"{}\" text-anchor=\"middle\" font-family=\"TeX Gyre Chorus\" font-size=\"{fs}\" fill=\"{fg}\">{}</text>",
                        cy + fs * 0.3,
                        xml(&v.str("title").chars().take(1).collect::<String>())
                    ));
                }
            }
            let a = inout(t, 0.45, 0.7);
            s.push_str(&format!(
                "<text x=\"{cx}\" y=\"{}\" text-anchor=\"middle\" font-family=\"Latin Modern Sans\" font-weight=\"bold\" font-size=\"{}\" letter-spacing=\"{}\" fill=\"{fg}\" opacity=\"{a}\">{}</text>",
                cy + r + 8.0 * u,
                4.2 * u,
                0.5 * u,
                xml(&v.str("title").to_uppercase())
            ));
        }
        "title" => {
            let a = inout(t, 0.0, 0.3);
            let ghost = inout(t, 0.1, 0.45) * 0.1;
            let fs = 10.0 * u;
            let initial = xml(&v.str("title").chars().take(1).collect::<String>());
            s.push_str(&format!(
                "<text x=\"{}\" y=\"{}\" font-family=\"TeX Gyre Chorus\" font-size=\"{}\" fill=\"{fg}\" opacity=\"{ghost}\">{initial}</text>",
                w * 0.12 - 3.0 * u,
                h * 0.5 + 6.0 * u,
                fs * 3.6
            ));
            let dx = (1.0 - ease(seg(t, 0.0, 0.3))) * -4.0 * u;
            s.push_str(&format!(
                "<text x=\"{}\" y=\"{}\" font-family=\"Latin Modern Sans\" font-weight=\"bold\" font-size=\"{fs}\" fill=\"{fg}\" opacity=\"{a}\">{title}</text>",
                w * 0.12 + dx,
                h * 0.5
            ));
            let b = inout(t, 0.2, 0.45);
            s.push_str(&format!(
                "<text x=\"{}\" y=\"{}\" font-family=\"Latin Modern Roman\" font-style=\"italic\" font-size=\"{}\" fill=\"{fg}\" opacity=\"{b}\">{sub}</text>",
                w * 0.12,
                h * 0.5 + 7.0 * u,
                4.0 * u
            ));
            let lw = w * 0.76 * ease(seg(t, 0.15, 0.55)) * (1.0 - ease(seg(t, 0.82, 0.97)));
            let d = 0.9 * u;
            s.push_str(&format!(
                "<rect x=\"{}\" y=\"{}\" width=\"{lw}\" height=\"{}\" fill=\"{fg}\" opacity=\"{a}\"/><path d=\"M{} {}l{d} {d}l-{d} {d}l-{d} -{d}Z\" fill=\"none\" stroke=\"{fg}\" stroke-width=\"{}\" opacity=\"{b}\"/>",
                w * 0.12,
                h * 0.5 + 11.0 * u,
                0.18 * u,
                w * 0.12 + lw + d,
                h * 0.5 + 11.0 * u - d + 0.09 * u,
                0.18 * u
            ));
        }
        _ => {
            // Lower third: accent bar slides in, name and role follow, everything slides out.
            let x0 = w * 0.06;
            let y0 = h * 0.74;
            let slide = ease(seg(t, 0.0, 0.22));
            let out = ease(seg(t, 0.82, 0.97));
            let bw = 50.0 * u;
            let bar_w = bw * slide * (1.0 - out);
            s.push_str(&format!("<defs><clipPath id=\"c\"><rect x=\"{x0}\" y=\"{}\" width=\"{}\" height=\"{}\"/></clipPath></defs>", y0 - 9.0 * u, bar_w + 2.0 * u, 16.0 * u));
            s.push_str(&format!("<rect x=\"{x0}\" y=\"{}\" width=\"{}\" height=\"{}\" fill=\"{acc}\"/>", y0 - 7.5 * u, 0.9 * u, 12.5 * u * slide * (1.0 - out)));
            s.push_str("<g clip-path=\"url(#c)\">");
            let a = inout(t, 0.12, 0.32);
            let dx = (1.0 - ease(seg(t, 0.12, 0.32))) * -6.0 * u;
            s.push_str(&format!(
                "<rect x=\"{}\" y=\"{}\" width=\"{bw}\" height=\"{}\" fill=\"{bgf}\" fill-opacity=\"0.92\"/>",
                x0 + 0.9 * u,
                y0 - 7.5 * u,
                12.5 * u,
                bgf = if style::luminance(v.color("color")) < 0.5 { "#fcfbf9" } else { "#1b1616" }
            ));
            s.push_str(&format!(
                "<text x=\"{}\" y=\"{}\" font-family=\"Latin Modern Sans\" font-weight=\"bold\" font-size=\"{}\" fill=\"{fg}\" opacity=\"{a}\">{title}</text>",
                x0 + 3.2 * u + dx,
                y0 - 0.5 * u,
                5.0 * u
            ));
            let b = inout(t, 0.2, 0.4);
            s.push_str(&format!(
                "<text x=\"{}\" y=\"{}\" font-family=\"Latin Modern Roman\" font-style=\"italic\" font-size=\"{}\" fill=\"{fg}\" opacity=\"{b}\">{sub}</text>",
                x0 + 3.2 * u + dx * 0.6,
                y0 + 3.4 * u,
                2.8 * u
            ));
            s.push_str("</g>");
        }
    }
    s.push_str("</svg>");
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn every_preset_draws() {
        let t = MotionTool::new();
        for p in PRESETS {
            let mut v = Values::defaults(&t.options());
            v.set("preset", castor_core::Value::Text(p.to_string()));
            for tt in [0.0, 0.5, 1.0] {
                let svg = frame(&v, 320.0, 180.0, tt, None);
                png::render_svg(svg.as_bytes(), 320, 180, false).unwrap_or_else(|e| panic!("{p}: {e}"));
            }
        }
    }
}
