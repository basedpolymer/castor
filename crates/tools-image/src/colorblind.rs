//! image.colorblind — simulate protanopia, deuteranopia, tritanopia (Machado et al. 2009) and
//! achromatopsia; report which dominant colours become hard to tell apart.

use anyhow::Result;
use castor_core::options::{Choice, OptionSpec, Values};
use castor_core::{FileInfo, Job, Output, Preview, RunCx, Tool, ToolMeta, ToolStatus};
use image::{DynamicImage, Rgba, RgbaImage};
use rayon::prelude::*;

use crate::codec::{DecodeOpts, EncodeOpts, Fmt};
use crate::palette::{extract, hex, rgb_to_lab};
use crate::{common, ops};

pub struct Colorblind {
    meta: ToolMeta,
}

pub const KINDS: &[&str] = &["protanopia", "deuteranopia", "tritanopia", "achromatopsia"];

impl Colorblind {
    pub fn new() -> Self {
        Colorblind {
            meta: common::meta(
                "image.colorblind",
                "PNG",
                &["color blind", "colorblind", "daltonism", "accessibility", "protanopia", "deuteranopia", "tritanopia", "a11y", "contrast"],
                "colorblind",
                ToolStatus::Ready,
                &[],
                false,
            ),
        }
    }
}

impl Tool for Colorblind {
    fn meta(&self) -> &ToolMeta {
        &self.meta
    }

    fn options(&self) -> Vec<OptionSpec> {
        let mut kinds = vec![Choice::key("all")];
        kinds.extend(KINDS.iter().map(|k| Choice::key(k)));
        vec![OptionSpec::choice("kind", kinds, "all"), OptionSpec::int("severity", 10, 100, 100).slider().unit("%")]
    }

    fn run(&self, job: &Job, v: &Values, cx: &RunCx) -> Result<Vec<Output>> {
        let input = job.input()?;
        let d = common::load(input, DecodeOpts::default())?;
        let img = d.image.to_rgba8();
        let sev = v.int("severity").clamp(0, 100) as f32 / 100.0;
        cx.progress(0.2);
        let out_img = render(&img, v.str("kind"), sev);
        cx.progress(0.7);
        let report = report(&img, sev);
        let o = EncodeOpts { png_effort: 1, ..Default::default() };
        let out = common::save(&DynamicImage::ImageRgba8(out_img), Fmt::Png, &o, Some(input), cx)?;
        let txt = cx.output_named(&format!("{}-colorblind.txt", input.stem()));
        common::write(&txt, report.0.as_bytes())?;
        Ok(vec![Output::new(out).with_note(report.1), Output::new(txt)])
    }

    fn preview(&self, input: Option<&FileInfo>, v: &Values, max_side: u32) -> Result<Option<Preview>> {
        let Some(f) = input else { return Ok(None) };
        let img = common::load_preview(f, max_side)?.to_rgba8();
        let sev = v.int("severity").clamp(0, 100) as f32 / 100.0;
        common::preview(&render(&img, v.str("kind"), sev), max_side, None)
    }
}

/// Machado 2009 matrices at severity 1.0 (linear RGB).
fn matrix(kind: &str) -> [[f32; 3]; 3] {
    match kind {
        "protanopia" => [[0.152286, 1.052583, -0.204868], [0.114503, 0.786281, 0.099216], [-0.003882, -0.048116, 1.051998]],
        "deuteranopia" => [[0.367322, 0.860646, -0.227968], [0.280085, 0.672501, 0.047413], [-0.011820, 0.042940, 0.968881]],
        "tritanopia" => [[1.255528, -0.076749, -0.178779], [-0.078411, 0.930809, 0.147602], [0.004733, 0.691367, 0.303900]],
        _ => [[0.2126, 0.7152, 0.0722], [0.2126, 0.7152, 0.0722], [0.2126, 0.7152, 0.0722]],
    }
}

fn lut_to_lin() -> [f32; 256] {
    let mut t = [0f32; 256];
    for (i, v) in t.iter_mut().enumerate() {
        let c = i as f32 / 255.0;
        *v = if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) };
    }
    t
}

fn to_srgb(v: f32) -> u8 {
    let v = v.clamp(0.0, 1.0);
    let s = if v <= 0.003_130_8 { v * 12.92 } else { 1.055 * v.powf(1.0 / 2.4) - 0.055 };
    (s * 255.0).round() as u8
}

pub fn simulate_px(c: [u8; 3], kind: &str, sev: f32, lut: &[f32; 256]) -> [u8; 3] {
    let m = matrix(kind);
    let l = [lut[c[0] as usize], lut[c[1] as usize], lut[c[2] as usize]];
    let mut o = [0f32; 3];
    for r in 0..3 {
        let s = m[r][0] * l[0] + m[r][1] * l[1] + m[r][2] * l[2];
        o[r] = l[r] + (s - l[r]) * sev;
    }
    [to_srgb(o[0]), to_srgb(o[1]), to_srgb(o[2])]
}

pub fn simulate(img: &RgbaImage, kind: &str, sev: f32) -> RgbaImage {
    let lut = lut_to_lin();
    let mut out = img.clone();
    out.par_chunks_mut(4).for_each(|p| {
        let s = simulate_px([p[0], p[1], p[2]], kind, sev, &lut);
        p[0] = s[0];
        p[1] = s[1];
        p[2] = s[2];
    });
    out
}

/// One simulation, or a labelled 2 × 2 grid (original + three most common types) for "all".
pub fn render(img: &RgbaImage, kind: &str, sev: f32) -> RgbaImage {
    if kind != "all" {
        return simulate(img, kind, sev);
    }
    let (w, h) = img.dimensions();
    let (cw, ch) = if w.max(h) > 1600 { ops::fit_size(w, h, 1600, 1600) } else { (w, h) };
    let small = ops::resize_rgba(img, cw, ch);
    let gap = (cw.max(ch) / 60).max(6);
    let label_h = (ch / 12).clamp(24, 64);
    let mut out = RgbaImage::from_pixel(cw * 2 + gap * 3, (ch + label_h) * 2 + gap * 3, Rgba([255, 255, 255, 255]));
    let panels = [("original", None), ("protanopia", Some("protanopia")), ("deuteranopia", Some("deuteranopia")), ("tritanopia", Some("tritanopia"))];
    for (i, (label, k)) in panels.iter().enumerate() {
        let im = match k {
            Some(k) => simulate(&small, k, sev),
            None => small.clone(),
        };
        let x = gap + (i as u32 % 2) * (cw + gap);
        let y = gap + (i as u32 / 2) * (ch + label_h + gap);
        ops::overlay(&mut out, &im, x as i64, (y + label_h) as i64, 1.0);
        let text = castor_core::tr(&format!("image.colorblind.opt.kind.{label}"));
        if let Some(t) = ops::render_text(&text, "", label_h as f32 * 0.55, [30, 30, 30, 255], true) {
            ops::overlay(&mut out, &t, x as i64, (y + (label_h - t.height()) / 2) as i64, 1.0);
        }
    }
    out
}

/// (full text report, one-line note). Pairs of dominant colours that are distinct for normal
/// vision (ΔE > 20) but close (ΔE < 10) under a deficiency are flagged.
pub fn report(img: &RgbaImage, sev: f32) -> (String, String) {
    let pal = extract(img, 8, "share");
    let lut = lut_to_lin();
    let de = |a: [u8; 3], b: [u8; 3]| {
        let (x, y) = (rgb_to_lab(a), rgb_to_lab(b));
        ((x[0] - y[0]).powi(2) + (x[1] - y[1]).powi(2) + (x[2] - y[2]).powi(2)).sqrt()
    };
    let mut txt = String::new();
    txt += &format!("{}\n\n", castor_core::tr("image.colorblind.report_title"));
    let mut flagged = 0;
    for kind in KINDS {
        txt += &format!("{}:\n", castor_core::tr(&format!("image.colorblind.opt.kind.{kind}")));
        let mut any = false;
        for i in 0..pal.len() {
            for j in i + 1..pal.len() {
                let (a, b) = (pal[i].rgb, pal[j].rgb);
                if de(a, b) > 20.0 {
                    let (sa, sb) = (simulate_px(a, kind, sev, &lut), simulate_px(b, kind, sev, &lut));
                    let d = de(sa, sb);
                    if d < 10.0 {
                        any = true;
                        flagged += 1;
                        txt += &format!("  {} ↔ {}  (ΔE {:.0} → {:.0})\n", hex(a), hex(b), de(a, b), d);
                    }
                }
            }
        }
        if !any {
            txt += &format!("  {}\n", castor_core::tr("image.colorblind.ok"));
        }
        txt += "\n";
    }
    let note = if flagged == 0 {
        castor_core::tr("image.colorblind.note_ok")
    } else {
        castor_core::tr_args("image.colorblind.note_pairs", &[("count", &flagged.to_string())])
    };
    (txt, note)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn red_green_confused_for_deuteranopia() {
        let lut = lut_to_lin();
        let r = simulate_px([200, 60, 50], "deuteranopia", 1.0, &lut);
        let g = simulate_px([90, 140, 40], "deuteranopia", 1.0, &lut);
        let d = |a: [u8; 3], b: [u8; 3]| {
            let (x, y) = (rgb_to_lab(a), rgb_to_lab(b));
            ((x[0] - y[0]).powi(2) + (x[1] - y[1]).powi(2) + (x[2] - y[2]).powi(2)).sqrt()
        };
        assert!(d(r, g) < d([200, 60, 50], [90, 140, 40]) / 2.0);
    }
}
