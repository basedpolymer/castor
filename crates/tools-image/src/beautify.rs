//! image.beautify — screenshots on painted backgrounds: procedural mesh gradients with grain
//! (no external images), padding, rounded corners, soft shadow, window frame, canvas ratio.

use anyhow::Result;
use castor_core::options::{Choice, Cond, OptionSpec, Values};
use castor_core::{FileInfo, Job, Output, Preview, RunCx, Tool, ToolMeta, ToolStatus};
use image::{DynamicImage, Rgba, RgbaImage};
use rayon::prelude::*;

use crate::codec::{DecodeOpts, EncodeOpts, Fmt};
use crate::{common, ops};

pub struct Beautify {
    meta: ToolMeta,
}

/// Painted backgrounds: (id, four colours). Ids are stable.
pub const BACKGROUNDS: &[(&str, [u32; 4])] = &[
    ("aurora", [0x0f2027, 0x2c5364, 0x00c9a7, 0x845ec2]),
    ("sunset", [0xff7e5f, 0xfeb47b, 0xff4e50, 0x6a3093]),
    ("ocean", [0x1cb5e0, 0x000851, 0x00d2ff, 0x3a7bd5]),
    ("peach", [0xffecd2, 0xfcb69f, 0xff9a8b, 0xffd1ff]),
    ("lavender", [0xe0c3fc, 0x8ec5fc, 0xc2a5f5, 0xf5e6ff]),
    ("mint", [0xd4fc79, 0x96e6a1, 0x43e97b, 0xe0f7f4]),
    ("ember", [0xf12711, 0xf5af19, 0x8e0e00, 0xff6a00]),
    ("dusk", [0x2c3e50, 0xfd746c, 0x4b6cb7, 0xff9068]),
    ("sand", [0xe6dada, 0xd9c5a0, 0xf3e7d3, 0xc9a66b]),
    ("forest", [0x134e5e, 0x71b280, 0x0b3d2e, 0xa8e063]),
    ("candy", [0xff9a9e, 0xfad0c4, 0xa18cd1, 0xfbc2eb]),
    ("slate", [0x485563, 0x29323c, 0x8e9eab, 0x5d6d7e]),
    ("midnight", [0x0f0c29, 0x302b63, 0x24243e, 0x6a11cb]),
    ("rose", [0xf4c4f3, 0xfc67fa, 0xee9ca7, 0xffdde1]),
    ("lagoon", [0x43cea2, 0x185a9d, 0x6dd5ed, 0x2193b0]),
    ("citrus", [0xf7971e, 0xffd200, 0xfff3b0, 0xff6f00]),
    ("berry", [0x8e2de2, 0x4a00e0, 0xda22ff, 0x9733ee]),
    ("glacier", [0xe0eafc, 0xcfdef3, 0xa1c4fd, 0xc2e9fb]),
    ("mocha", [0x3e2723, 0x8d6e63, 0xd7ccc8, 0x5d4037]),
    ("neon", [0x00f260, 0x0575e6, 0xf000ff, 0x00e5ff]),
    ("paper", [0xf7f3ea, 0xeee6d6, 0xfbf8f1, 0xe7dcc6]),
    ("graphite", [0x1a1a1a, 0x3a3a3a, 0x262626, 0x4d4d4d]),
];

impl Beautify {
    pub fn new() -> Self {
        Beautify {
            meta: common::meta(
                "image.beautify",
                "PNG",
                &["screenshot", "beautify", "mockup", "background", "shadow", "frame", "window", "pretty", "social", "showcase"],
                "pretty",
                ToolStatus::Ready,
                &[],
                false,
            ),
        }
    }
}

impl Tool for Beautify {
    fn meta(&self) -> &ToolMeta {
        &self.meta
    }

    fn options(&self) -> Vec<OptionSpec> {
        let mut bgs: Vec<Choice> = BACKGROUNDS.iter().map(|(id, _)| Choice::key(id)).collect();
        bgs.push(Choice::key("solid"));
        bgs.push(Choice::key("transparent"));
        vec![
            OptionSpec::choice("background", bgs, "aurora"),
            OptionSpec::color("color", "#f2f2f2").when(Cond::Eq("background", "solid")),
            OptionSpec::int("padding", 0, 40, 8).slider().unit("%"),
            OptionSpec::int("radius", 0, 80, 12).slider().unit("px"),
            OptionSpec::choice("shadow", vec![Choice::key("none"), Choice::key("soft"), Choice::key("strong")], "soft"),
            OptionSpec::choice("frame", vec![Choice::key("none"), Choice::key("macos"), Choice::key("windows")], "none"),
            OptionSpec::choice("frame_theme", vec![Choice::key("light"), Choice::key("dark")], "light").when(Cond::Ne("frame", "none")),
            OptionSpec::choice(
                "ratio",
                vec![
                    Choice::key("auto"),
                    Choice::lit("1-1", "1:1"),
                    Choice::lit("4-5", "4:5"),
                    Choice::lit("16-9", "16:9"),
                    Choice::lit("9-16", "9:16"),
                    Choice::lit("4-3", "4:3"),
                    Choice::lit("3-2", "3:2"),
                ],
                "auto",
            ),
            OptionSpec::toggle("grain", true).advanced(),
        ]
    }

    fn run(&self, job: &Job, v: &Values, cx: &RunCx) -> Result<Vec<Output>> {
        let input = job.input()?;
        let d = common::load(input, DecodeOpts::default())?;
        cx.progress(0.2);
        let out_img = render(&d.image.to_rgba8(), v);
        cx.progress(0.8);
        let o = EncodeOpts { png_effort: 1, icc: d.icc.clone(), ..Default::default() };
        let out = common::save(&DynamicImage::ImageRgba8(out_img), Fmt::Png, &o, Some(input), cx)?;
        Ok(vec![Output::new(out)])
    }

    fn preview(&self, input: Option<&FileInfo>, v: &Values, max_side: u32) -> Result<Option<Preview>> {
        let shot = match input {
            Some(f) => common::load_preview(f, max_side)?.to_rgba8(),
            None => sample_screenshot(),
        };
        common::preview(&render(&shot, v), max_side, None)
    }
}

/// A plain fake window content for previews without a file.
fn sample_screenshot() -> RgbaImage {
    let mut img = RgbaImage::from_pixel(640, 400, Rgba([250, 250, 250, 255]));
    for i in 0..6 {
        let y = 40.0 + i as f32 * 52.0;
        ops::fill_rounded_rect(&mut img, 40.0, y, 560.0 - (i % 3) as f32 * 90.0, 22.0, 6.0, [220, 224, 232, 255]);
    }
    ops::fill_rounded_rect(&mut img, 40.0, 340.0, 140.0, 34.0, 8.0, [64, 120, 242, 255]);
    img
}

fn rgb(c: u32) -> [f32; 3] {
    [((c >> 16) & 255) as f32, ((c >> 8) & 255) as f32, (c & 255) as f32]
}

/// Deterministic hash noise in -1..1.
#[inline]
fn noise(x: u32, y: u32) -> f32 {
    let mut n = x.wrapping_mul(374_761_393) ^ y.wrapping_mul(668_265_263);
    n = (n ^ (n >> 13)).wrapping_mul(1_274_126_177);
    ((n ^ (n >> 16)) & 0xffff) as f32 / 32_767.5 - 1.0
}

/// Paint a mesh-gradient background of size w × h.
pub fn paint(id: &str, w: u32, h: u32, grain: bool) -> RgbaImage {
    let cols = BACKGROUNDS.iter().find(|(i, _)| *i == id).map(|(_, c)| *c).unwrap_or(BACKGROUNDS[0].1);
    let c: Vec<[f32; 3]> = cols.iter().map(|c| rgb(*c)).collect();
    // Blob centres (relative), seeded by the preset name so each background has its own shape.
    let seed = id.bytes().fold(7u32, |a, b| a.wrapping_mul(31).wrapping_add(b as u32));
    let centres: Vec<(f32, f32)> = (0..4)
        .map(|i| {
            let a = noise(seed, i * 17 + 1) * 0.5 + 0.5;
            let b = noise(seed + 3, i * 29 + 5) * 0.5 + 0.5;
            let base = [(0.15, 0.2), (0.85, 0.25), (0.25, 0.85), (0.8, 0.8)][i as usize];
            (base.0 * 0.7 + a * 0.3, base.1 * 0.7 + b * 0.3)
        })
        .collect();
    let mut img = RgbaImage::new(w, h);
    let (fw, fh) = (w as f32, h as f32);
    let scale = fw.max(fh);
    img.par_chunks_mut(w as usize * 4).enumerate().for_each(|(y, row)| {
        let v = y as f32 / scale;
        for x in 0..w as usize {
            let u = x as f32 / scale;
            // Gentle warp: brush-like flowing shapes.
            let wu = u + 0.06 * (v * 6.0 + seed as f32 * 0.001).sin();
            let wv = v + 0.06 * (u * 5.0 + 1.3).cos();
            let mut acc = [0f32; 3];
            let mut wsum = 0f32;
            for (k, (cx, cy)) in centres.iter().enumerate() {
                let dx = wu - cx * fw / scale;
                let dy = wv - cy * fh / scale;
                let d2 = dx * dx + dy * dy;
                let wgt = 1.0 / (d2 * d2 * 40.0 + 0.002);
                for ch in 0..3 {
                    acc[ch] += c[k][ch] * wgt;
                }
                wsum += wgt;
            }
            let g = if grain { noise(x as u32, y as u32) * 5.0 } else { 0.0 };
            let px = &mut row[x * 4..x * 4 + 4];
            for ch in 0..3 {
                px[ch] = (acc[ch] / wsum + g).round().clamp(0.0, 255.0) as u8;
            }
            px[3] = 255;
        }
    });
    img
}

fn frame_bar(content_w: u32, kind: &str, dark: bool) -> (RgbaImage, u32) {
    let bar_h = ((content_w as f32 * 0.035).round() as u32).clamp(24, 72);
    let bg = if dark { [43, 43, 46, 255] } else { [236, 236, 238, 255] };
    let mut bar = RgbaImage::from_pixel(content_w, bar_h, Rgba(bg));
    let r = bar_h as f32 * 0.18;
    let cy = bar_h as f32 / 2.0;
    if kind == "macos" {
        for (i, col) in [[255, 95, 87, 255], [254, 188, 46, 255], [40, 200, 64, 255]].iter().enumerate() {
            ops::fill_circle(&mut bar, bar_h as f32 * 0.6 + i as f32 * r * 3.3, cy, r, *col);
        }
    } else {
        // Windows: minimise, maximise, close glyphs on the right.
        let fg = if dark { [230, 230, 230, 255] } else { [40, 40, 40, 255] };
        let s = bar_h as f32 * 0.28;
        let step = bar_h as f32 * 1.6;
        let x0 = content_w as f32 - step * 2.5;
        let t = (bar_h as f32 / 30.0).max(1.0);
        ops::fill_rounded_rect(&mut bar, x0 - s / 2.0, cy - t / 2.0, s, t, 0.0, fg);
        let x1 = x0 + step;
        for (x, y, w, h) in [(x1 - s / 2.0, cy - s / 2.0, s, t), (x1 - s / 2.0, cy + s / 2.0 - t, s, t), (x1 - s / 2.0, cy - s / 2.0, t, s), (x1 + s / 2.0 - t, cy - s / 2.0, t, s)] {
            ops::fill_rounded_rect(&mut bar, x, y, w, h, 0.0, fg);
        }
        let x2 = x0 + 2.0 * step;
        let n = (s * 2.0) as i32;
        for i in 0..=n {
            let f = i as f32 / n as f32 - 0.5;
            ops::fill_circle(&mut bar, x2 + f * s, cy + f * s, t / 1.6, fg);
            ops::fill_circle(&mut bar, x2 + f * s, cy - f * s, t / 1.6, fg);
        }
    }
    (bar, bar_h)
}

pub fn render(shot: &RgbaImage, v: &Values) -> RgbaImage {
    // 1. Window: optional frame bar + content, rounded corners.
    let frame = v.str("frame");
    let mut window = if frame == "macos" || frame == "windows" {
        let (bar, bh) = frame_bar(shot.width(), frame, v.str("frame_theme") == "dark");
        let mut w = RgbaImage::new(shot.width(), shot.height() + bh);
        ops::overlay(&mut w, &bar, 0, 0, 1.0);
        ops::overlay(&mut w, shot, 0, bh as i64, 1.0);
        w
    } else {
        shot.clone()
    };
    let size = window.width().max(window.height()) as f32;
    // Radius is given for a 1000-px wide window; scale with the image.
    let radius = v.int("radius").max(0) as f32 * (size / 1000.0).max(0.25);
    ops::round_corners(&mut window, radius);

    // 2. Canvas size: padding, then the requested ratio.
    let pad = (size * v.int("padding").clamp(0, 60) as f32 / 100.0).round() as u32;
    let (mut cw, mut ch) = (window.width() + 2 * pad, window.height() + 2 * pad);
    let ratio = match v.str("ratio") {
        "1-1" => Some(1.0),
        "4-5" => Some(0.8),
        "16-9" => Some(16.0 / 9.0),
        "9-16" => Some(9.0 / 16.0),
        "4-3" => Some(4.0 / 3.0),
        "3-2" => Some(1.5),
        _ => None,
    };
    if let Some(r) = ratio {
        if cw as f32 / ch as f32 > r {
            ch = (cw as f32 / r).round() as u32;
        } else {
            cw = (ch as f32 * r).round() as u32;
        }
    }

    // 3. Background.
    let bg_id = v.str("background");
    let mut canvas = match bg_id {
        "transparent" => RgbaImage::new(cw, ch),
        "solid" => RgbaImage::from_pixel(cw, ch, Rgba(v.color("color"))),
        id => paint(id, cw, ch, v.bool("grain") || v.get("grain").is_none()),
    };

    // 4. Shadow + window.
    let x = ((cw - window.width()) / 2) as i64;
    let y = ((ch - window.height()) / 2) as i64;
    let (blur, alpha, dy) = match v.str("shadow") {
        "strong" => (size * 0.035, 150u8, size * 0.025),
        "soft" => (size * 0.025, 90u8, size * 0.012),
        _ => (0.0, 0, 0.0),
    };
    if alpha > 0 {
        // Shadow from a downscaled silhouette (fast on big screenshots), scaled back up.
        let k = (300.0 / size).min(1.0);
        let (sw, sh) = (((window.width() as f32 * k) as u32).max(1), ((window.height() as f32 * k) as u32).max(1));
        let small = ops::resize_rgba(&window, sw, sh);
        let pad_s = (blur * k * 3.0).ceil() as u32 + 2;
        let s = ops::shadow(&small, (blur * k).max(0.8), [0, 0, 0, alpha], pad_s);
        let full_pad = (pad_s as f32 / k).round() as i64;
        let big = ops::resize_rgba(&s, ((s.width() as f32) / k).round() as u32, ((s.height() as f32) / k).round() as u32);
        ops::overlay(&mut canvas, &big, x - full_pad, y - full_pad + dy as i64, 1.0);
    }
    ops::overlay(&mut canvas, &window, x, y, 1.0);
    canvas
}
