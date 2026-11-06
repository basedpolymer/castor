//! `create.placeholder`: placeholder images of any size (or a whole batch of sizes) with the
//! dimensions written on them, a label, colours and a light pattern.

use anyhow::Result;
use castor_core::options::{Choice, Cond, OptionSpec, Values};
use castor_core::{FileInfo, InputSpec, Job, Output, Preview, RunCx, Tool, ToolMeta, tr};

use crate::common::{self, xml};
use crate::render::{png, style};

pub struct PlaceholderTool {
    meta: ToolMeta,
}

impl PlaceholderTool {
    pub fn new() -> Self {
        PlaceholderTool {
            meta: common::meta(
                "create.placeholder",
                InputSpec::none(),
                "PNG · JPG · WEBP · SVG",
                &["placeholder", "dummy", "mock", "image", "size", "banner", "wireframe"],
                "placeholder",
            ),
        }
    }
}

const PRESETS: &[(&str, &str)] = &[
    ("custom", ""),
    ("social", "1200x630, 1080x1080, 1080x1350, 1080x1920, 1500x500, 1584x396"),
    ("web", "1920x1080, 1440x900, 1200x800, 800x600, 600x400, 300x250, 728x90, 160x600"),
    ("devices", "1290x2796, 1179x2556, 1080x2400, 2048x2732, 1668x2388, 2560x1600"),
    ("thumbs", "128x128, 256x256, 400x300, 640x360"),
];

impl Tool for PlaceholderTool {
    fn meta(&self) -> &ToolMeta {
        &self.meta
    }

    fn options(&self) -> Vec<OptionSpec> {
        vec![
            OptionSpec::choice("preset", PRESETS.iter().map(|(k, _)| Choice::key(k)).collect(), "custom"),
            OptionSpec::text("sizes", "1200x630").when(Cond::Eq("preset", "custom")),
            OptionSpec::text("label", ""),
            OptionSpec::toggle("show_size", true),
            OptionSpec::choice("pattern", ["none", "cross", "grid", "diamonds", "stripes"].map(Choice::key).to_vec(), "cross"),
            OptionSpec::color("background", "#f4f2ee"),
            OptionSpec::color("color", "#5c5a55"),
            OptionSpec::choice(
                "format",
                vec![Choice::lit("png", "PNG"), Choice::lit("jpg", "JPG"), Choice::lit("webp", "WebP"), Choice::lit("svg", "SVG")],
                "png",
            ),
        ]
    }

    fn preview(&self, _input: Option<&FileInfo>, v: &Values, max_side: u32) -> Result<Option<Preview>> {
        let sizes = sizes(v)?;
        let (w, h) = sizes[0];
        let svg = svg(w, h, v);
        let pm = png::render_svg_fit(svg.as_bytes(), max_side.min(w.max(h)), false)?;
        let cap = if sizes.len() > 1 {
            castor_core::tr_args("create.placeholder.n_sizes", &[("first", &common::px(w, h)), ("count", &sizes.len().to_string())])
        } else {
            common::px(w, h)
        };
        Ok(Some(common::preview(png::encode_pixmap(&pm)?, common::figure(&cap))))
    }

    fn run(&self, _job: &Job, v: &Values, cx: &RunCx) -> Result<Vec<Output>> {
        let all = sizes(v)?;
        let fmt = v.str("format");
        let mut outs = Vec::new();
        for (i, (w, h)) in all.iter().copied().enumerate() {
            cx.check_cancel()?;
            let s = svg(w, h, v);
            let label = common::slug(v.str("label"));
            let stem = if v.str("label").trim().is_empty() { format!("placeholder-{w}x{h}") } else { format!("{label}-{w}x{h}") };
            let out = cx.output_named(&format!("{stem}.{fmt}"));
            if fmt == "svg" {
                let tree = png::parse_svg(s.as_bytes(), false)?;
                common::write(&out, tree.to_string(&resvg::usvg::WriteOptions::default()).as_bytes())?;
            } else {
                let pm = png::render_svg(s.as_bytes(), w, h, false)?;
                let rgb = png::flatten(&png::pixmap_to_rgba(&pm), [255, 255, 255]);
                crate::gradient::save_rgb(&rgb, &out, fmt)?;
            }
            outs.push(Output::new(out).with_note(common::px(w, h)));
            cx.progress((i + 1) as f32 / all.len() as f32);
        }
        Ok(outs)
    }
}

/// Parse "1200x630, 800 × 600" (or the preset list).
pub fn sizes(v: &Values) -> Result<Vec<(u32, u32)>> {
    let p = v.str("preset");
    let list = PRESETS.iter().find(|(k, _)| *k == p).map(|(_, s)| *s).filter(|s| !s.is_empty()).unwrap_or(v.str("sizes"));
    let mut out = Vec::new();
    for part in list.split([',', ';', '\n']).map(str::trim).filter(|s| !s.is_empty()) {
        let (a, b) = part
            .split_once(['x', 'X', '×', '*'])
            .ok_or_else(|| anyhow::anyhow!("{}", castor_core::tr_args("create.placeholder.err.size", &[("value", part)])))?;
        let (w, h): (u32, u32) = (
            a.trim().parse().map_err(|_| anyhow::anyhow!("{}", castor_core::tr_args("create.placeholder.err.size", &[("value", part)])))?,
            b.trim().parse().map_err(|_| anyhow::anyhow!("{}", castor_core::tr_args("create.placeholder.err.size", &[("value", part)])))?,
        );
        anyhow::ensure!((1..=16384).contains(&w) && (1..=16384).contains(&h), "{}", tr("create.common.err.size"));
        out.push((w, h));
    }
    anyhow::ensure!(!out.is_empty(), "{}", tr("create.placeholder.err.none"));
    Ok(out)
}

pub fn svg(w: u32, h: u32, v: &Values) -> String {
    let bg = v.color("background");
    let fg = v.color("color");
    let (bgc, bga) = style::svg_paint(bg);
    let (fgc, _) = style::svg_paint(fg);
    let (wf, hf) = (w as f32, h as f32);
    let m = wf.min(hf);
    let sw = (m / 400.0).clamp(0.6, 3.0);
    let mut o = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 {w} {h}\" width=\"{w}\" height=\"{h}\"><rect width=\"{w}\" height=\"{h}\" fill=\"{bgc}\" fill-opacity=\"{bga}\"/>"
    );
    let pat = |o: &mut String, inner: String, size: f32| {
        o.push_str(&format!(
            "<defs><pattern id=\"p\" width=\"{size}\" height=\"{size}\" patternUnits=\"userSpaceOnUse\">{inner}</pattern></defs><rect width=\"{w}\" height=\"{h}\" fill=\"url(#p)\"/>"
        ));
    };
    let stroke = format!("stroke=\"{fgc}\" stroke-opacity=\"0.22\" stroke-width=\"{sw}\" fill=\"none\"");
    match v.str("pattern") {
        "cross" => {
            o.push_str(&format!("<path d=\"M0 0L{w} {h}M{w} 0L0 {h}\" {stroke}/>"));
            o.push_str(&format!("<rect x=\"{h2}\" y=\"{h2}\" width=\"{}\" height=\"{}\" {stroke}/>", wf - sw, hf - sw, h2 = sw / 2.0));
        }
        "grid" => {
            let s = (m / 8.0).max(8.0);
            pat(&mut o, format!("<path d=\"M{s} 0V{s}H0\" {stroke}/>"), s);
        }
        "diamonds" => {
            let s = (m / 7.0).max(10.0);
            let r = s * 0.16;
            let c = s / 2.0;
            pat(&mut o, format!("<path d=\"M{c} {}L{} {c}L{c} {}L{} {c}Z\" {stroke}/>", c - r, c + r, c + r, c - r), s);
        }
        "stripes" => {
            let s = (m / 14.0).max(6.0);
            pat(&mut o, format!("<path d=\"M0 {s}L{s} 0\" {stroke}/>"), s);
        }
        _ => {}
    }
    let label = v.str("label").trim();
    let dims = if v.bool("show_size") { format!("{w} × {h}") } else { String::new() };
    // Text sized to the canvas, on a small plate of the background colour so it reads over lines.
    let fs = (wf / (dims.chars().count().max(label.chars().count()).max(6) as f32 * 0.62)).min(hf * 0.22).min(m * 0.3).max(6.0);
    let lfs = (fs * 0.55).max(5.0);
    let (cy, gap) = (hf / 2.0, if !label.is_empty() && !dims.is_empty() { fs * 0.45 } else { 0.0 });
    if !dims.is_empty() || !label.is_empty() {
        let tw = (dims.chars().count() as f32 * fs * 0.55).max(label.chars().count() as f32 * lfs * 0.5) + fs;
        let th = (if dims.is_empty() { 0.0 } else { fs }) + (if label.is_empty() { 0.0 } else { lfs * 1.4 }) + fs * 0.5;
        o.push_str(&format!(
            "<rect x=\"{}\" y=\"{}\" width=\"{tw}\" height=\"{th}\" rx=\"{}\" fill=\"{bgc}\"/>",
            wf / 2.0 - tw / 2.0,
            cy - th / 2.0,
            fs * 0.12
        ));
    }
    if !label.is_empty() {
        o.push_str(&format!(
            "<text x=\"{}\" y=\"{}\" font-family=\"Latin Modern Sans\" font-size=\"{lfs}\" letter-spacing=\"{}\" text-anchor=\"middle\" fill=\"{fgc}\">{}</text>",
            wf / 2.0,
            cy - gap + if dims.is_empty() { lfs * 0.35 } else { -fs * 0.15 },
            lfs * 0.06,
            xml(&label.to_uppercase())
        ));
    }
    if !dims.is_empty() {
        o.push_str(&format!(
            "<text x=\"{}\" y=\"{}\" font-family=\"Latin Modern Sans\" font-weight=\"bold\" font-size=\"{fs}\" text-anchor=\"middle\" fill=\"{fgc}\">{}</text>",
            wf / 2.0,
            cy + fs * 0.35 + gap,
            xml(&dims)
        ));
    }
    o.push_str("</svg>");
    o
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_sizes() {
        let t = PlaceholderTool::new();
        let mut v = Values::defaults(&t.options());
        v.set("sizes", castor_core::Value::Text("1200x630, 800 × 600".into()));
        assert_eq!(sizes(&v).unwrap(), vec![(1200, 630), (800, 600)]);
        v.set("preset", castor_core::Value::Text("social".into()));
        assert_eq!(sizes(&v).unwrap().len(), 6);
    }
}
