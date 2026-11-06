//! `create.data-merge`: a CSV/TSV + a layout → one PDF per row, or one merged PDF.
//! Layouts: built-in (letter, certificate, name badges, address labels, postcards) with
//! `{column}` placeholders, or your own Typst template (`sys.inputs.<column>` or
//! `json("/row.json")`). Name badges and labels are imposed several per A4 sheet.

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{Result, anyhow};
use castor_core::options::{Choice, Cond, OptionSpec, Values};
use castor_core::{FileInfo, FileKind, InputSpec, Job, Output, Preview, RunCx, Tool, ToolMeta, tr, tr_args};

use crate::common;
use crate::render::{style, typ};
use typst::model::Document as _;

pub struct DataMergeTool {
    meta: ToolMeta,
}

impl DataMergeTool {
    pub fn new() -> Self {
        DataMergeTool {
            meta: common::meta(
                "create.data-merge",
                InputSpec::kinds(&[FileKind::Table]).max(1),
                "PDF",
                &["mail merge", "csv", "certificates", "badges", "labels", "letters", "batch", "template", "typst"],
                "merge",
            ),
        }
    }
}

pub type Row = BTreeMap<String, String>;

/// Read a CSV/TSV (delimiter sniffed: comma, semicolon, tab), UTF-8 with or without BOM.
pub fn read_table(path: &Path) -> Result<(Vec<String>, Vec<Row>)> {
    let raw = std::fs::read(path).map_err(|e| anyhow!("{}: {e}", path.display()))?;
    let text = String::from_utf8_lossy(raw.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(&raw)).into_owned();
    let first = text.lines().next().unwrap_or("");
    let delim = [b'\t', b';', b','].into_iter().max_by_key(|d| first.matches(*d as char).count()).unwrap_or(b',');
    let mut rd = csv::ReaderBuilder::new().delimiter(delim).flexible(true).trim(csv::Trim::All).from_reader(text.as_bytes());
    let headers: Vec<String> = rd.headers()?.iter().map(|h| h.trim().to_string()).collect();
    anyhow::ensure!(headers.iter().any(|h| !h.is_empty()), "{}", tr("create.data-merge.err.no_header"));
    let mut rows = Vec::new();
    for rec in rd.records() {
        let rec = rec?;
        if rec.iter().all(|c| c.trim().is_empty()) {
            continue;
        }
        let mut r = Row::new();
        for (i, h) in headers.iter().enumerate() {
            r.insert(h.clone(), rec.get(i).unwrap_or("").to_string());
        }
        rows.push(r);
    }
    Ok((headers, rows))
}

/// Replace `{column}` (case-insensitive) with the row's value. Unknown names are kept as is.
pub fn fill(template: &str, row: &Row) -> String {
    let mut out = String::with_capacity(template.len());
    let mut rest = template;
    while let Some(a) = rest.find('{') {
        out.push_str(&rest[..a]);
        let after = &rest[a + 1..];
        match after.find('}') {
            Some(b) => {
                let key = after[..b].trim();
                match row.iter().find(|(k, _)| k.eq_ignore_ascii_case(key)) {
                    Some((_, v)) => out.push_str(v),
                    None => {
                        out.push('{');
                        out.push_str(&after[..b]);
                        out.push('}');
                    }
                }
                rest = &after[b + 1..];
            }
            None => {
                out.push_str(&rest[a..]);
                rest = "";
            }
        }
    }
    out.push_str(rest);
    out
}

impl Tool for DataMergeTool {
    fn meta(&self) -> &ToolMeta {
        &self.meta
    }

    fn options(&self) -> Vec<OptionSpec> {
        vec![
            OptionSpec::choice("layout", ["letter", "certificate", "badge", "label", "postcard", "typst"].map(Choice::key).to_vec(), "certificate"),
            OptionSpec::file("template", &["typ"]).when(Cond::Eq("layout", "typst")),
            OptionSpec::text("title", "Certificate of completion").when(Cond::Ne("layout", "typst")),
            OptionSpec::text("heading", "{name}").when(Cond::Ne("layout", "typst")),
            OptionSpec::multiline("body", "for completing the course {course} on {date}.").when(Cond::Ne("layout", "typst")),
            OptionSpec::text("footer", "").when(Cond::Ne("layout", "typst")),
            OptionSpec::file("logo", &["png", "jpg", "jpeg", "svg", "webp"]).when(Cond::Ne("layout", "typst")),
            OptionSpec::color("accent", "#ffeca4").when(Cond::Ne("layout", "typst")),
            OptionSpec::choice("paper", vec![Choice::lit("a4", "A4"), Choice::lit("us-letter", "US Letter")], "a4").when(Cond::Ne("layout", "typst")),
            OptionSpec::choice("output", ["merged", "each"].map(Choice::key).to_vec(), "merged"),
            OptionSpec::text("name_pattern", "{name}").when(Cond::Eq("output", "each")),
        ]
    }

    fn plan(&self, inputs: &[FileInfo], _v: &Values) -> Result<Vec<Job>> {
        anyhow::ensure!(!inputs.is_empty(), "{}", tr("create.data-merge.err.no_csv"));
        Ok(inputs.iter().cloned().map(Job::single).collect())
    }

    fn preview(&self, input: Option<&FileInfo>, v: &Values, max_side: u32) -> Result<Option<Preview>> {
        let rows = match input {
            Some(f) if f.kind == FileKind::Table => read_table(&f.path)?.1,
            _ => vec![sample_row()],
        };
        let rows: Vec<Row> = rows.into_iter().take(if matches!(v.str("layout"), "badge" | "label") { 30 } else { 1 }).collect();
        let doc = build(v, &rows)?;
        let pages = typ::page_count(&doc);
        Ok(Some(common::preview(
            typ::png_fit(&doc, 0, max_side)?,
            common::figure(&tr_args("create.data-merge.caption", &[("pages", &pages.to_string())])),
        )))
    }

    fn run(&self, job: &Job, v: &Values, cx: &RunCx) -> Result<Vec<Output>> {
        let input = job.input()?;
        let (_, rows) = read_table(&input.path)?;
        anyhow::ensure!(!rows.is_empty(), "{}", tr("create.data-merge.err.no_rows"));
        let sheet = matches!(v.str("layout"), "badge" | "label");
        if v.str("output") == "each" && !sheet {
            let mut outs = Vec::new();
            for (i, r) in rows.iter().enumerate() {
                cx.check_cancel()?;
                let doc = build(v, std::slice::from_ref(r))?;
                let name = common::slug_keep_case(&fill(v.str("name_pattern"), r));
                let name = if name == "castor" || name.contains('{') { format!("{}-{:03}", input.stem(), i + 1) } else { name };
                let out = cx.output_named(&format!("{name}.pdf"));
                common::write(&out, &typ::pdf(&doc)?)?;
                outs.push(Output::new(out));
                cx.progress((i + 1) as f32 / rows.len() as f32);
            }
            return Ok(outs);
        }
        let doc = build_cancellable(v, &rows, cx)?;
        let out = cx.output_path(Some(input), "pdf");
        common::write(&out, &typ::pdf(&doc)?)?;
        Ok(vec![Output::new(out).with_note(tr_args(
            "create.data-merge.note",
            &[("rows", &rows.len().to_string()), ("pages", &typ::page_count(&doc).to_string())],
        ))])
    }
}

fn sample_row() -> Row {
    let mut r = Row::new();
    r.insert("name".into(), "Ada Example".into());
    r.insert("course".into(), "Typography 101".into());
    r.insert("date".into(), "6 November 2026".into());
    r.insert("role".into(), "Speaker".into());
    r.insert("org".into(), "Castor".into());
    r.insert("address".into(), "12 Rue de l’Exemple\n75000 Paris".into());
    r
}

fn build_cancellable(v: &Values, rows: &[Row], cx: &RunCx) -> Result<typ::PagedDocument> {
    if v.str("layout") != "typst" || rows.len() == 1 {
        return build(v, rows);
    }
    // User templates: compile per row and concatenate the pages.
    let mut pages = typst::ecow::EcoVec::new();
    let mut info = None;
    for (i, r) in rows.iter().enumerate() {
        cx.check_cancel()?;
        let d = build(v, std::slice::from_ref(r)).map_err(|e| anyhow!("{} {}: {e:#}", tr("create.data-merge.row"), i + 1))?;
        if info.is_none() {
            info = Some(d.info().clone());
        }
        pages.extend(d.pages().iter().cloned());
        cx.progress((i + 1) as f32 / rows.len() as f32);
    }
    Ok(typ::PagedDocument::new(pages, info.unwrap_or_default()))
}

/// Compile the layout for these rows (all rows in one document).
pub fn build(v: &Values, rows: &[Row]) -> Result<typ::PagedDocument> {
    if v.str("layout") == "typst" {
        let path = v.path("template").ok_or_else(|| anyhow!("{}", tr("create.data-merge.err.template")))?;
        let src = std::fs::read_to_string(path).map_err(|e| anyhow!("{}: {e}", path.display()))?;
        let row = rows.first().cloned().unwrap_or_default();
        let mut doc = typ::Doc::new(src).json("/row.json", &serde_json::to_value(&row)?);
        doc.inputs = row.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
        if let Some(dir) = path.parent() {
            doc = doc.root(dir);
        }
        return doc.compile();
    }
    let filled: Vec<serde_json::Value> = rows
        .iter()
        .map(|r| {
            serde_json::json!({
                "title": fill(v.str("title"), r),
                "heading": fill(v.str("heading"), r),
                "body": fill(v.str("body"), r),
                "footer": fill(v.str("footer"), r),
            })
        })
        .collect();
    let mut doc = typ::Doc::new(String::new());
    let has_logo = match v.path("logo") {
        Some(p) => {
            let img = crate::render::png::load_image(p, 1024)?;
            doc = doc.file("/logo.png", crate::render::png::encode_rgba(&img)?);
            true
        }
        None => false,
    };
    let data = serde_json::json!({
        "rows": filled, "accent": style::hex(v.color("accent")), "logo": has_logo,
        "paper": v.str("paper"),
    });
    let body = match v.str("layout") {
        "letter" => L_LETTER,
        "badge" => L_BADGE,
        "label" => L_LABEL,
        "postcard" => L_POSTCARD,
        _ => L_CERTIFICATE,
    };
    doc.source = format!("{}\n{L_COMMON}\n{body}", style::TYPST_PRELUDE);
    doc.json("/data.json", &data).compile()
}

const L_COMMON: &str = r##"
#let d = json("/data.json")
#let accent = rgb(d.accent)
#let lines(s) = s.split("\n").map(l => [#l]).join(linebreak())
#let logo(h) = if d.logo { image("/logo.png", height: h) }
#set text(font: serif, fill: ink, size: 11pt, lang: "en")
#set par(justify: true, leading: 0.6em)
"##;

const L_LETTER: &str = r##"
#set page(paper: d.paper, margin: (x: 25mm, top: 22mm, bottom: 25mm), fill: paper,
  footer: context align(center, text(size: 9pt, fill: muted, counter(page).display())))
#for (i, r) in d.rows.enumerate() {
  if i > 0 { pagebreak() }
  grid(columns: (1fr, auto), align(left, logo(14mm)), align(right, text(font: sans, size: 9pt, fill: muted, tracking: 0.08em, upper(r.title))))
  v(4mm)
  pagerule(stroke: 0.6pt + ink, size: 4pt)
  v(14mm)
  text(font: sans, weight: "bold", size: 12pt, lines(r.heading))
  v(10mm)
  lines(r.body)
  v(1fr)
  if r.footer != "" { line(length: 100%, stroke: 0.4pt + rulec); v(2mm); text(size: 9pt, fill: muted, lines(r.footer)) }
}
"##;

const L_CERTIFICATE: &str = r##"
#set page(paper: d.paper, flipped: true, margin: 14mm, fill: paper)
#for (i, r) in d.rows.enumerate() {
  if i > 0 { pagebreak() }
  block(width: 100%, height: 100%, stroke: 0.8pt + ink, inset: 3mm,
    block(width: 100%, height: 100%, stroke: 0.4pt + rulec, inset: 14mm, {
      place(top + center, dy: -14mm - 3mm - 4pt, box(fill: paper, inset: (x: 5pt), diamond(size: 7pt)))
      place(top + left, rect(width: 22mm, height: 2.2mm, fill: accent))
      place(top + right, logo(16mm))
      align(center + horizon, {
        text(font: sans, size: 11pt, tracking: 0.25em, fill: muted, upper(r.title))
        v(10mm)
        text(font: chorus, size: 46pt, lines(r.heading))
        v(4mm)
        box(width: 90mm, pagerule(stroke: 0.6pt + ink, size: 4pt))
        v(8mm)
        block(width: 150mm, align(center, text(size: 13pt, style: "italic", lines(r.body))))
      })
      place(bottom + center, text(size: 9.5pt, fill: muted, lines(r.footer)))
    }))
}
"##;

const L_BADGE: &str = r##"
// Name badges 90 × 55 mm, 2 × 5 per A4 sheet, crop marks between.
#set page(paper: "a4", margin: (x: 15mm, y: 10.5mm), fill: white)
#let badge(r) = box(width: 90mm, height: 55mm, stroke: 0.3pt + rulec, clip: true, {
  place(top + left, rect(width: 100%, height: 4mm, fill: accent))
  place(top + right, dx: -4mm, dy: 7mm, logo(8mm))
  pad(x: 7mm, top: 11mm, bottom: 6mm, {
    text(font: sans, size: 7.5pt, fill: muted, tracking: 0.12em, upper(r.title))
    v(1fr)
    text(font: sans, weight: "bold", size: 19pt, lines(r.heading))
    v(1.5mm)
    text(size: 10pt, style: "italic", lines(r.body))
    v(1fr)
    box(width: 100%, pagerule(stroke: 0.4pt + ink, size: 3pt))
    v(1mm)
    text(font: mono, size: 7pt, fill: muted, r.footer)
  })
})
#grid(columns: (90mm, 90mm), rows: 55mm, gutter: 0mm, ..d.rows.map(badge))
"##;

const L_LABEL: &str = r##"
// Address labels 70 × 37 mm, 3 × 8 per A4 sheet (common Avery/Herma layout, no margins).
#set page(paper: "a4", margin: (x: 0mm, top: 0.5mm, bottom: 0mm), fill: white)
#let label(r) = box(width: 70mm, height: 37mm, inset: (x: 6mm, y: 5mm), {
  set par(justify: false, leading: 0.45em)
  text(font: sans, weight: "bold", size: 10pt, lines(r.heading))
  v(1mm)
  text(size: 9.5pt, lines(r.body))
  if r.footer != "" { v(1fr); text(font: mono, size: 6.5pt, fill: muted, r.footer) }
})
#grid(columns: (70mm,) * 3, rows: 37mm, ..d.rows.map(label))
"##;

const L_POSTCARD: &str = r##"
#set page(width: 148mm, height: 105mm, margin: 0pt, fill: paper)
#for (i, r) in d.rows.enumerate() {
  if i > 0 { pagebreak() }
  place(top + left, rect(width: 52mm, height: 100%, fill: accent))
  place(top + left, dx: 8mm, dy: 9mm, box(width: 38mm, {
    logo(10mm)
    v(4mm)
    text(font: chorus, size: 22pt, lines(r.title))
  }))
  place(top + left, dx: 60mm, dy: 12mm, box(width: 80mm, {
    text(font: sans, weight: "bold", size: 13pt, lines(r.heading))
    v(4mm)
    text(size: 10.5pt, lines(r.body))
  }))
  place(bottom + left, dx: 60mm, dy: -8mm, box(width: 80mm, { pagerule(stroke: 0.5pt + ink, size: 3.5pt); v(1.5mm); text(size: 8.5pt, fill: muted, lines(r.footer)) }))
}
"##;

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn placeholders() {
        let mut r = Row::new();
        r.insert("Name".into(), "Zoé".into());
        assert_eq!(fill("Hi {name}, {unknown} {", &r), "Hi Zoé, {unknown} {");
    }
    #[test]
    fn layouts_compile() {
        let t = DataMergeTool::new();
        for l in ["letter", "certificate", "badge", "label", "postcard"] {
            let mut v = Values::defaults(&t.options());
            v.set("layout", castor_core::Value::Text(l.into()));
            let rows: Vec<Row> = (0..12).map(|_| sample_row()).collect();
            let d = build(&v, &rows).unwrap_or_else(|e| panic!("{l}: {e:#}"));
            assert!(typ::page_count(&d) >= 1);
        }
    }
}
