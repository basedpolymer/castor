//! `create.social-card`: Open Graph / social images (1200 × 630 and friends) from Typst
//! templates: title, subtitle, eyebrow, footer, logo, optional photo, colours. Give it a CSV and
//! it makes one card per row (columns `title`, `subtitle`, `eyebrow`, `footer`, `file`).

use anyhow::Result;
use castor_core::options::{Choice, Cond, OptionSpec, Values};
use castor_core::{FileInfo, FileKind, InputSpec, Job, Output, Preview, RunCx, Tool, ToolMeta, tr};

use crate::common;
use crate::render::{png, style, typ};

pub struct SocialCardTool {
    meta: ToolMeta,
}

impl SocialCardTool {
    pub fn new() -> Self {
        SocialCardTool {
            meta: common::meta(
                "create.social-card",
                InputSpec::kinds(&[FileKind::Table]).min(0),
                "PNG · JPG · PDF",
                &["og", "open graph", "social", "card", "twitter", "linkedin", "thumbnail", "banner", "share", "image"],
                "social-card",
            ),
        }
    }
}

pub const TEMPLATES: &[&str] = &["classic", "tint", "split", "quote", "photo", "minimal"];
const SIZES: &[(&str, u32, u32)] = &[
    ("og", 1200, 630),
    ("x", 1600, 900),
    ("square", 1080, 1080),
    ("portrait", 1080, 1350),
    ("story", 1080, 1920),
    ("youtube", 1280, 720),
    ("linkedin-banner", 1584, 396),
];

impl Tool for SocialCardTool {
    fn meta(&self) -> &ToolMeta {
        &self.meta
    }

    fn options(&self) -> Vec<OptionSpec> {
        let mut sizes: Vec<Choice> = SIZES.iter().map(|(k, _, _)| Choice::key(k)).collect();
        sizes.push(Choice::key("custom"));
        vec![
            OptionSpec::choice("template", TEMPLATES.iter().map(|t| Choice::key(t)).collect(), "classic"),
            OptionSpec::multiline("title", "Every file tool, right on your desk"),
            OptionSpec::multiline("subtitle", "Free, offline and open source. Nothing leaves your machine."),
            OptionSpec::text("eyebrow", "Castor"),
            OptionSpec::text("footer", "basedpolymer.eu/castor"),
            OptionSpec::file("logo", &["png", "jpg", "jpeg", "webp", "svg"]),
            OptionSpec::file("image", &["png", "jpg", "jpeg", "webp"]).when(Cond::Eq("template", "photo")),
            OptionSpec::choice("size", sizes, "og"),
            OptionSpec::size("custom_size", 1200, 630).when(Cond::Eq("size", "custom")),
            OptionSpec::color("background", "#fcfbf9"),
            OptionSpec::color("color", "#1b1616"),
            OptionSpec::color("accent", "#ffeca4"),
            OptionSpec::choice("font", ["sans", "serif", "chorus", "system"].map(Choice::key).to_vec(), "sans"),
            OptionSpec::font("family", "").when(Cond::Eq("font", "system")),
            OptionSpec::choice("format", vec![Choice::lit("png", "PNG"), Choice::lit("jpg", "JPG"), Choice::lit("pdf", "PDF")], "png"),
            OptionSpec::text("file_name", "").advanced(),
        ]
    }

    fn preview(&self, input: Option<&FileInfo>, v: &Values, max_side: u32) -> Result<Option<Preview>> {
        let row = match input {
            Some(f) if f.kind == FileKind::Table => crate::data_merge::read_table(&f.path)?.1.into_iter().next(),
            _ => None,
        };
        let doc = card(v, row.as_ref())?.compile()?;
        let (w, h) = size(v);
        Ok(Some(common::preview(typ::png_fit(&doc, 0, max_side)?, common::figure(&common::px(w, h)))))
    }

    fn plan(&self, inputs: &[FileInfo], _v: &Values) -> Result<Vec<Job>> {
        Ok(if inputs.is_empty() { vec![Job::empty()] } else { inputs.iter().cloned().map(Job::single).collect() })
    }

    fn run(&self, job: &Job, v: &Values, cx: &RunCx) -> Result<Vec<Output>> {
        let rows: Vec<Option<std::collections::BTreeMap<String, String>>> = match job.inputs.first() {
            Some(f) => {
                let (_, rows) = crate::data_merge::read_table(&f.path)?;
                anyhow::ensure!(!rows.is_empty(), "{}", tr("create.data-merge.err.no_rows"));
                rows.into_iter().map(Some).collect()
            }
            None => vec![None],
        };
        let fmt = v.str("format");
        let (w, h) = size(v);
        let mut outs = Vec::new();
        for (i, row) in rows.iter().enumerate() {
            cx.check_cancel()?;
            let doc = card(v, row.as_ref())?.compile()?;
            let name = row
                .as_ref()
                .and_then(|r| r.get("file").or_else(|| r.get("title")).cloned())
                .unwrap_or_else(|| v.str("file_name").to_string());
            let name = if name.trim().is_empty() { String::new() } else { common::slug(&name) };
            let out = common::out_named(cx, &name, fmt);
            match fmt {
                "pdf" => common::write(&out, &typ::pdf(&doc)?)?,
                "jpg" => {
                    let pm = typ::pixmap(&doc, 0, 1.0)?;
                    let rgb = png::flatten(&png::pixmap_to_rgba(&pm), [255, 255, 255]);
                    crate::gradient::save_rgb(&rgb, &out, "jpg")?;
                }
                _ => common::write(&out, &png::encode_pixmap(&typ::pixmap(&doc, 0, 1.0)?)?)?,
            }
            outs.push(Output::new(out).with_note(common::px(w, h)));
            cx.progress((i + 1) as f32 / rows.len() as f32);
        }
        Ok(outs)
    }
}

fn size(v: &Values) -> (u32, u32) {
    let s = v.str("size");
    if let Some((_, w, h)) = SIZES.iter().find(|(k, _, _)| *k == s) {
        return (*w, *h);
    }
    let (w, h) = v.size("custom_size");
    (w.clamp(100, 8000), h.clamp(100, 8000))
}

/// Build the Typst document of one card. `row` overrides texts (CSV batch).
pub fn card(v: &Values, row: Option<&std::collections::BTreeMap<String, String>>) -> Result<typ::Doc> {
    let (w, h) = size(v);
    let get = |k: &str| row.and_then(|r| r.get(k)).cloned().unwrap_or_else(|| v.str(k).to_string());
    let family = match v.str("font") {
        "serif" => "Latin Modern Roman".to_string(),
        "chorus" => "TeX Gyre Chorus".to_string(),
        "system" if !v.str("family").trim().is_empty() => v.str("family").trim().to_string(),
        _ => "Latin Modern Sans".to_string(),
    };
    let mut doc = typ::Doc::new(String::new()).family(&family);
    let mut has_logo = false;
    if let Some(p) = v.path("logo") {
        let img = png::load_image(p, 1024)?;
        doc = doc.file("/logo.png", png::encode_rgba(&img)?);
        has_logo = true;
    }
    let mut has_image = false;
    if v.str("template") == "photo" {
        let img = match v.path("image") {
            Some(p) => png::load_image(p, 3000)?,
            // No photo: a painted gradient from the accent colour.
            None => {
                let cols = [v.color("accent"), style::LINK, v.color("background"), style::mix_oklab(v.color("accent"), style::INK, 0.5)];
                let m = crate::gradient::Mesh::with(&cols, 6, 9, [0.5, 0.6, 0.3, 0.1]);
                image::DynamicImage::ImageRgb8(m.render((w / 2).max(64), (h / 2).max(64), 0.0)).to_rgba8()
            }
        };
        doc = doc.file("/photo.png", png::encode_rgba(&img)?);
        has_image = true;
    }
    let bg = v.color("background");
    let data = serde_json::json!({
        "w": w, "h": h,
        "title": get("title"), "subtitle": get("subtitle"), "eyebrow": get("eyebrow"), "footer": get("footer"),
        "bg": style::hex(bg), "fg": style::hex(v.color("color")), "accent": style::hex(v.color("accent")),
        "on_accent": style::hex(style::text_on(v.color("accent"))),
        "family": family, "bold": v.str("font") == "sans" || v.str("font") == "serif",
        "logo": has_logo, "image": has_image,
    });
    let body = match v.str("template") {
        "tint" => T_TINT,
        "split" => T_SPLIT,
        "quote" => T_QUOTE,
        "photo" => T_PHOTO,
        "minimal" => T_MINIMAL,
        _ => T_CLASSIC,
    };
    doc.source = format!("{}\n{COMMON}\n{body}", style::TYPST_PRELUDE);
    Ok(doc.json("/data.json", &data))
}

const COMMON: &str = r##"
#let d = json("/data.json")
#let W = d.w * 1pt
#let H = d.h * 1pt
#let u = calc.min(W, H) / 100
#let bg = rgb(d.bg)
#let fg = rgb(d.fg)
#let accent = rgb(d.accent)
#set page(width: W, height: H, margin: 0pt, fill: bg)
#set text(font: serif, fill: fg, size: 3.2 * u, lang: "en", hyphenate: false)
#set par(leading: 0.42em, justify: false)
#let lines(s) = s.split("\n").map(l => [#l]).join(linebreak())
// Largest size (from `max` down to `min`) at which `body` fits in `w` × `h`.
#let fit(body, w, h, max: 9 * u, min: 3 * u, font: none, weight: "regular", style: "normal", lead: 0.12em) = context {
  let f = if font == none { d.family } else { font }
  let s = max
  let mk(s) = block(width: w, text(font: f, size: s, weight: weight, style: style, top-edge: "cap-height", par(leading: lead + 0.1em, body)))
  while s > min and measure(mk(s)).height > h { s = s * 0.94 }
  mk(s)
}
#let wt = if d.bold { "bold" } else { "regular" }
#let logo(h) = if d.logo { image("/logo.png", height: h) }
"##;

const T_CLASSIC: &str = r##"
#place(top + left, dx: 2 * u, dy: 4 * u, text(font: chorus, size: 46 * u, fill: fg.transparentize(91%), d.title.clusters().at(0, default: "")))
#place(top + left, dx: 8 * u, dy: 8 * u, {
  if d.eyebrow != "" { box(baseline: 0.15em, diamond(size: 1.5 * u, stroke: 0.3 * u + fg)); h(1.6 * u); text(font: sans, size: 2.4 * u, tracking: 0.14em, upper(d.eyebrow)) }
})
#place(top + right, dx: -8 * u, dy: 7 * u, logo(7 * u))
#place(top + left, dx: 8 * u, dy: 19 * u, {
  fit(lines(d.title), W - 16 * u, H * 0.46, max: 15 * u, weight: wt)
  v(3 * u)
  block(width: W * 0.75, text(size: 4.2 * u, fill: fg.transparentize(25%), lines(d.subtitle)))
})
#place(bottom + left, dx: 8 * u, dy: -7 * u, box(width: W - 16 * u, {
  text(font: mono, size: 2.3 * u, fill: fg.transparentize(30%), d.footer)
  v(1.5 * u)
  pagerule(stroke: 0.25 * u + fg, size: 1.4 * u)
}))
#place(bottom + left, rect(width: 100%, height: 1.6 * u, fill: accent))
"##;

const T_TINT: &str = r##"
#set page(fill: accent)
#set text(fill: rgb(d.on_accent))
#let ink2 = rgb(d.on_accent)
#for i in range(5) {
  for j in range(3) {
    place(top + right, dx: -6 * u - i * 5 * u, dy: 6 * u + j * 5 * u, diamond(size: 1.3 * u, stroke: 0.22 * u + ink2.transparentize(55%)))
  }
}
#place(left + horizon, dx: 9 * u, box(width: W * 0.72, {
  if d.eyebrow != "" { text(font: sans, size: 2.5 * u, tracking: 0.14em, weight: "bold", upper(d.eyebrow)); v(2 * u) }
  fit(lines(d.title), W * 0.72, H * 0.5, max: 15 * u, weight: wt)
  v(3 * u)
  text(size: 4.2 * u, lines(d.subtitle))
}))
#place(bottom + left, dx: 9 * u, dy: -7 * u, text(font: mono, size: 2.3 * u, d.footer))
#place(bottom + right, dx: -8 * u, dy: -6 * u, logo(8 * u))
"##;

const T_SPLIT: &str = r##"
#place(top + left, rect(width: W * 0.36, height: H, fill: accent))
#place(top + left, dx: 0pt, dy: 0pt, box(width: W * 0.36, height: H, inset: 7 * u, {
  set text(fill: rgb(d.on_accent))
  if d.logo { logo(10 * u); v(4 * u) } else { diamond(size: 4 * u, stroke: 0.5 * u + rgb(d.on_accent)); v(4 * u) }
  text(font: sans, size: 2.6 * u, tracking: 0.12em, weight: "bold", upper(d.eyebrow))
  place(bottom + left, text(font: mono, size: 2.2 * u, d.footer))
}))
#place(top + left, dx: W * 0.36 + 7 * u, dy: 0pt, box(width: W * 0.64 - 14 * u, height: H, align(horizon, {
  fit(lines(d.title), W * 0.64 - 14 * u, H * 0.55, max: 13 * u, weight: wt)
  v(3 * u)
  line(length: 12 * u, stroke: 0.3 * u + fg)
  v(3 * u)
  text(size: 4 * u, fill: fg.transparentize(20%), lines(d.subtitle))
})))
"##;

const T_QUOTE: &str = r##"
#place(top + left, dx: 6 * u, dy: 4 * u, text(font: chorus, size: 40 * u, fill: accent.darken(15%), "“"))
#place(left + horizon, dx: 12 * u, box(width: W - 24 * u, {
  fit(lines(d.title), W - 24 * u, H * 0.55, max: 11 * u, font: serif, style: "italic", lead: 0.3em)
  v(4 * u)
  box(baseline: 0.15em, diamond(size: 1.4 * u, stroke: 0.3 * u + fg))
  h(1.5 * u)
  text(font: sans, size: 2.8 * u, weight: "bold", d.subtitle)
  if d.eyebrow != "" { h(1.5 * u); text(font: sans, size: 2.6 * u, fill: fg.transparentize(35%), d.eyebrow) }
}))
#place(bottom + right, dx: -8 * u, dy: -6 * u, logo(7 * u))
#place(bottom + left, dx: 12 * u, dy: -7 * u, text(font: mono, size: 2.2 * u, fill: fg.transparentize(35%), d.footer))
"##;

const T_PHOTO: &str = r##"
#if d.image { place(top + left, image("/photo.png", width: W, height: H, fit: "cover")) } else { place(top + left, rect(width: W, height: H, fill: gradient.linear(accent, bg, angle: 30deg))) }
#place(top + left, rect(width: W, height: H, fill: gradient.linear(black.transparentize(100%), black.transparentize(25%), angle: 90deg)))
#set text(fill: white)
#place(bottom + left, dx: 8 * u, dy: -8 * u, box(width: W - 16 * u, {
  if d.eyebrow != "" { text(font: sans, size: 2.5 * u, tracking: 0.14em, weight: "bold", upper(d.eyebrow)); v(2 * u) }
  fit(lines(d.title), W - 16 * u, H * 0.42, max: 13 * u, weight: wt)
  v(2.5 * u)
  text(size: 4 * u, lines(d.subtitle))
  v(3 * u)
  text(font: mono, size: 2.2 * u, d.footer)
}))
#place(top + right, dx: -8 * u, dy: 7 * u, logo(7 * u))
"##;

const T_MINIMAL: &str = r##"
#place(top + left, dx: 10 * u, dy: 10 * u, line(length: W - 20 * u, stroke: 0.35 * u + fg))
#place(top + left, dx: 10 * u, dy: 11.4 * u, line(length: W - 20 * u, stroke: 0.15 * u + fg))
#place(bottom + left, dx: 10 * u, dy: -10 * u, line(length: W - 20 * u, stroke: 0.35 * u + fg))
#place(center + horizon, box(width: W - 28 * u, align(center, {
  if d.eyebrow != "" { text(font: sans, size: 2.4 * u, tracking: 0.16em, fill: fg.transparentize(30%), upper(d.eyebrow)); v(3 * u) }
  fit(align(center, lines(d.title)), W - 28 * u, H * 0.45, max: 12 * u, weight: wt)
  v(3 * u)
  text(size: 4 * u, style: "italic", fill: fg.transparentize(25%), lines(d.subtitle))
})))
#place(bottom + center, dy: -12.5 * u, text(font: mono, size: 2.1 * u, fill: fg.transparentize(35%), d.footer))
#place(top + center, dy: 13.5 * u, logo(5 * u))
"##;

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn every_template_compiles() {
        let t = SocialCardTool::new();
        for tpl in TEMPLATES {
            let mut v = Values::defaults(&t.options());
            v.set("template", castor_core::Value::Text(tpl.to_string()));
            let doc = card(&v, None).unwrap().compile().unwrap_or_else(|e| panic!("{tpl}: {e:#}"));
            assert_eq!(typ::page_size(&doc, 0), (1200.0, 630.0));
        }
    }
}
