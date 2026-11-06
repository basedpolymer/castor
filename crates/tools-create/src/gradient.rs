//! `create.gradient`: mesh gradients — 4 to 8 coloured control points blended in OKLab, a
//! domain-warp distortion, soft lighting and film grain — as images of any size or seamless
//! looping videos (the points travel on closed orbits).

use anyhow::Result;
use castor_core::options::{Choice, Cond, OptionSpec, Values};
use castor_core::{FileInfo, InputSpec, Job, Output, Preview, RunCx, Tool, ToolMeta, tr};
use rayon::prelude::*;

use crate::common::{self, Rng};
use crate::render::noise::{Noise, hash2};
use crate::render::{png, style, video};

pub struct GradientTool {
    meta: ToolMeta,
}

impl GradientTool {
    pub fn new() -> Self {
        GradientTool {
            meta: common::meta(
                "create.gradient",
                InputSpec::none(),
                "PNG · JPG · WEBP · MP4 · WEBM",
                &["gradient", "mesh", "background", "wallpaper", "blur", "aurora", "loop"],
                "gradient",
            ),
        }
    }
}

pub const PALETTES: &[(&str, &[&str])] = &[
    ("castor", &["#ffeca4", "#fbe2ff", "#ade1ff", "#aafaba", "#ffdcc8", "#ccfaff"]),
    ("sunset", &["#ff9a8b", "#ff6a88", "#ffd29d", "#a18cd1", "#fbc2eb"]),
    ("ocean", &["#0b3d91", "#1fa2ff", "#12d8fa", "#a6ffcb", "#2f6db5"]),
    ("aurora", &["#0f2027", "#2c5364", "#4ade80", "#a78bfa", "#22d3ee"]),
    ("peach", &["#ffecd2", "#fcb69f", "#ff9a9e", "#fecfef", "#fff1eb"]),
    ("forest", &["#134e5e", "#71b280", "#d4fc79", "#96e6a1", "#0b2e2a"]),
    ("dusk", &["#2b1055", "#7597de", "#d38312", "#a83279", "#3a1c71"]),
    ("ink", &["#1b1616", "#3a3833", "#5c5a55", "#8c8a84", "#d9d6cf"]),
];

pub fn parse_colors(s: &str) -> Vec<[u8; 4]> {
    s.split([',', ';', ' ', '\n'])
        .filter_map(|t| castor_core::options::parse_hex_color(t.trim()))
        .collect()
}

pub fn palette(v: &Values) -> Vec<[u8; 4]> {
    let name = v.str("palette");
    let cols = if name == "custom" {
        parse_colors(v.str("colors"))
    } else {
        PALETTES.iter().find(|(n, _)| *n == name).map(|(_, c)| c.iter().filter_map(|h| castor_core::options::parse_hex_color(h)).collect()).unwrap_or_default()
    };
    if cols.len() >= 2 { cols } else { PALETTES[0].1.iter().filter_map(|h| castor_core::options::parse_hex_color(h)).collect() }
}

impl Tool for GradientTool {
    fn meta(&self) -> &ToolMeta {
        &self.meta
    }

    fn options(&self) -> Vec<OptionSpec> {
        let mut pal: Vec<Choice> = PALETTES.iter().map(|(n, _)| Choice::key(n)).collect();
        pal.push(Choice::key("custom"));
        vec![
            OptionSpec::choice("palette", pal, "castor"),
            OptionSpec::text("colors", "#ffeca4, #fbe2ff, #ade1ff, #aafaba").when(Cond::Eq("palette", "custom")),
            OptionSpec::int("points", 4, 8, 6).slider(),
            OptionSpec::int("seed", 0, 999_999, 7),
            OptionSpec::int("distortion", 0, 100, 45).slider().unit("%"),
            OptionSpec::int("softness", 0, 100, 55).slider().unit("%"),
            OptionSpec::int("lighting", 0, 100, 30).slider().unit("%"),
            OptionSpec::int("grain", 0, 100, 18).slider().unit("%"),
            OptionSpec::size("size", 1920, 1080),
            OptionSpec::choice(
                "format",
                vec![Choice::lit("png", "PNG"), Choice::lit("jpg", "JPG"), Choice::lit("webp", "WebP"), Choice::lit("mp4", "MP4"), Choice::lit("webm", "WebM")],
                "png",
            ),
            OptionSpec::float("duration", 2.0, 60.0, 8.0).unit("s").when(Cond::Eq("format", "mp4")),
            OptionSpec::int("fps", 12, 60, 30).unit("fps").when(Cond::Eq("format", "mp4")).advanced(),
            OptionSpec::text("file_name", "").advanced(),
        ]
    }

    fn preview(&self, _input: Option<&FileInfo>, v: &Values, max_side: u32) -> Result<Option<Preview>> {
        let (w, h) = v.size("size");
        let (w, h) = fit(w.max(16), h.max(16), max_side);
        let g = Mesh::new(v);
        let img = g.render(w, h, 0.0);
        let (ow, oh) = v.size("size");
        Ok(Some(common::preview(png::encode_rgb(&img)?, common::figure(&format!("{} · {}", common::px(ow, oh), tr(&format!("create.gradient.opt.palette.{}", v.str("palette"))))))))
    }

    fn run(&self, _job: &Job, v: &Values, cx: &RunCx) -> Result<Vec<Output>> {
        let (w, h) = v.size("size");
        anyhow::ensure!((16..=16384).contains(&w) && (16..=16384).contains(&h), "{}", tr("create.common.err.size"));
        let g = Mesh::new(v);
        let fmt = v.str("format");
        let out = common::out_named(cx, v.str("file_name"), fmt);
        match fmt {
            "mp4" | "webm" => {
                let (w, h) = (w & !1, h & !1);
                let fps = v.int("fps").clamp(12, 60) as u32;
                let frames = (v.float("duration").clamp(1.0, 120.0) * fps as f64).round() as u32;
                let mut sink = video::FrameSink::start(&out, w, h, fps, video::Codec::from_ext(fmt), None, cx)?;
                for i in 0..frames {
                    let img = g.render(w, h, i as f32 / frames as f32);
                    sink.push(img.as_raw())?;
                    cx.progress((i + 1) as f32 / frames as f32);
                }
                sink.finish()?;
                Ok(vec![Output::new(out).with_note(format!("{} · {} s loop", common::px(w, h), frames / fps))])
            }
            _ => {
                let img = g.render(w, h, 0.0);
                cx.progress(0.8);
                save_rgb(&img, &out, fmt)?;
                Ok(vec![Output::new(out).with_note(common::px(w, h))])
            }
        }
    }
}

/// Save an RGB image as PNG / JPG (q 92) / WebP (lossless).
pub fn save_rgb(img: &image::RgbImage, out: &std::path::Path, fmt: &str) -> Result<()> {
    let data = match fmt {
        "jpg" | "jpeg" => {
            let mut b = Vec::new();
            image::codecs::jpeg::JpegEncoder::new_with_quality(&mut b, 92).encode_image(img)?;
            b
        }
        "webp" => {
            let mut b = Vec::new();
            image::codecs::webp::WebPEncoder::new_lossless(&mut b).encode(img.as_raw(), img.width(), img.height(), image::ExtendedColorType::Rgb8)?;
            b
        }
        _ => png::encode_rgb(img)?,
    };
    common::write(out, &data)
}

/// Fit (w, h) into a square of `side`, keeping the ratio.
pub fn fit(w: u32, h: u32, side: u32) -> (u32, u32) {
    let k = side as f32 / w.max(h) as f32;
    if k >= 1.0 {
        return (w, h);
    }
    (((w as f32 * k).round() as u32).max(1), ((h as f32 * k).round() as u32).max(1))
}

pub struct Mesh {
    pts: Vec<([f32; 2], [f32; 2], f32, [f32; 3])>, // base, orbit radii, phase, OKLab colour
    noise: Noise,
    distortion: f32,
    softness: f32,
    lighting: f32,
    grain: f32,
    seed: u32,
}

impl Mesh {
    pub fn new(v: &Values) -> Mesh {
        let pct = |k: &str| v.int(k).clamp(0, 100) as f32 / 100.0;
        Mesh::with(&palette(v), v.int("points").clamp(4, 8) as usize, v.int("seed").max(0) as u64, [pct("distortion"), pct("softness"), pct("lighting"), pct("grain")])
    }

    /// `look` = [distortion, softness, lighting, grain], each 0..1.
    pub fn with(cols: &[[u8; 4]], n: usize, seed: u64, look: [f32; 4]) -> Mesh {
        let cols = if cols.is_empty() { vec![style::PAPER, style::CREATE_TINT] } else { cols.to_vec() };
        let mut rng = Rng::new(seed);
        // Jittered positions spread over the canvas (and a little beyond its edges).
        let mut pts = Vec::with_capacity(n);
        let cols_shuffled: Vec<[u8; 4]> = {
            let mut c = cols.clone();
            for i in (1..c.len()).rev() {
                let j = (rng.next_u64() % (i as u64 + 1)) as usize;
                c.swap(i, j);
            }
            c
        };
        for i in 0..n {
            let a = i as f32 / n as f32 * std::f32::consts::TAU + rng.range(-0.4, 0.4);
            let r = if i % 3 == 0 { rng.range(0.0, 0.25) } else { rng.range(0.35, 0.62) };
            let base = [0.5 + r * a.cos(), 0.5 + r * a.sin()];
            let orbit = [rng.range(0.06, 0.16), rng.range(0.06, 0.16)];
            let c = cols_shuffled[i % cols_shuffled.len()];
            pts.push((base, orbit, rng.range(0.0, std::f32::consts::TAU), style::to_oklab(c)));
        }
        Mesh {
            pts,
            noise: Noise::new(seed.wrapping_add(17)),
            distortion: look[0],
            softness: look[1],
            lighting: look[2],
            grain: look[3],
            seed: seed as u32,
        }
    }

    /// Render at time `t` in [0, 1) of the loop.
    pub fn render(&self, w: u32, h: u32, t: f32) -> image::RgbImage {
        let tau = std::f32::consts::TAU;
        let pos: Vec<([f32; 2], [f32; 3])> = self
            .pts
            .iter()
            .enumerate()
            .map(|(i, (b, o, ph, c))| {
                let k = if i % 2 == 0 { 1.0 } else { -1.0 };
                ([b[0] + o[0] * (tau * t + ph).cos(), b[1] + o[1] * (k * tau * t + ph).sin()], *c)
            })
            .collect();
        let aspect = w as f32 / h as f32;
        let (sx, sy) = if aspect >= 1.0 { (aspect, 1.0) } else { (1.0, 1.0 / aspect) };
        // Sharpness of the blend: soft = wide gaussian kernels.
        let k = 4.0 + (1.0 - self.softness) * 26.0;
        let warp = self.distortion * 0.35;
        let (ox, oy) = (0.9 * (tau * t).cos(), 0.9 * (tau * t).sin());
        let mut buf = vec![0u8; (w * h * 3) as usize];
        buf.par_chunks_mut((w * 3) as usize).enumerate().for_each(|(y, row)| {
            let fy = (y as f32 + 0.5) / h as f32;
            for x in 0..w as usize {
                let fx = (x as f32 + 0.5) / w as f32;
                let (mut px, mut py) = (fx, fy);
                if warp > 0.0 {
                    let nx = self.noise.fbm(fx * sx * 1.6 + ox, fy * sy * 1.6 + oy, 0.3, 3);
                    let ny = self.noise.fbm(fx * sx * 1.6 + 5.2 + ox, fy * sy * 1.6 + 1.3 + oy, 7.1, 3);
                    px += warp * nx;
                    py += warp * ny;
                }
                let mut acc = [0.0f32; 3];
                let mut wsum = 0.0;
                for (p, c) in &pos {
                    let dx = (px - p[0]) * sx;
                    let dy = (py - p[1]) * sy;
                    let wgt = (-(dx * dx + dy * dy) * k).exp() + 1e-6;
                    acc[0] += c[0] * wgt;
                    acc[1] += c[1] * wgt;
                    acc[2] += c[2] * wgt;
                    wsum += wgt;
                }
                let mut lab = [acc[0] / wsum, acc[1] / wsum, acc[2] / wsum];
                if self.lighting > 0.0 {
                    // Soft light from the top-left plus gentle folds from the noise field.
                    let fold = self.noise.fbm(px * sx * 2.2 + ox * 0.5, py * sy * 2.2, 3.7, 2);
                    let glow = 1.0 - ((fx - 0.15).powi(2) + (fy - 0.1).powi(2)).sqrt();
                    lab[0] += self.lighting * (0.06 * fold + 0.05 * (glow - 0.5));
                }
                let mut lin = style::oklab_to_linear(lab);
                let gr = hash2(x as u32, y as u32, self.seed ^ (t * 1000.0) as u32) - 0.5;
                let g = self.grain * 0.06 * gr + gr * (1.0 / 255.0);
                for c in lin.iter_mut() {
                    *c = (*c + g * (0.2 + *c)).max(0.0);
                }
                row[x * 3] = style::linear_to_srgb(lin[0]);
                row[x * 3 + 1] = style::linear_to_srgb(lin[1]);
                row[x * 3 + 2] = style::linear_to_srgb(lin[2]);
            }
        });
        image::RgbImage::from_raw(w, h, buf).expect("buffer size")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn renders_and_loops() {
        let t = GradientTool::new();
        let v = Values::defaults(&t.options());
        let g = Mesh::new(&v);
        let a = g.render(64, 36, 0.0);
        let b = g.render(64, 36, 0.999_99);
        let diff: u32 = a.as_raw().iter().zip(b.as_raw()).map(|(x, y)| (*x as i32 - *y as i32).unsigned_abs()).max().unwrap();
        assert!(diff < 20, "loop is not seamless: {diff}");
        assert_eq!(a.dimensions(), (64, 36));
    }
}
