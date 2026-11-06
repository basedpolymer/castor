//! `create.favicon`: a complete favicon pack (ICO, SVG, PNGs, Apple touch icon, Android/PWA
//! icons, web manifest and the HTML snippet) from a letter, a word, an emoji or a logo image.

use anyhow::{Result, anyhow};
use castor_core::options::{Choice, Cond, OptionSpec, Values};
use castor_core::{FileInfo, InputSpec, Job, Output, Preview, RunCx, Tool, ToolMeta, tr};

use crate::common::{self, xml};
use crate::render::{png, style};

pub struct FaviconTool {
    meta: ToolMeta,
}

impl FaviconTool {
    pub fn new() -> Self {
        FaviconTool {
            meta: common::meta(
                "create.favicon",
                InputSpec::kinds(common::IMAGES).min(0).max(1),
                "ICO · SVG · PNG",
                &["favicon", "icon", "ico", "website", "pwa", "apple-touch-icon", "manifest", "emoji", "letter"],
                "favicon",
            ),
        }
    }
}

impl Tool for FaviconTool {
    fn meta(&self) -> &ToolMeta {
        &self.meta
    }

    fn options(&self) -> Vec<OptionSpec> {
        vec![
            OptionSpec::text("text", "C"),
            OptionSpec::choice("font", ["sans", "serif", "chorus", "mono", "system"].map(Choice::key).to_vec(), "serif"),
            OptionSpec::font("family", "").when(Cond::Eq("font", "system")),
            OptionSpec::choice("shape", ["square", "rounded", "circle", "squircle", "diamond", "none"].map(Choice::key).to_vec(), "rounded"),
            OptionSpec::color("background", "#1b1616"),
            OptionSpec::color("background2", "").advanced(),
            OptionSpec::color("color", "#fcfbf9"),
            OptionSpec::int("scale", 30, 100, 68).slider().unit("%"),
            OptionSpec::text("app_name", "").advanced(),
        ]
    }

    fn preview(&self, input: Option<&FileInfo>, v: &Values, max_side: u32) -> Result<Option<Preview>> {
        let svg = icon_svg(input, v)?;
        // Show the master icon and the small sizes side by side (how it looks in a tab).
        let big = max_side.clamp(64, 512) * 2 / 3;
        let mut canvas = image::RgbaImage::from_pixel(big + big / 2, big, image::Rgba([0xfc, 0xfb, 0xf9, 255]));
        let master = png::pixmap_to_rgba(&png::render_svg(svg.as_bytes(), big * 3 / 4, big * 3 / 4, true)?);
        image::imageops::overlay(&mut canvas, &master, (big / 8) as i64, (big / 8) as i64);
        let mut y = big / 8;
        for s in [48u32, 32, 16] {
            let small = png::pixmap_to_rgba(&png::render_svg(svg.as_bytes(), s, s, true)?);
            image::imageops::overlay(&mut canvas, &small, (big + big / 8) as i64, y as i64);
            y += s + 12;
        }
        Ok(Some(common::preview(png::encode_rgba(&canvas)?, common::figure(&tr("create.favicon.caption")))))
    }

    fn run(&self, job: &Job, v: &Values, cx: &RunCx) -> Result<Vec<Output>> {
        let svg = icon_svg(job.inputs.first(), v)?;
        let dir = cx.output_named("favicon");
        std::fs::create_dir_all(&dir)?;
        let mut outs = Vec::new();
        let render = |s: u32| -> Result<Vec<u8>> { png::encode_pixmap(&png::render_svg(svg.as_bytes(), s, s, true)?) };
        // Vector master (text converted to paths so it renders the same in every browser).
        let tree = png::parse_svg(svg.as_bytes(), true)?;
        let p = dir.join("favicon.svg");
        common::write(&p, tree.to_string(&resvg::usvg::WriteOptions::default()).as_bytes())?;
        outs.push(Output::new(p));
        let ico: Vec<(u32, Vec<u8>)> = [16, 32, 48].into_iter().map(|s| render(s).map(|d| (s, d))).collect::<Result<_>>()?;
        let p = dir.join("favicon.ico");
        common::write(&p, &ico_bytes(&ico))?;
        outs.push(Output::new(p).with_note("16 · 32 · 48"));
        for (name, s) in [("favicon-16x16.png", 16), ("favicon-32x32.png", 32), ("android-chrome-192x192.png", 192), ("android-chrome-512x512.png", 512)] {
            cx.check_cancel()?;
            let p = dir.join(name);
            common::write(&p, &render(s)?)?;
            outs.push(Output::new(p));
        }
        // Apple touch icon: opaque, full-bleed (iOS rounds the corners itself).
        let opaque = icon_svg_with(job.inputs.first(), v, Some("square"))?;
        let p = dir.join("apple-touch-icon.png");
        let pm = png::render_svg(opaque.as_bytes(), 180, 180, true)?;
        let bg = v.color("background");
        let rgb = png::flatten(&png::pixmap_to_rgba(&pm), [bg[0], bg[1], bg[2]]);
        common::write(&p, &png::encode_rgb(&rgb)?)?;
        outs.push(Output::new(p));
        // Maskable PWA icon: content inside the 80 % safe zone.
        let p = dir.join("maskable-512x512.png");
        let mut vv = v.clone();
        vv.set("scale", castor_core::Value::Int((v.int("scale") as f32 * 0.8) as i64));
        let mask = icon_svg_with(job.inputs.first(), &vv, Some("square"))?;
        common::write(&p, &png::encode_pixmap(&png::render_svg(mask.as_bytes(), 512, 512, true)?)?)?;
        outs.push(Output::new(p));
        let name = if v.str("app_name").trim().is_empty() { v.str("text").trim().to_string() } else { v.str("app_name").trim().to_string() };
        let theme = style::hex(v.color("background"));
        let manifest = serde_json::json!({
            "name": name, "short_name": name,
            "icons": [
                {"src": "/android-chrome-192x192.png", "sizes": "192x192", "type": "image/png"},
                {"src": "/android-chrome-512x512.png", "sizes": "512x512", "type": "image/png"},
                {"src": "/maskable-512x512.png", "sizes": "512x512", "type": "image/png", "purpose": "maskable"}
            ],
            "theme_color": theme, "background_color": theme, "display": "standalone"
        });
        let p = dir.join("site.webmanifest");
        common::write(&p, serde_json::to_string_pretty(&manifest)?.as_bytes())?;
        outs.push(Output::new(p));
        let html = format!(
            "<link rel=\"icon\" href=\"/favicon.ico\" sizes=\"48x48\">\n<link rel=\"icon\" href=\"/favicon.svg\" type=\"image/svg+xml\">\n<link rel=\"apple-touch-icon\" href=\"/apple-touch-icon.png\">\n<link rel=\"manifest\" href=\"/site.webmanifest\">\n<meta name=\"theme-color\" content=\"{theme}\">\n"
        );
        let p = dir.join("favicon.html");
        common::write(&p, html.as_bytes())?;
        outs.push(Output::new(p).with_note(tr("create.favicon.note_html")));
        cx.progress(1.0);
        Ok(outs)
    }
}

/// ICO container with PNG-compressed entries (Vista+ / every browser).
pub fn ico_bytes(images: &[(u32, Vec<u8>)]) -> Vec<u8> {
    let mut o = Vec::new();
    o.extend_from_slice(&[0, 0, 1, 0]);
    o.extend_from_slice(&(images.len() as u16).to_le_bytes());
    let mut offset = 6 + 16 * images.len() as u32;
    for (s, data) in images {
        let b = if *s >= 256 { 0 } else { *s as u8 };
        o.extend_from_slice(&[b, b, 0, 0]);
        o.extend_from_slice(&1u16.to_le_bytes());
        o.extend_from_slice(&32u16.to_le_bytes());
        o.extend_from_slice(&(data.len() as u32).to_le_bytes());
        o.extend_from_slice(&offset.to_le_bytes());
        offset += data.len() as u32;
    }
    for (_, data) in images {
        o.extend_from_slice(data);
    }
    o
}

fn icon_svg(input: Option<&FileInfo>, v: &Values) -> Result<String> {
    icon_svg_with(input, v, None)
}

/// Shape path in a 512 × 512 box.
fn shape(kind: &str) -> String {
    match kind {
        "circle" => "<circle cx=\"256\" cy=\"256\" r=\"256\"/>".into(),
        "rounded" => "<rect width=\"512\" height=\"512\" rx=\"96\"/>".into(),
        "squircle" => "<path d=\"M256 0C51 0 0 51 0 256s51 256 256 256 256-51 256-256S461 0 256 0Z\"/>".into(),
        "diamond" => "<path d=\"M256 6L506 256L256 506L6 256Z\"/>".into(),
        "none" => String::new(),
        _ => "<rect width=\"512\" height=\"512\"/>".into(),
    }
}

fn icon_svg_with(input: Option<&FileInfo>, v: &Values, force_shape: Option<&str>) -> Result<String> {
    let kind = force_shape.unwrap_or(v.str("shape"));
    let bg = v.color("background");
    let (bgc, bga) = style::svg_paint(bg);
    let fill = match castor_core::options::parse_hex_color(v.str("background2")) {
        Some(c2) => {
            let (c2c, c2a) = style::svg_paint(c2);
            format!(
                "<defs><linearGradient id=\"bg\" x1=\"0\" y1=\"0\" x2=\"1\" y2=\"1\"><stop offset=\"0\" stop-color=\"{bgc}\" stop-opacity=\"{bga}\"/><stop offset=\"1\" stop-color=\"{c2c}\" stop-opacity=\"{c2a}\"/></linearGradient></defs><g fill=\"url(#bg)\">"
            )
        }
        None => format!("<g fill=\"{bgc}\" fill-opacity=\"{bga}\">"),
    };
    let scale = v.int("scale").clamp(10, 100) as f32 / 100.0;
    // The diamond and circle have less room inside.
    let room = match kind {
        "diamond" => 0.62,
        "circle" | "squircle" => 0.86,
        _ => 0.94,
    } * 512.0
        * scale;
    let mut o = format!("<svg xmlns=\"http://www.w3.org/2000/svg\" xmlns:xlink=\"http://www.w3.org/1999/xlink\" viewBox=\"0 0 512 512\" width=\"512\" height=\"512\">");
    o.push_str(&fill);
    o.push_str(&shape(kind));
    o.push_str("</g>");
    if let Some(f) = input {
        let img = png::load_image(&f.path, 1024)?;
        let data = png::encode_rgba(&img)?;
        let (w, h) = (img.width() as f32, img.height() as f32);
        let k = room / w.max(h);
        let (dw, dh) = (w * k, h * k);
        o.push_str(&format!(
            "<image x=\"{}\" y=\"{}\" width=\"{dw}\" height=\"{dh}\" href=\"data:image/png;base64,{}\"/>",
            256.0 - dw / 2.0,
            256.0 - dh / 2.0,
            common::base64(&data)
        ));
    } else {
        let text = v.str("text").trim();
        anyhow::ensure!(!text.is_empty(), "{}", tr("create.favicon.err.text"));
        let fam = match v.str("font") {
            "sans" => "Latin Modern Sans".to_string(),
            "chorus" => "TeX Gyre Chorus".to_string(),
            "mono" => "Latin Modern Mono".to_string(),
            "system" if !v.str("family").trim().is_empty() => v.str("family").trim().to_string(),
            _ => "Latin Modern Roman".to_string(),
        };
        let weight = if matches!(v.str("font"), "sans" | "serif") { "bold" } else { "normal" };
        let (fc, fa) = style::svg_paint(v.color("color"));
        // Emoji fallbacks after the chosen family.
        let glyph = format!(
            "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"2000\" height=\"1000\" viewBox=\"0 0 2000 1000\"><text x=\"100\" y=\"700\" font-family=\"'{}', 'Segoe UI Emoji', 'Apple Color Emoji', 'Noto Color Emoji', 'Segoe UI Symbol'\" font-weight=\"{weight}\" font-size=\"600\" fill=\"{fc}\" fill-opacity=\"{fa}\">{}</text></svg>",
            xml(&fam),
            xml(text)
        );
        let tree = png::parse_svg(glyph.as_bytes(), true)?;
        let bb = tree.root().abs_bounding_box();
        anyhow::ensure!(bb.width() > 0.0 && bb.height() > 0.0, "{}", tr("create.favicon.err.font"));
        let k = room / bb.width().max(bb.height());
        let (dw, dh) = (bb.width() * k, bb.height() * k);
        let inner = tree.to_string(&resvg::usvg::WriteOptions::default());
        // Re-embed the outlined text, cropped to its ink box and centred.
        let inner = inner.replacen("<svg ", &format!(
            "<svg x=\"{}\" y=\"{}\" width=\"{dw}\" height=\"{dh}\" viewBox=\"{} {} {} {}\" ",
            256.0 - dw / 2.0,
            256.0 - dh / 2.0,
            bb.x(),
            bb.y(),
            bb.width(),
            bb.height()
        ), 1);
        let inner = strip_size_attrs(&inner);
        o.push_str(&inner);
    }
    o.push_str("</svg>");
    // Validate.
    png::parse_svg(o.as_bytes(), true).map_err(|e| anyhow!("{e}"))?;
    Ok(o)
}

/// Remove the first `width="…"`/`height="…"`/`viewBox` that usvg wrote after our own attributes.
fn strip_size_attrs(s: &str) -> String {
    let Some(end) = s.find('>') else { return s.to_string() };
    let (head, rest) = s.split_at(end);
    let mut seen = std::collections::HashSet::new();
    let mut out = String::new();
    // Keep the first occurrence of each attribute name (ours come first).
    let mut i = 0;
    let bytes: Vec<char> = head.chars().collect();
    while i < bytes.len() {
        if bytes[i] == ' ' {
            let start = i + 1;
            let mut j = start;
            while j < bytes.len() && bytes[j] != '=' && bytes[j] != ' ' {
                j += 1;
            }
            let name: String = bytes[start..j].iter().collect();
            if j < bytes.len() && bytes[j] == '=' {
                let mut k = j + 2;
                while k < bytes.len() && bytes[k] != '"' {
                    k += 1;
                }
                let attr: String = bytes[i..=k.min(bytes.len() - 1)].iter().collect();
                if seen.insert(name) {
                    out.push_str(&attr);
                }
                i = k + 1;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ico_header() {
        let ico = ico_bytes(&[(16, vec![1, 2, 3]), (32, vec![4])]);
        assert_eq!(&ico[..6], &[0, 0, 1, 0, 2, 0]);
        assert_eq!(ico.len(), 6 + 32 + 4);
    }
    #[test]
    fn letter_icon() {
        let t = FaviconTool::new();
        let v = Values::defaults(&t.options());
        let s = icon_svg(None, &v).unwrap();
        let pm = png::render_svg(s.as_bytes(), 64, 64, false).unwrap();
        // Centre pixel area contains some light ink of the letter.
        let img = png::pixmap_to_rgba(&pm);
        assert!(img.pixels().any(|p| p[0] > 200 && p[3] == 255));
        assert_eq!(strip_size_attrs("<svg a=\"1\" b=\"2\" a=\"3\">x"), "<svg a=\"1\" b=\"2\">x");
    }
}
