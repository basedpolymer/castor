//! `create.brand-sheet`: a one-page brand sheet (PDF or PNG) from a logo, colours and fonts —
//! logo on light and dark with clear space, swatches with HEX / RGB / HSL / CMYK, type
//! specimens and a WCAG contrast table. Typeset with Typst in the house style.

use anyhow::Result;
use castor_core::options::{Choice, Cond, OptionSpec, Values};
use castor_core::{FileInfo, InputSpec, Job, Output, Preview, RunCx, Tool, ToolMeta, tr, tr_args};

use crate::common;
use crate::render::{png, style, typ};

pub struct BrandSheetTool {
    meta: ToolMeta,
}

impl BrandSheetTool {
    pub fn new() -> Self {
        BrandSheetTool {
            meta: common::meta(
                "create.brand-sheet",
                InputSpec::none(),
                "PDF · PNG",
                &["brand", "style guide", "identity", "colours", "palette", "logo", "fonts", "guidelines", "one pager"],
                "brand-sheet",
            ),
        }
    }
}

impl Tool for BrandSheetTool {
    fn meta(&self) -> &ToolMeta {
        &self.meta
    }

    fn options(&self) -> Vec<OptionSpec> {
        vec![
            OptionSpec::text("name", "Castor"),
            OptionSpec::text("tagline", "Every file tool, right on your desk."),
            OptionSpec::file("logo", &["png", "jpg", "jpeg", "svg", "webp"]),
            OptionSpec::file("logo_dark", &["png", "jpg", "jpeg", "svg", "webp"]),
            OptionSpec::multiline("colors", "Ink #1b1616\nPaper #fcfbf9\nCreate #ffeca4\nImage #ccfaff\nPDF #fbe2ff\nLink #2f6db5"),
            OptionSpec::choice("heading_font", ["sans", "serif", "chorus", "system"].map(Choice::key).to_vec(), "sans"),
            OptionSpec::font("heading_family", "").when(Cond::Eq("heading_font", "system")),
            OptionSpec::choice("body_font", ["serif", "sans", "system"].map(Choice::key).to_vec(), "serif"),
            OptionSpec::font("body_family", "").when(Cond::Eq("body_font", "system")),
            OptionSpec::multiline("notes", ""),
            OptionSpec::choice("format", vec![Choice::lit("pdf", "PDF"), Choice::lit("png", "PNG")], "pdf"),
            OptionSpec::text("file_name", "").advanced(),
        ]
    }

    fn preview(&self, _input: Option<&FileInfo>, v: &Values, max_side: u32) -> Result<Option<Preview>> {
        let doc = sheet(v)?.compile()?;
        Ok(Some(common::preview(typ::png_fit(&doc, 0, max_side)?, common::figure("A4"))))
    }

    fn run(&self, _job: &Job, v: &Values, cx: &RunCx) -> Result<Vec<Output>> {
        let doc = sheet(v)?.compile()?;
        let name = if v.str("file_name").trim().is_empty() { format!("{}-brand-sheet", common::slug(v.str("name"))) } else { v.str("file_name").to_string() };
        if v.str("format") == "png" {
            let out = common::out_named(cx, &name, "png");
            common::write(&out, &typ::png_width(&doc, 0, 2480)?)?;
            return Ok(vec![Output::new(out).with_note("2480 × 3508 px (A4, 300 dpi)")]);
        }
        let out = common::out_named(cx, &name, "pdf");
        common::write(&out, &typ::pdf(&doc)?)?;
        Ok(vec![Output::new(out)])
    }
}

/// "Name #hex" lines (name optional) → (name, colour).
pub fn parse_named_colors(s: &str) -> Vec<(String, [u8; 4])> {
    s.lines()
        .filter_map(|l| {
            let l = l.trim();
            let i = l.find('#')?;
            let hex: String = l[i..].chars().take_while(|c| *c == '#' || c.is_ascii_hexdigit()).collect();
            let c = castor_core::options::parse_hex_color(&hex)?;
            let name = l[..i].trim().trim_end_matches([':', '-', '—', '=']).trim();
            Some((if name.is_empty() { hex.to_uppercase() } else { name.to_string() }, c))
        })
        .collect()
}

fn hsl(c: [u8; 4]) -> (f32, f32, f32) {
    let (r, g, b) = (c[0] as f32 / 255.0, c[1] as f32 / 255.0, c[2] as f32 / 255.0);
    let (mx, mn) = (r.max(g).max(b), r.min(g).min(b));
    let l = (mx + mn) / 2.0;
    if mx == mn {
        return (0.0, 0.0, l * 100.0);
    }
    let d = mx - mn;
    let s = if l > 0.5 { d / (2.0 - mx - mn) } else { d / (mx + mn) };
    let h = if mx == r {
        (g - b) / d + if g < b { 6.0 } else { 0.0 }
    } else if mx == g {
        (b - r) / d + 2.0
    } else {
        (r - g) / d + 4.0
    };
    (h * 60.0, s * 100.0, l * 100.0)
}

/// Naive device CMYK (no ICC profile): fine as a starting point for print briefs.
fn cmyk(c: [u8; 4]) -> (f32, f32, f32, f32) {
    let (r, g, b) = (c[0] as f32 / 255.0, c[1] as f32 / 255.0, c[2] as f32 / 255.0);
    let k = 1.0 - r.max(g).max(b);
    if k >= 1.0 {
        return (0.0, 0.0, 0.0, 100.0);
    }
    (((1.0 - r - k) / (1.0 - k)) * 100.0, ((1.0 - g - k) / (1.0 - k)) * 100.0, ((1.0 - b - k) / (1.0 - k)) * 100.0, k * 100.0)
}

fn grade(ratio: f32) -> &'static str {
    if ratio >= 7.0 {
        "AAA"
    } else if ratio >= 4.5 {
        "AA"
    } else if ratio >= 3.0 {
        "AA large"
    } else {
        "—"
    }
}

fn family(choice: &str, system: &str, default: &str) -> String {
    match choice {
        "sans" => "Latin Modern Sans".into(),
        "serif" => "Latin Modern Roman".into(),
        "chorus" => "TeX Gyre Chorus".into(),
        "system" if !system.trim().is_empty() => system.trim().to_string(),
        _ => default.into(),
    }
}

pub fn sheet(v: &Values) -> Result<typ::Doc> {
    let colors = parse_named_colors(v.str("colors"));
    anyhow::ensure!(!colors.is_empty(), "{}", tr("create.brand-sheet.err.colors"));
    let head = family(v.str("heading_font"), v.str("heading_family"), "Latin Modern Sans");
    let body = family(v.str("body_font"), v.str("body_family"), "Latin Modern Roman");
    let mut doc = typ::Doc::new(String::new()).family(&head).family(&body);
    let mut logo = false;
    if let Some(p) = v.path("logo") {
        doc = doc.file("/logo.png", png::encode_rgba(&png::load_image(p, 1600)?)?);
        logo = true;
    }
    let mut logo_dark = false;
    if let Some(p) = v.path("logo_dark") {
        doc = doc.file("/logo-dark.png", png::encode_rgba(&png::load_image(p, 1600)?)?);
        logo_dark = true;
    }
    let swatches: Vec<serde_json::Value> = colors
        .iter()
        .map(|(n, c)| {
            let (h, s, l) = hsl(*c);
            let (cc, m, y, k) = cmyk(*c);
            serde_json::json!({
                "name": n, "hex": style::hex(*c).to_uppercase(),
                "rgb": format!("{} {} {}", c[0], c[1], c[2]),
                "hsl": format!("{h:.0}° {s:.0} % {l:.0} %"),
                "cmyk": format!("{cc:.0} {m:.0} {y:.0} {k:.0}"),
                "on": style::hex(style::text_on(*c)),
            })
        })
        .collect();
    // Contrast: every pair, best first (up to 10).
    let mut pairs = Vec::new();
    for (i, (na, a)) in colors.iter().enumerate() {
        for (nb, b) in colors.iter().skip(i + 1) {
            let r = style::contrast(*a, *b);
            pairs.push((r, na.clone(), nb.clone(), style::hex(*a), style::hex(*b)));
        }
    }
    pairs.sort_by(|x, y| y.0.partial_cmp(&x.0).unwrap_or(std::cmp::Ordering::Equal));
    let contrast: Vec<serde_json::Value> = pairs
        .iter()
        .take(8)
        .map(|(r, na, nb, a, b)| serde_json::json!({"a": na, "b": nb, "ca": a, "cb": b, "ratio": format!("{r:.2} : 1"), "grade": grade(*r)}))
        .collect();
    let darkest = colors.iter().min_by(|x, y| style::luminance(x.1).partial_cmp(&style::luminance(y.1)).unwrap()).map(|c| style::hex(c.1)).unwrap_or_default();
    let (y, m, d) = typ::today();
    let data = serde_json::json!({
        "name": v.str("name"), "tagline": v.str("tagline"), "notes": v.str("notes"),
        "logo": logo, "logo_dark": logo_dark, "swatches": swatches, "contrast": contrast,
        "head": head, "body": body, "dark": darkest, "date": format!("{y}-{m:02}-{d:02}"),
        "t": {
            "title": tr("create.brand-sheet.t.title"), "logo": tr("create.brand-sheet.t.logo"),
            "clear": tr("create.brand-sheet.t.clear"), "colour": tr("create.brand-sheet.t.colour"),
            "type": tr("create.brand-sheet.t.type"), "contrast": tr("create.brand-sheet.t.contrast"),
            "pair": tr("create.brand-sheet.t.pair"), "ratio": tr("create.brand-sheet.t.ratio"),
            "wcag": tr("create.brand-sheet.t.wcag"), "notes": tr("create.brand-sheet.t.notes"),
            "headings": tr("create.brand-sheet.t.headings"), "text": tr("create.brand-sheet.t.text"),
            "sample": tr("create.brand-sheet.t.sample"),
            "nologo": tr("create.brand-sheet.t.nologo"),
        },
    });
    doc.source = format!("{}\n{SHEET}", style::TYPST_PRELUDE);
    let _ = tr_args;
    Ok(doc.json("/data.json", &data))
}

const SHEET: &str = r##"
#let d = json("/data.json")
#let t = d.t
#set page(paper: "a4", margin: (x: 16mm, top: 16mm, bottom: 14mm), fill: paper,
  footer: { set text(font: serif, size: 7.5pt, fill: muted); d.name + " — " + t.title; h(1fr); d.date })
#set text(font: d.body, size: 9pt, fill: ink, fallback: true)
#let section(n, title) = { v(3.2mm); text(font: sans, weight: "bold", size: 11pt, [#n.#h(0.6em)#title]); v(-1.6mm); line(length: 100%, stroke: 0.4pt + rulec); v(1.2mm) }
#place(top + left, dx: -6mm, dy: -14mm, text(font: chorus, size: 96pt, fill: ink.transparentize(91%), d.name.clusters().at(0, default: "")))
#grid(columns: (1fr, 52mm), align: (left + bottom, right + bottom),
  { text(font: d.head, weight: "bold", size: 26pt, d.name); linebreak(); text(size: 11pt, style: "italic", fill: muted, d.tagline) },
  { text(font: sans, size: 8pt, fill: muted, tracking: 0.1em, upper(t.title)); v(-2.4mm); pagerule(stroke: 0.5pt + ink, size: 3.5pt) })
#section(1, t.logo)
#let logobox(fill, file, label, fg) = box(width: 100%, height: 30mm, fill: fill, stroke: 0.4pt + rulec, radius: 1.5mm, {
  if d.logo {
    place(center + horizon, box(stroke: (paint: fg.transparentize(55%), thickness: 0.5pt, dash: "dashed"), inset: 4mm, image(file, height: 16mm)))
  } else {
    place(center + horizon, text(font: d.head, weight: "bold", size: 20pt, fill: fg, d.name))
  }
  place(bottom + left, dx: 2.5mm, dy: -2mm, text(font: mono, size: 6.5pt, fill: fg.transparentize(35%), label))
})
#grid(columns: (1fr, 1fr), gutter: 4mm,
  logobox(white, "/logo.png", t.clear, ink),
  logobox(rgb(d.dark), if d.logo_dark { "/logo-dark.png" } else { "/logo.png" }, d.dark, white))
#section(2, t.colour)
#let n = d.swatches.len()
#let cols = if n <= 3 { n } else if n == 4 { 4 } else { 3 }
#grid(columns: (1fr,) * cols, gutter: 3mm,
  ..d.swatches.map(s => box(width: 100%, stroke: 0.4pt + rulec, radius: 1.5mm, clip: true, {
    block(width: 100%, height: 14mm, fill: rgb(s.hex), inset: 2.5mm,
      align(bottom + left, text(font: sans, weight: "bold", size: 9.5pt, fill: rgb(s.on), s.name)))
    block(width: 100%, inset: (x: 2.5mm, y: 2mm), {
      set text(font: mono, size: 6.8pt)
      grid(columns: (9mm, 1fr), row-gutter: 1.3mm,
        text(fill: muted, "HEX"), s.hex, text(fill: muted, "RGB"), s.rgb,
        text(fill: muted, "HSL"), s.hsl, text(fill: muted, "CMYK"), s.cmyk)
    })
  })))
#section(3, t.type)
#grid(columns: (1fr, 1fr), gutter: 6mm,
  { text(font: d.head, weight: "bold", size: 34pt, "Aa"); h(3mm); text(font: mono, size: 7pt, fill: muted, d.head + " — " + t.headings); linebreak(); text(font: d.head, weight: "bold", size: 13pt, t.sample) },
  { text(font: d.body, size: 34pt, "Aa"); h(3mm); text(font: mono, size: 7pt, fill: muted, d.body + " — " + t.text); linebreak(); par(justify: true, text(font: d.body, size: 9pt, "Lorem ipsum dolor sit amet, consectetur adipiscing elit. Integer posuere erat a ante venenatis dapibus, posuere velit aliquet. 0123456789 — “quotes” & ligatures fi fl.")) })
#section(4, t.contrast)
#text(font: serif, size: 8pt, fill: muted, [*Table 1* — ] + t.wcag)
#v(-1.5mm)
#line(length: 100%, stroke: 0.9pt + ink)
#v(-2.2mm)
#table(columns: (1fr, 26mm, 22mm, 30mm), stroke: none, inset: (x: 0pt, y: 2.6pt), align: (left, left, right, right),
  text(font: sans, weight: "bold", size: 8pt, t.pair), text(font: sans, weight: "bold", size: 8pt, "Aa"), text(font: sans, weight: "bold", size: 8pt, t.ratio), text(font: sans, weight: "bold", size: 8pt, "WCAG"),
  table.hline(stroke: 0.4pt + ink),
  ..d.contrast.map(c => (
    text(size: 8pt, c.a + " / " + c.b),
    box(fill: rgb(c.cb), inset: (x: 2mm, y: 0.8mm), radius: 0.8mm, text(font: sans, weight: "bold", size: 7.5pt, fill: rgb(c.ca), "Aa")),
    text(font: mono, size: 7.5pt, c.ratio), text(font: sans, size: 7.5pt, c.grade))).flatten())
#v(-2.2mm)
#line(length: 100%, stroke: 0.9pt + ink)
#if d.notes != "" { section(5, t.notes); text(size: 9pt, d.notes) }
"##;

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn colours_and_sheet() {
        let c = parse_named_colors("Ink #1b1616\n#fff\nAccent: #FFECA4 (main)");
        assert_eq!(c.len(), 3);
        assert_eq!(c[2].0, "Accent");
        assert_eq!(c[1].0, "#FFF");
        let t = BrandSheetTool::new();
        let v = Values::defaults(&t.options());
        let d = sheet(&v).unwrap().compile().unwrap();
        assert_eq!(typ::page_count(&d), 1, "brand sheet must fit on one page");
    }
}
