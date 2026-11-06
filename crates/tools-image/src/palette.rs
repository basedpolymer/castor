//! image.palette — dominant colours by k-means in CIE Lab; swatches PNG + ASE / GPL / CSS / JSON.

use anyhow::Result;
use castor_core::options::{Choice, OptionSpec, Values};
use castor_core::{FileInfo, Job, Output, Preview, RunCx, Tool, ToolMeta, ToolStatus};
use image::{Rgba, RgbaImage};

use crate::codec::DecodeOpts;
use crate::{common, ops};

pub struct Palette {
    meta: ToolMeta,
}

impl Palette {
    pub fn new() -> Self {
        Palette {
            meta: common::meta(
                "image.palette",
                "PNG · ASE · GPL · CSS · JSON",
                &["palette", "colors", "colours", "swatches", "dominant", "hex", "theme", "extract"],
                "palette",
                ToolStatus::Ready,
                &[],
                false,
            ),
        }
    }
}

impl Tool for Palette {
    fn meta(&self) -> &ToolMeta {
        &self.meta
    }

    fn options(&self) -> Vec<OptionSpec> {
        vec![
            OptionSpec::int("count", 2, 16, 6).slider(),
            OptionSpec::choice(
                "export",
                vec![
                    Choice::key("all"),
                    Choice::lit("png", "PNG"),
                    Choice::lit("ase", "ASE"),
                    Choice::lit("gpl", "GPL"),
                    Choice::lit("css", "CSS"),
                    Choice::lit("json", "JSON"),
                ],
                "all",
            ),
            OptionSpec::choice("order", vec![Choice::key("share"), Choice::key("hue"), Choice::key("lightness")], "share"),
        ]
    }

    fn run(&self, job: &Job, v: &Values, cx: &RunCx) -> Result<Vec<Output>> {
        let input = job.input()?;
        let d = common::load(input, DecodeOpts::default())?;
        cx.progress(0.2);
        let pal = extract(&d.image.to_rgba8(), v.int("count").clamp(2, 32) as usize, v.str("order"));
        cx.progress(0.7);
        let stem = input.stem();
        let mut outs = Vec::new();
        let want = |k: &str| v.str("export") == "all" || v.str("export") == k;
        let mut write = |ext: &str, bytes: Vec<u8>| -> Result<()> {
            let p = cx.output_named(&format!("{stem}-palette.{ext}"));
            common::write(&p, &bytes)?;
            outs.push(Output::new(p));
            Ok(())
        };
        if want("png") {
            let img = swatches(&pal, 1200);
            let mut b = std::io::Cursor::new(Vec::new());
            img.write_to(&mut b, image::ImageFormat::Png)?;
            write("png", b.into_inner())?;
        }
        if want("ase") {
            write("ase", ase(&pal, &stem))?;
        }
        if want("gpl") {
            write("gpl", gpl(&pal, &stem).into_bytes())?;
        }
        if want("css") {
            write("css", css(&pal).into_bytes())?;
        }
        if want("json") {
            write("json", json(&pal).into_bytes())?;
        }
        if let Some(first) = outs.first_mut() {
            first.note = Some(pal.iter().map(|c| hex(c.rgb)).collect::<Vec<_>>().join(" "));
        }
        Ok(outs)
    }

    fn preview(&self, input: Option<&FileInfo>, v: &Values, max_side: u32) -> Result<Option<Preview>> {
        let Some(f) = input else { return Ok(None) };
        let img = common::load_preview(f, 256)?.to_rgba8();
        let pal = extract(&img, v.int("count").clamp(2, 32) as usize, v.str("order"));
        let caption = pal.iter().map(|c| hex(c.rgb)).collect::<Vec<_>>().join("  ");
        common::preview(&swatches(&pal, max_side.min(1200)), max_side, Some(caption))
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Swatch {
    pub rgb: [u8; 3],
    /// Share of the pixels (0..1).
    pub share: f32,
}

pub fn hex(c: [u8; 3]) -> String {
    format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2])
}

fn srgb_to_lin(v: f32) -> f32 {
    let v = v / 255.0;
    if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
}
fn lin_to_srgb(v: f32) -> f32 {
    let v = if v <= 0.003_130_8 { v * 12.92 } else { 1.055 * v.powf(1.0 / 2.4) - 0.055 };
    (v * 255.0).clamp(0.0, 255.0)
}

pub fn rgb_to_lab(c: [u8; 3]) -> [f32; 3] {
    let (r, g, b) = (srgb_to_lin(c[0] as f32), srgb_to_lin(c[1] as f32), srgb_to_lin(c[2] as f32));
    let x = (0.4124 * r + 0.3576 * g + 0.1805 * b) / 0.95047;
    let y = 0.2126 * r + 0.7152 * g + 0.0722 * b;
    let z = (0.0193 * r + 0.1192 * g + 0.9505 * b) / 1.08883;
    let f = |t: f32| if t > 0.008_856 { t.cbrt() } else { 7.787 * t + 16.0 / 116.0 };
    let (fx, fy, fz) = (f(x), f(y), f(z));
    [116.0 * fy - 16.0, 500.0 * (fx - fy), 200.0 * (fy - fz)]
}

pub fn lab_to_rgb(l: [f32; 3]) -> [u8; 3] {
    let fy = (l[0] + 16.0) / 116.0;
    let fx = fy + l[1] / 500.0;
    let fz = fy - l[2] / 200.0;
    let inv = |t: f32| if t.powi(3) > 0.008_856 { t.powi(3) } else { (t - 16.0 / 116.0) / 7.787 };
    let (x, y, z) = (inv(fx) * 0.95047, inv(fy), inv(fz) * 1.08883);
    let r = 3.2406 * x - 1.5372 * y - 0.4986 * z;
    let g = -0.9689 * x + 1.8758 * y + 0.0415 * z;
    let b = 0.0557 * x - 0.2040 * y + 1.0570 * z;
    [lin_to_srgb(r).round() as u8, lin_to_srgb(g).round() as u8, lin_to_srgb(b).round() as u8]
}

/// k-means++ in Lab on a ≤ 200 × 200 sample, transparent pixels ignored.
pub fn extract(img: &RgbaImage, k: usize, order: &str) -> Vec<Swatch> {
    let small = if img.width().max(img.height()) > 200 {
        let (w, h) = ops::fit_size(img.width(), img.height(), 200, 200);
        ops::resize_rgba(img, w, h)
    } else {
        img.clone()
    };
    let pts: Vec<[f32; 3]> = small.pixels().filter(|p| p[3] > 128).map(|p| rgb_to_lab([p[0], p[1], p[2]])).collect();
    if pts.is_empty() {
        return vec![Swatch { rgb: [0, 0, 0], share: 1.0 }];
    }
    let k = k.min(pts.len()).max(1);
    let d2 = |a: &[f32; 3], b: &[f32; 3]| (a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2);
    // k-means++ seeding (deterministic: farthest-point weighted by a fixed sequence).
    let mut cent: Vec<[f32; 3]> = vec![pts[pts.len() / 2]];
    let mut dist: Vec<f32> = pts.iter().map(|p| d2(p, &cent[0])).collect();
    let mut seed = 0x9e37_79b9u32;
    while cent.len() < k {
        let total: f32 = dist.iter().sum();
        if total <= 0.0 {
            break;
        }
        seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        let mut r = (seed as f32 / u32::MAX as f32) * total;
        let mut idx = 0;
        for (i, d) in dist.iter().enumerate() {
            r -= d;
            if r <= 0.0 {
                idx = i;
                break;
            }
        }
        cent.push(pts[idx]);
        for (d, p) in dist.iter_mut().zip(&pts) {
            *d = d.min(d2(p, cent.last().unwrap()));
        }
    }
    let mut assign = vec![0usize; pts.len()];
    for _ in 0..30 {
        let mut changed = false;
        for (i, p) in pts.iter().enumerate() {
            let best = (0..cent.len()).min_by(|a, b| d2(p, &cent[*a]).total_cmp(&d2(p, &cent[*b]))).unwrap_or(0);
            if assign[i] != best {
                assign[i] = best;
                changed = true;
            }
        }
        let mut sums = vec![[0f32; 4]; cent.len()];
        for (p, a) in pts.iter().zip(&assign) {
            for c in 0..3 {
                sums[*a][c] += p[c];
            }
            sums[*a][3] += 1.0;
        }
        for (c, s) in cent.iter_mut().zip(&sums) {
            if s[3] > 0.0 {
                *c = [s[0] / s[3], s[1] / s[3], s[2] / s[3]];
            }
        }
        if !changed {
            break;
        }
    }
    let mut counts = vec![0usize; cent.len()];
    for a in &assign {
        counts[*a] += 1;
    }
    let mut sw: Vec<(Swatch, [f32; 3])> = cent
        .iter()
        .zip(&counts)
        .filter(|(_, n)| **n > 0)
        .map(|(c, n)| (Swatch { rgb: lab_to_rgb(*c), share: *n as f32 / pts.len() as f32 }, *c))
        .collect();
    match order {
        "hue" => sw.sort_by(|a, b| a.1[2].atan2(a.1[1]).total_cmp(&b.1[2].atan2(b.1[1]))),
        "lightness" => sw.sort_by(|a, b| b.1[0].total_cmp(&a.1[0])),
        _ => sw.sort_by(|a, b| b.0.share.total_cmp(&a.0.share)),
    }
    sw.into_iter().map(|s| s.0).collect()
}

/// Swatch strip with hex labels.
pub fn swatches(pal: &[Swatch], width: u32) -> RgbaImage {
    let n = pal.len().max(1) as u32;
    let h = (width / n).clamp(80, 260) + 48;
    let mut img = RgbaImage::from_pixel(width, h, Rgba([255, 255, 255, 255]));
    let sw = width as f32 / n as f32;
    for (i, c) in pal.iter().enumerate() {
        let x = i as f32 * sw;
        ops::fill_rounded_rect(&mut img, x + 4.0, 4.0, sw - 8.0, (h - 52) as f32, 10.0, [c.rgb[0], c.rgb[1], c.rgb[2], 255]);
        if let Some(t) = ops::render_text(&hex(c.rgb), "monospace", 20.0, [40, 40, 40, 255], false) {
            let tx = (x + sw / 2.0 - t.width() as f32 / 2.0) as i64;
            ops::overlay(&mut img, &t, tx, (h - 38) as i64, 1.0);
        }
    }
    img
}

/// Adobe Swatch Exchange (big-endian, RGB floats, global colours).
pub fn ase(pal: &[Swatch], name: &str) -> Vec<u8> {
    let mut b = b"ASEF".to_vec();
    b.extend_from_slice(&1u16.to_be_bytes());
    b.extend_from_slice(&0u16.to_be_bytes());
    b.extend_from_slice(&((pal.len() + 2) as u32).to_be_bytes());
    let utf16 = |s: &str| -> Vec<u8> {
        let mut v: Vec<u16> = s.encode_utf16().collect();
        v.push(0);
        let mut out = (v.len() as u16).to_be_bytes().to_vec();
        for c in v {
            out.extend_from_slice(&c.to_be_bytes());
        }
        out
    };
    // Group start
    let gname = utf16(name);
    b.extend_from_slice(&0xC001u16.to_be_bytes());
    b.extend_from_slice(&(gname.len() as u32).to_be_bytes());
    b.extend_from_slice(&gname);
    for c in pal {
        let mut body = utf16(&hex(c.rgb));
        body.extend_from_slice(b"RGB ");
        for v in c.rgb {
            body.extend_from_slice(&(v as f32 / 255.0).to_be_bytes());
        }
        body.extend_from_slice(&0u16.to_be_bytes()); // global
        b.extend_from_slice(&0x0001u16.to_be_bytes());
        b.extend_from_slice(&(body.len() as u32).to_be_bytes());
        b.extend_from_slice(&body);
    }
    b.extend_from_slice(&0xC002u16.to_be_bytes());
    b.extend_from_slice(&0u32.to_be_bytes());
    b
}

pub fn gpl(pal: &[Swatch], name: &str) -> String {
    let mut s = format!("GIMP Palette\nName: {name}\nColumns: {}\n#\n", pal.len());
    for c in pal {
        s += &format!("{:3} {:3} {:3}\t{}\n", c.rgb[0], c.rgb[1], c.rgb[2], hex(c.rgb));
    }
    s
}

pub fn css(pal: &[Swatch]) -> String {
    let mut s = String::from(":root {\n");
    for (i, c) in pal.iter().enumerate() {
        s += &format!("  --color-{}: {};\n", i + 1, hex(c.rgb));
    }
    s + "}\n"
}

pub fn json(pal: &[Swatch]) -> String {
    let v: Vec<_> = pal
        .iter()
        .map(|c| serde_json::json!({"hex": hex(c.rgb), "rgb": c.rgb, "share": (c.share * 1000.0).round() / 1000.0}))
        .collect();
    serde_json::to_string_pretty(&serde_json::json!({ "colors": v })).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn lab_roundtrip() {
        for c in [[255u8, 0, 0], [12, 200, 90], [128, 128, 128], [0, 0, 0], [255, 255, 255]] {
            let r = lab_to_rgb(rgb_to_lab(c));
            for i in 0..3 {
                assert!((r[i] as i32 - c[i] as i32).abs() <= 1, "{c:?} → {r:?}");
            }
        }
    }
    #[test]
    fn two_colours() {
        let img = RgbaImage::from_fn(100, 100, |x, _| if x < 70 { Rgba([255, 0, 0, 255]) } else { Rgba([0, 0, 255, 255]) });
        let p = extract(&img, 2, "share");
        assert_eq!(p.len(), 2);
        assert!(p[0].rgb[0] > 240 && p[0].share > 0.6);
        assert!(p[1].rgb[2] > 240);
    }
}
