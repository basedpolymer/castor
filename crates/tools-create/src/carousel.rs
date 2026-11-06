//! `create.carousel`: multi-slide social carousels from text blocks (and dropped images, one per
//! slide in order) with one consistent template; every slide as PNG plus a PDF of the set.

use anyhow::Result;
use castor_core::options::{Choice, OptionSpec, Values};
use castor_core::{FileInfo, InputSpec, Job, Output, Preview, RunCx, Tool, ToolMeta, tr, tr_args};

use crate::common;
use crate::render::{png, style, typ};

pub struct CarouselTool {
    meta: ToolMeta,
}

impl CarouselTool {
    pub fn new() -> Self {
        CarouselTool {
            meta: common::meta(
                "create.carousel",
                InputSpec::kinds(common::IMAGES).min(0),
                "PNG · PDF",
                &["carousel", "slides", "instagram", "linkedin", "swipe", "post", "social", "thread"],
                "carousel",
            ),
        }
    }
}

const SIZES: &[(&str, u32, u32)] = &[("portrait", 1080, 1350), ("square", 1080, 1080), ("story", 1080, 1920), ("landscape", 1920, 1080)];

impl Tool for CarouselTool {
    fn meta(&self) -> &ToolMeta {
        &self.meta
    }

    fn options(&self) -> Vec<OptionSpec> {
        vec![
            OptionSpec::multiline(
                "slides",
                "Five things Castor does offline\nSwipe →\n---\nCompress PDFs\nSmaller files, sharp text.\n---\nQR codes that always scan\nEvery code is decoded before it is saved.\n---\nRename 500 photos\nBy date taken, numbered, undoable.\n---\nTry it\nFree and open source.",
            ),
            OptionSpec::choice("template", ["classic", "bold", "minimal", "photo"].map(Choice::key).to_vec(), "classic"),
            OptionSpec::choice("size", SIZES.iter().map(|(k, _, _)| Choice::key(k)).collect(), "portrait"),
            OptionSpec::color("background", "#fcfbf9"),
            OptionSpec::color("color", "#1b1616"),
            OptionSpec::color("accent", "#ffeca4"),
            OptionSpec::text("handle", "@castor"),
            OptionSpec::toggle("numbers", true),
            OptionSpec::text("name", "carousel"),
            OptionSpec::toggle("pdf", true),
        ]
    }

    fn plan(&self, inputs: &[FileInfo], _v: &Values) -> Result<Vec<Job>> {
        Ok(vec![Job::many(inputs.to_vec())])
    }

    fn preview(&self, input: Option<&FileInfo>, v: &Values, max_side: u32) -> Result<Option<Preview>> {
        let imgs: Vec<FileInfo> = input.into_iter().cloned().collect();
        let doc = build(v, &imgs)?.compile()?;
        let n = typ::page_count(&doc);
        // A contact strip of the first slides.
        let show = n.min(3);
        let (pw, ph) = typ::page_size(&doc, 0);
        let k = (max_side as f64 / (pw * show as f64 + 24.0 * (show as f64 - 1.0)).max(ph)).min(1.0);
        let (sw, sh) = ((pw * k) as u32, (ph * k) as u32);
        let gap = (24.0 * k) as u32;
        let mut canvas = image::RgbaImage::from_pixel(sw * show as u32 + gap * (show as u32 - 1), sh, image::Rgba([0xf4, 0xf2, 0xee, 255]));
        for i in 0..show {
            let pm = typ::pixmap(&doc, i, k as f32)?;
            image::imageops::overlay(&mut canvas, &png::pixmap_to_rgba(&pm), (i as u32 * (sw + gap)) as i64, 0);
        }
        Ok(Some(common::preview(png::encode_rgba(&canvas)?, common::figure(&tr_args("create.carousel.caption", &[("count", &n.to_string())])))))
    }

    fn run(&self, job: &Job, v: &Values, cx: &RunCx) -> Result<Vec<Output>> {
        let doc = build(v, &job.inputs)?.compile()?;
        let n = typ::page_count(&doc);
        let base = common::slug_keep_case(if v.str("name").trim().is_empty() { "carousel" } else { v.str("name") });

        let mut outs = Vec::new();
        for i in 0..n {
            cx.check_cancel()?;
            let out = cx.output_named(&format!("{base}-{:02}.png", i + 1));
            let pm = typ::pixmap(&doc, i, 1.0)?;
            common::write(&out, &png::encode_pixmap(&pm)?)?;
            outs.push(Output::new(out));
            cx.progress((i + 1) as f32 / (n + 1) as f32);
        }
        if v.bool("pdf") {
            let out = cx.output_named(&format!("{base}.pdf"));
            common::write(&out, &typ::pdf(&doc)?)?;
            outs.push(Output::new(out).with_note(tr_args("create.carousel.caption", &[("count", &n.to_string())])));
        }
        Ok(outs)
    }
}

/// Split the text into (title, body) slides at lines made of "---".
pub fn parse_slides(s: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for block in s.split('\n').collect::<Vec<_>>().split(|l| l.trim() == "---") {
        let lines: Vec<&str> = block.iter().map(|l| l.trim_end()).skip_while(|l| l.trim().is_empty()).collect();
        if lines.iter().all(|l| l.trim().is_empty()) {
            continue;
        }
        let title = lines[0].trim().trim_start_matches('#').trim().to_string();
        let body = lines[1..].join("\n").trim().to_string();
        out.push((title, body));
    }
    out
}

pub fn build(v: &Values, images: &[FileInfo]) -> Result<typ::Doc> {
    let slides = parse_slides(v.str("slides"));
    anyhow::ensure!(!slides.is_empty(), "{}", tr("create.carousel.err.empty"));
    let size = v.str("size");
    let (w, h) = SIZES.iter().find(|s| s.0 == size).map(|s| (s.1, s.2)).unwrap_or((1080, 1350));
    let mut doc = typ::Doc::new(String::new());
    let mut list = Vec::new();
    for (i, (t, b)) in slides.iter().enumerate() {
        let img = match images.get(i) {
            Some(f) => {
                let p = format!("/img{i}.png");
                doc = doc.file(&p, png::encode_rgba(&png::load_image(&f.path, 2200)?)?);
                p
            }
            None => String::new(),
        };
        list.push(serde_json::json!({"title": t, "body": b, "img": img}));
    }
    let accent = v.color("accent");
    let data = serde_json::json!({
        "w": w, "h": h, "slides": list, "handle": v.str("handle"), "numbers": v.bool("numbers"),
        "bg": style::hex(v.color("background")), "fg": style::hex(v.color("color")),
        "accent": style::hex(accent), "on_accent": style::hex(style::text_on(accent)),
        "template": v.str("template"), "swipe": tr("create.carousel.swipe"),
    });
    doc.source = format!("{}\n{CAROUSEL}", style::TYPST_PRELUDE);
    Ok(doc.json("/data.json", &data))
}

const CAROUSEL: &str = r##"
#let d = json("/data.json")
#let W = d.w * 1pt
#let H = d.h * 1pt
#let u = W / 100
#let bg = rgb(d.bg)
#let fg = rgb(d.fg)
#let accent = rgb(d.accent)
#let n = d.slides.len()
#set page(width: W, height: H, margin: 0pt, fill: bg)
#set text(font: serif, fill: fg, size: 3.6 * u, hyphenate: false)
#set par(leading: 0.5em)
#let lines(s) = s.split("\n").map(l => [#l]).join(linebreak())
#let fit(body, w, h, max: 9 * u, min: 3.5 * u, font: sans, weight: "bold") = context {
  let s = max
  let mk(s) = block(width: w, text(font: font, size: s, weight: weight, par(leading: 0.25em, body)))
  while s > min and measure(mk(s)).height > h { s = s * 0.94 }
  mk(s)
}
#let footer(i) = {
  place(bottom + left, dx: 7 * u, dy: -6 * u, text(font: mono, size: 2.4 * u, fill: fg.transparentize(30%), d.handle))
  if d.numbers { place(bottom + right, dx: -7 * u, dy: -6 * u, text(font: sans, size: 2.4 * u, fill: fg.transparentize(30%), str(i + 1) + " / " + str(n))) }
  place(bottom + left, dx: 7 * u, dy: -11 * u, box(width: W - 14 * u, pagerule(stroke: 0.18 * u + fg.transparentize(40%), size: 1.1 * u)))
}
#for (i, s) in d.slides.enumerate() {
  if i > 0 { pagebreak() }
  let first = i == 0
  let last = i == n - 1
  if d.template == "bold" {
    place(top + left, rect(width: W, height: H, fill: if calc.rem(i, 2) == 0 { accent } else { bg }))
    let col = if calc.rem(i, 2) == 0 { rgb(d.on_accent) } else { fg }
    set text(fill: col)
    place(top + left, dx: 7 * u, dy: 7 * u, text(font: sans, weight: "bold", size: 14 * u, fill: col.transparentize(80%), str(i + 1)))
    place(left + horizon, dx: 7 * u, box(width: W - 14 * u, {
      fit(lines(s.title), W - 14 * u, H * 0.4, max: (if first { 12 } else { 9 }) * u)
      v(3 * u)
      text(size: 4 * u, lines(s.body))
      if s.img != "" { v(4 * u); box(radius: 2 * u, clip: true, image(s.img, width: W - 14 * u, height: H * 0.3, fit: "cover")) }
    }))
  } else if d.template == "photo" and s.img != "" {
    place(top + left, image(s.img, width: W, height: H, fit: "cover"))
    place(top + left, rect(width: W, height: H, fill: gradient.linear(black.transparentize(100%), black.transparentize(20%), angle: 90deg)))
    set text(fill: white)
    place(bottom + left, dx: 7 * u, dy: -16 * u, box(width: W - 14 * u, {
      fit(lines(s.title), W - 14 * u, H * 0.3, max: 9 * u)
      v(2 * u)
      text(size: 3.8 * u, lines(s.body))
    }))
  } else if d.template == "minimal" {
    place(center + horizon, box(width: W - 20 * u, align(center, {
      diamond(size: 1.8 * u, stroke: 0.25 * u + fg)
      v(4 * u)
      fit(align(center, lines(s.title)), W - 20 * u, H * 0.35, max: 8 * u, font: serif, weight: "regular")
      v(3 * u)
      text(size: 3.8 * u, style: "italic", fill: fg.transparentize(25%), lines(s.body))
      if s.img != "" { v(4 * u); box(radius: 1.5 * u, clip: true, image(s.img, width: W - 30 * u, height: H * 0.3, fit: "cover")) }
    })))
  } else {
    // classic
    place(top + left, dx: 2 * u, dy: -5 * u, text(font: chorus, size: 40 * u, fill: fg.transparentize(92%), s.title.clusters().at(0, default: "")))
    place(top + left, dx: 7 * u, dy: 8 * u, box(fill: accent, inset: (x: 1.6 * u, y: 0.9 * u), radius: 0.6 * u, text(font: sans, weight: "bold", size: 2.4 * u, fill: rgb(d.on_accent), if first { d.handle } else { str(i) })))
    place(if s.img != "" { top + left } else { left + horizon }, dx: 7 * u, dy: if s.img != "" { 18 * u } else { -4 * u }, box(width: W - 14 * u, {
      fit(lines(s.title), W - 14 * u, if s.img != "" { H * 0.25 } else { H * 0.42 }, max: (if first { 12 } else { 9.5 }) * u)
      v(3 * u)
      text(size: 4.4 * u, fill: fg.transparentize(15%), lines(s.body))
      if s.img != "" { v(4 * u); box(radius: 1.5 * u, clip: true, stroke: 0.15 * u + fg.transparentize(70%), image(s.img, width: W - 14 * u, height: H * 0.38, fit: "cover")) }
    }))
  }
  footer(i)
  if first and n > 1 { place(bottom + right, dx: -7 * u, dy: -14 * u, text(font: sans, weight: "bold", size: 2.8 * u, fill: fg, d.swipe + " →")) }
}
"##;

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn slides_compile() {
        assert_eq!(parse_slides("A\nb\n---\n\n# C\n---\n").len(), 2);
        let t = CarouselTool::new();
        for tpl in ["classic", "bold", "minimal", "photo"] {
            let mut v = Values::defaults(&t.options());
            v.set("template", castor_core::Value::Text(tpl.into()));
            let d = build(&v, &[]).unwrap().compile().unwrap_or_else(|e| panic!("{tpl}: {e:#}"));
            assert_eq!(typ::page_count(&d), 5);
        }
    }
}
