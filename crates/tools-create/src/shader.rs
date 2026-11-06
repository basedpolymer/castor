//! `create.shader`: generative backgrounds — waves, silk, noise, aurora, topography, plasma,
//! dunes — rendered on the CPU (rayon), as images or seamless looping videos.

use anyhow::Result;
use castor_core::options::{Choice, Cond, OptionSpec, Values};
use castor_core::{FileInfo, InputSpec, Job, Output, Preview, RunCx, Tool, ToolMeta, tr};
use rayon::prelude::*;

use crate::common;
use crate::gradient::{PALETTES, fit, palette, save_rgb};
use crate::render::noise::{Noise, hash2, smooth};
use crate::render::{png, style, video};

pub struct ShaderTool {
    meta: ToolMeta,
}

impl ShaderTool {
    pub fn new() -> Self {
        ShaderTool {
            meta: common::meta(
                "create.shader",
                InputSpec::none(),
                "PNG · JPG · WEBP · MP4 · WEBM",
                &["shader", "generative", "background", "waves", "silk", "aurora", "topography", "contour", "noise", "loop", "wallpaper"],
                "shader",
            ),
        }
    }
}

pub const PRESETS: &[&str] = &["waves", "silk", "noise", "aurora", "topography", "plasma", "dunes"];

impl Tool for ShaderTool {
    fn meta(&self) -> &ToolMeta {
        &self.meta
    }

    fn options(&self) -> Vec<OptionSpec> {
        let mut pal: Vec<Choice> = PALETTES.iter().map(|(n, _)| Choice::key(n)).collect();
        pal.push(Choice::key("custom"));
        vec![
            OptionSpec::choice("preset", PRESETS.iter().map(|p| Choice::key(p)).collect(), "silk"),
            OptionSpec::choice("palette", pal, "ocean"),
            OptionSpec::text("colors", "#0b3d91, #1fa2ff, #a6ffcb").when(Cond::Eq("palette", "custom")),
            OptionSpec::int("seed", 0, 999_999, 3),
            OptionSpec::int("scale", 10, 400, 100).slider().unit("%"),
            OptionSpec::int("detail", 1, 6, 4).slider(),
            OptionSpec::int("grain", 0, 100, 10).slider().unit("%"),
            OptionSpec::size("size", 1920, 1080),
            OptionSpec::choice(
                "format",
                vec![Choice::lit("png", "PNG"), Choice::lit("jpg", "JPG"), Choice::lit("webp", "WebP"), Choice::lit("mp4", "MP4"), Choice::lit("webm", "WebM")],
                "png",
            ),
            OptionSpec::float("duration", 2.0, 60.0, 10.0).unit("s").when(Cond::Eq("format", "mp4")),
            OptionSpec::int("fps", 12, 60, 30).unit("fps").advanced(),
            OptionSpec::text("file_name", "").advanced(),
        ]
    }

    fn preview(&self, _input: Option<&FileInfo>, v: &Values, max_side: u32) -> Result<Option<Preview>> {
        let (w, h) = v.size("size");
        let (pw, ph) = fit(w.max(16), h.max(16), max_side);
        let s = Shader::new(v);
        let img = s.render(pw, ph, 0.0, w as f32 / pw as f32);
        let label = tr(&format!("create.shader.opt.preset.{}", v.str("preset")));
        Ok(Some(common::preview(png::encode_rgb(&img)?, common::figure(&format!("{label} · {}", common::px(w, h))))))
    }

    fn run(&self, _job: &Job, v: &Values, cx: &RunCx) -> Result<Vec<Output>> {
        let (w, h) = v.size("size");
        anyhow::ensure!((16..=16384).contains(&w) && (16..=16384).contains(&h), "{}", tr("create.common.err.size"));
        let s = Shader::new(v);
        let fmt = v.str("format");
        let out = common::out_named(cx, v.str("file_name"), fmt);
        if matches!(fmt, "mp4" | "webm") {
            let (w, h) = (w & !1, h & !1);
            let fps = v.int("fps").clamp(12, 60) as u32;
            let frames = (v.float("duration").clamp(1.0, 120.0) * fps as f64).round() as u32;
            let mut sink = video::FrameSink::start(&out, w, h, fps, video::Codec::from_ext(fmt), None, cx)?;
            for i in 0..frames {
                sink.push(s.render(w, h, i as f32 / frames as f32, 1.0).as_raw())?;
                cx.progress((i + 1) as f32 / frames as f32);
            }
            sink.finish()?;
            return Ok(vec![Output::new(out).with_note(format!("{} · {:.0} s loop", common::px(w, h), frames as f32 / fps as f32))]);
        }
        let img = s.render(w, h, 0.0, 1.0);
        save_rgb(&img, &out, fmt)?;
        Ok(vec![Output::new(out).with_note(common::px(w, h))])
    }
}

pub struct Shader {
    preset: String,
    stops: Vec<[f32; 3]>,
    noise: Noise,
    scale: f32,
    detail: u32,
    grain: f32,
    seed: u32,
    /// Contour half-width in pixels of this rendering (thinner in small previews).
    line_px: f32,
}

impl Shader {
    pub fn new(v: &Values) -> Shader {
        let cols = palette(v);
        Shader {
            preset: v.str("preset").to_string(),
            stops: cols.iter().map(|c| style::to_oklab(*c)).collect(),
            noise: Noise::new(v.int("seed").max(0) as u64),
            scale: v.int("scale").clamp(10, 400) as f32 / 100.0,
            detail: v.int("detail").clamp(1, 6) as u32,
            grain: v.int("grain").clamp(0, 100) as f32 / 100.0,
            seed: v.int("seed").max(0) as u32,
            line_px: 1.0,
        }
    }

    /// Colour ramp through the palette (OKLab), t in [0, 1].
    fn ramp(&self, t: f32) -> [f32; 3] {
        let n = self.stops.len().max(1);
        if n == 1 {
            return self.stops[0];
        }
        let x = t.clamp(0.0, 1.0) * (n - 1) as f32;
        let i = (x.floor() as usize).min(n - 2);
        let f = smooth(0.0, 1.0, x - i as f32);
        let (a, b) = (self.stops[i], self.stops[i + 1]);
        [a[0] + (b[0] - a[0]) * f, a[1] + (b[1] - a[1]) * f, a[2] + (b[2] - a[2]) * f]
    }

    /// One pixel: (x, y) in canvas units where the short side is 1; t = loop phase.
    fn shade(&self, x: f32, y: f32, t: f32, px: f32) -> [f32; 3] {
        let tau = std::f32::consts::TAU;
        let (c, s) = ((tau * t).cos(), (tau * t).sin());
        let n = &self.noise;
        let k = 1.0 / self.scale;
        let (x, y) = (x * k, y * k);
        match self.preset.as_str() {
            "waves" => {
                let w = n.fbm(x * 0.8 + 0.6 * c, y * 0.8 + 0.6 * s, 1.0, self.detail) * 1.4;
                let v = 0.5 + 0.5 * ((y * 5.0 + w * 3.0 + (x * 1.5 + tau * t).sin() * 0.6) * 1.0).sin();
                let mut lab = self.ramp(v * 0.85 + 0.15 * (0.5 + 0.5 * w));
                lab[0] += 0.04 * (v - 0.5);
                lab
            }
            "silk" => {
                // A height field of warped folds, lit from the top-left: highlights and shade.
                let hf = |x: f32, y: f32| {
                    let w = n.fbm(x * 0.55 + 0.4 * c, y * 0.55 + 0.4 * s, 4.0, self.detail.min(4));
                    ((x * 1.1 + y * 2.3 + w * 3.2) * 5.5).sin() * 0.5 + 0.25 * ((x * 0.7 - y * 1.6 + w * 2.0) * 3.4).sin()
                };
                let e = 0.004;
                let h0 = hf(x, y);
                let (dx, dy) = ((hf(x + e, y) - h0) / e, (hf(x, y + e) - h0) / e);
                let nrm = {
                    let (a, b, cz) = (-dx * 0.05, -dy * 0.05, 1.0f32);
                    let l = (a * a + b * b + cz * cz).sqrt();
                    [a / l, b / l, cz / l]
                };
                let light = [-0.5f32, -0.6, 0.62];
                let ll = (light[0] * light[0] + light[1] * light[1] + light[2] * light[2]).sqrt();
                let diff = ((nrm[0] * light[0] + nrm[1] * light[1] + nrm[2] * light[2]) / ll).max(0.0);
                let spec = diff.powf(24.0);
                let mut lab = self.ramp((0.5 + 0.45 * h0).clamp(0.0, 1.0));
                lab[0] = lab[0] * (0.62 + 0.45 * diff) + 0.12 * spec;
                lab
            }
            "aurora" => {
                // Night sky, then curtains: a bright lower edge, rays fading upwards.
                let sky_top = self.ramp(0.0);
                let mut lab = [sky_top[0] * (0.35 + 0.25 * y.min(1.2)), sky_top[1] * 0.6, sky_top[2] * 0.6];
                for b in 0..2 {
                    let bf = b as f32;
                    let edge = 0.74 - 0.2 * bf + 0.13 * n.fbm(x * 0.8 + bf * 7.1 + 0.6 * c, bf * 3.3 + 0.6 * s, 2.0, 3);
                    let below = ((y - edge) / 0.05).max(0.0);
                    let above = ((edge - y) / (0.32 - 0.08 * bf)).max(0.0);
                    let body = if y > edge { (-below * below).exp() } else { (-above * 2.2).exp() };
                    let rays = 0.55 + 0.45 * (n.fbm(x * 9.0 + bf * 11.0 + 0.4 * c, bf + 0.4 * s, 4.0, 2) * 2.2).clamp(-1.0, 1.0);
                    let glow = body * rays * (0.85 - 0.25 * bf);
                    let col = self.ramp(if y > edge - 0.05 { 0.55 + 0.1 * bf } else { (0.95 - 0.1 * bf).min(1.0) });
                    for i in 0..3 {
                        lab[i] = lab[i] * (1.0 - glow) + col[i] * glow;
                    }
                    lab[0] += 0.06 * glow;
                }
                let st = hash2((x * 520.0) as u32, (y * 520.0) as u32, self.seed);
                if st > 0.9975 {
                    lab[0] += 0.35 * (st - 0.9975) / 0.0025 * (1.0 - (y * 1.4).min(1.0));
                }
                lab
            }
            "topography" => {
                let hf = |x: f32, y: f32| n.fbm(x * 0.9 + 0.35 * c, y * 0.9 + 0.35 * s, 3.0, self.detail) * 0.5 + 0.5;
                let lines = 26.0;
                let hgt = hf(x, y);
                let v = hgt * lines;
                // Anti-aliased lines: distance to the nearest contour, in pixels.
                let e = px * k;
                let gx = (hf(x + e, y) - hgt) * lines;
                let gy = (hf(x, y + e) - hgt) * lines;
                let per_px = (gx * gx + gy * gy).sqrt().max(1e-5);
                let dist_px = (v - v.round()).abs() / per_px;
                let major = (v.round() as i32).rem_euclid(5) == 0;
                let hw = if major { 1.2 } else { 0.7 } * self.line_px;
                let line = 1.0 - smooth(hw - 0.6, hw + 0.6, dist_px);
                let bg = self.ramp(0.0);
                let fg = self.ramp(1.0);
                let a = line * if major { 1.0 } else { 0.6 };
                [bg[0] * (1.0 - a) + fg[0] * a, bg[1] * (1.0 - a) + fg[1] * a, bg[2] * (1.0 - a) + fg[2] * a]
            }
            "plasma" => {
                let v = ((x * 3.0 + tau * t).sin() + (y * 4.0 - tau * t).sin() + ((x + y) * 3.0 + n.get(x, y, 0.5 * c) * 2.0).sin() + ((x * x + y * y).sqrt() * 5.0 - tau * t).sin()) / 4.0;
                self.ramp(0.5 + 0.5 * v)
            }
            "dunes" => {
                // Sharp-crested ridges lit from above: bright windward, dark lee sides.
                let hf = |x: f32, y: f32| {
                    let base = n.fbm(x * 0.5 + 0.3 * c, y * 0.5 + 0.3 * s, 5.0, 3);
                    let p = (y * 2.6 + base * 1.8 + x * 0.35).fract();
                    p.powf(1.6) * 0.8 + base * 0.2
                };
                let e = 0.003;
                let h0 = hf(x, y);
                let dy = (hf(x, y + e) - h0) / e;
                let shade = (0.5 + 0.08 * dy.clamp(-6.0, 6.0)).clamp(0.0, 1.0);
                let mut lab = self.ramp((0.15 + 0.7 * shade).clamp(0.0, 1.0));
                lab[0] *= 0.82 + 0.3 * shade;
                lab
            }
            _ => {
                // noise
                let v = n.fbm(x * 1.2 + 0.5 * c, y * 1.2 + 0.5 * s, 6.0, self.detail) * 0.5 + 0.5;
                self.ramp(smooth(0.15, 0.85, v))
            }
        }
    }

    /// Render a frame. `px_scale` keeps grain the same size between preview and output.
    pub fn render(&self, w: u32, h: u32, t: f32, px_scale: f32) -> image::RgbImage {
        // Contours keep the same look at every size: about 1 px per 1080 px of output.
        let line_px = (w.min(h) as f32 * px_scale / 1080.0).clamp(0.8, 3.0) / px_scale.max(1.0);
        let this = Shader { line_px: line_px.max(0.7), ..self.clone_cfg() };
        this.render_inner(w, h, t, px_scale)
    }

    fn clone_cfg(&self) -> Shader {
        Shader { preset: self.preset.clone(), stops: self.stops.clone(), noise: self.noise.clone(), scale: self.scale, detail: self.detail, grain: self.grain, seed: self.seed, line_px: self.line_px }
    }

    fn render_inner(&self, w: u32, h: u32, t: f32, px_scale: f32) -> image::RgbImage {
        let short = w.min(h) as f32;
        let mut buf = vec![0u8; (w * h * 3) as usize];
        buf.par_chunks_mut((w * 3) as usize).enumerate().for_each(|(y, row)| {
            for x in 0..w as usize {
                let (fx, fy) = ((x as f32 + 0.5) / short, (y as f32 + 0.5) / short);
                let lab = self.shade(fx, fy, t, 1.0 / short);
                let mut lin = style::oklab_to_linear(lab);
                let gx = (x as f32 * px_scale) as u32;
                let gy = (y as f32 * px_scale) as u32;
                let gr = hash2(gx, gy, self.seed.wrapping_add((t * 997.0) as u32)) - 0.5;
                let g = self.grain * 0.07 * gr + gr / 255.0;
                for c in lin.iter_mut() {
                    *c = (*c + g * (0.15 + *c)).max(0.0);
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
    fn every_preset_renders() {
        let t = ShaderTool::new();
        for p in PRESETS {
            let mut v = Values::defaults(&t.options());
            v.set("preset", castor_core::Value::Text(p.to_string()));
            let img = Shader::new(&v).render(48, 27, 0.25, 1.0);
            let (mn, mx) = img.as_raw().iter().fold((255u8, 0u8), |a, b| (a.0.min(*b), a.1.max(*b)));
            assert!(mx > mn + 10, "{p} is flat");
        }
    }
}
