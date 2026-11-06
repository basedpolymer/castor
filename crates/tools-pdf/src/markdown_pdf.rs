//! pdf.from-markdown — Markdown → PDF in the house ("TeXtured") style: Latin Modern, numbered
//! headings, captions, optional title page and table of contents, page numbers.
//!
//! Rendering uses MuPDF's HTML/CSS layout engine (hyphenation and justification included): the
//! Markdown is turned into XHTML with pulldown-cmark, laid out, and converted to PDF.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use castor_core::options::{Choice, OptionSpec, Values};
use castor_core::{Family, FileKind, InputSpec, Job, Output, Result, RunCx, Tool, ToolMeta, ToolStatus};
use engine_pdf::draw::Canvas;
use engine_pdf::fonts::{self, Face};
use pulldown_cmark::{CodeBlockKind, Event, HeadingLevel, Options as MdOptions, Parser, Tag, TagEnd};

use crate::common::*;
use crate::structure::xml_escape;

pub struct FromMarkdown {
    meta: ToolMeta,
}

impl FromMarkdown {
    pub fn new() -> Self {
        let mut m = meta(
            "pdf.from-markdown",
            Family::Pdf,
            InputSpec::kinds(&[FileKind::Markdown]).with_extensions(&["md", "markdown", "txt"]),
            "PDF",
            &["markdown", "md", "notes", "typeset", "latex", "report", "pretty"],
            "",
        );
        m.status = ToolStatus::Beta;
        FromMarkdown { meta: m }
    }
}

struct Front {
    title: String,
    author: String,
    date: String,
}

fn front_matter(src: &str) -> (Front, &str) {
    let mut f = Front { title: String::new(), author: String::new(), date: String::new() };
    let s = src.trim_start_matches('\u{feff}');
    if let Some(rest) = s.strip_prefix("---\n").or_else(|| s.strip_prefix("---\r\n")) {
        if let Some(end) = rest.find("\n---") {
            for line in rest[..end].lines() {
                if let Some((k, v)) = line.split_once(':') {
                    let v = v.trim().trim_matches('"').trim_matches('\'').to_string();
                    match k.trim().to_lowercase().as_str() {
                        "title" => f.title = v,
                        "author" | "authors" => f.author = v,
                        "date" => f.date = v,
                        _ => {}
                    }
                }
            }
            let after = &rest[end + 4..];
            let after = after.split_once('\n').map(|x| x.1).unwrap_or("");
            return (f, after);
        }
    }
    (f, s)
}

const CSS: &str = r#"
@font-face { font-family: "LM"; src: url("lmr.otf"); }
@font-face { font-family: "LM"; font-weight: bold; src: url("lmb.otf"); }
@font-face { font-family: "LM"; font-style: italic; src: url("lmi.otf"); }
@font-face { font-family: "LMS"; src: url("lms.otf"); }
@font-face { font-family: "LMS"; font-weight: bold; src: url("lmsb.otf"); }
@font-face { font-family: "LMM"; src: url("lmm.otf"); }
body { font-family: "LM"; line-height: 1.32; text-align: justify; hyphens: auto; margin: 0; }
p { margin: 0 0 0.55em 0; }
h1, h2, h3, h4, h5, h6 { font-family: "LM"; font-weight: bold; text-align: left; hyphens: none; line-height: 1.2; }
h1 { font-size: 1.6em; margin: 1.2em 0 0.6em 0; }
h2 { font-size: 1.3em; margin: 1.1em 0 0.5em 0; }
h3 { font-size: 1.12em; margin: 1em 0 0.4em 0; }
h4, h5, h6 { font-size: 1em; margin: 0.9em 0 0.3em 0; }
.num { padding-right: 0.7em; }
code, pre { font-family: "LMM"; font-size: 0.92em; }
pre { text-align: left; white-space: pre-wrap; margin: 0.6em 0 0.8em 1.2em; line-height: 1.2; }
blockquote { margin: 0.6em 2em; font-style: italic; }
ul, ol { margin: 0.3em 0 0.6em 1.4em; padding: 0; }
li { margin: 0 0 0.2em 0; }
table { border-collapse: collapse; margin: 0.8em auto; font-size: 0.95em; }
thead th { border-top: 1.2pt solid black; border-bottom: 0.6pt solid black; }
tbody tr:last-child td { border-bottom: 1.2pt solid black; }
th, td { padding: 0.2em 0.6em; text-align: left; }
.fig { text-align: center; margin: 0.8em 0; page-break-inside: avoid; }
.fig img { max-width: 100%; }
.cap { text-align: center; font-size: 0.92em; margin: 0.3em 2em 0.9em 2em; }
.capl { font-family: "LMS"; font-weight: bold; }
hr { border: none; border-top: 0.5pt solid #888; margin: 1em 20%; }
.title { text-align: center; margin-top: 30%; page-break-after: always; }
.title .t { font-size: 2.2em; font-weight: bold; line-height: 1.2; margin-bottom: 1.2em; }
.title .a { font-size: 1.25em; margin-bottom: 0.4em; }
.title .d { font-size: 1.05em; }
.toc { page-break-after: always; }
.toc h1 { margin-top: 0; }
.toc p { text-align: left; margin: 0.15em 0; }
.toc .l2 { margin-left: 1.6em; }
.toc .l3 { margin-left: 3.2em; }
.toc .pg { float: right; }
.fn { font-size: 0.85em; }
"#;

struct Heading {
    level: u8,
    number: String,
    text: String,
}

/// Markdown → XHTML body; collects headings; copies local images into `dir`.
fn render_body(md: &str, numbered: bool, base: &Path, dir: &Path, headings: &mut Vec<Heading>) -> Result<String> {
    let mut opts = MdOptions::empty();
    opts.insert(MdOptions::ENABLE_TABLES);
    opts.insert(MdOptions::ENABLE_FOOTNOTES);
    opts.insert(MdOptions::ENABLE_STRIKETHROUGH);
    opts.insert(MdOptions::ENABLE_TASKLISTS);
    let parser = Parser::new_ext(md, opts);
    let mut html = String::new();
    let mut counters = [0u32; 6];
    let mut in_heading: Option<(u8, String)> = None;
    let mut in_image: Option<(String, String)> = None; // (src, alt so far)
    let mut figures = 0;
    let mut images = 0;
    let mut in_table_head = false;
    for ev in parser {
        if let Some((_, ref mut alt)) = in_image {
            match ev {
                Event::Text(t) | Event::Code(t) => {
                    alt.push_str(&t);
                    continue;
                }
                Event::End(TagEnd::Image) => {}
                _ => continue,
            }
        }
        if let Some((_, ref mut text)) = in_heading {
            match &ev {
                Event::Text(t) | Event::Code(t) => {
                    text.push_str(t);
                    continue;
                }
                Event::End(TagEnd::Heading(_)) => {}
                _ => continue,
            }
        }
        match ev {
            Event::Start(Tag::Heading { level, .. }) => {
                let l = match level {
                    HeadingLevel::H1 => 1,
                    HeadingLevel::H2 => 2,
                    HeadingLevel::H3 => 3,
                    HeadingLevel::H4 => 4,
                    HeadingLevel::H5 => 5,
                    HeadingLevel::H6 => 6,
                };
                in_heading = Some((l, String::new()));
            }
            Event::End(TagEnd::Heading(_)) => {
                let (l, text) = in_heading.take().unwrap_or((1, String::new()));
                let li = l as usize - 1;
                counters[li] += 1;
                for c in counters.iter_mut().skip(li + 1) {
                    *c = 0;
                }
                let number = if numbered && l <= 3 {
                    counters[..=li].iter().skip_while(|c| **c == 0).map(|c| c.to_string()).collect::<Vec<_>>().join(".")
                } else {
                    String::new()
                };
                let num_html = if number.is_empty() { String::new() } else { format!("<span class=\"num\">{number}</span>") };
                let _ = write!(html, "<h{l}>{num_html}{}</h{l}>\n", xml_escape(&text));
                headings.push(Heading { level: l, number, text });
            }
            Event::Start(Tag::Image { dest_url, .. }) => {
                in_image = Some((dest_url.to_string(), String::new()));
            }
            Event::End(TagEnd::Image) => {
                let (src, alt) = in_image.take().unwrap_or_default();
                let local = if src.starts_with("http://") || src.starts_with("https://") || src.starts_with("data:") {
                    None
                } else {
                    let p = base.join(src.replace("%20", " "));
                    p.is_file().then_some(p)
                };
                if let Some(p) = local {
                    images += 1;
                    let ext = p.extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_else(|| "png".into());
                    let name = format!("img{images}.{ext}");
                    std::fs::copy(&p, dir.join(&name))?;
                    figures += 1;
                    let cap = if alt.trim().is_empty() {
                        String::new()
                    } else {
                        format!(
                            "<p class=\"cap\"><span class=\"capl\">{} {figures}</span> {}</p>",
                            xml_escape(&tr("pdf.from-markdown.figure")),
                            xml_escape(&alt)
                        )
                    };
                    let _ = write!(html, "</p><div class=\"fig\"><img src=\"{name}\"/></div>{cap}<p>");
                } else if !alt.is_empty() {
                    let _ = write!(html, "<em>{}</em>", xml_escape(&alt));
                }
            }
            Event::Start(Tag::Paragraph) => html.push_str("<p>"),
            Event::End(TagEnd::Paragraph) => html.push_str("</p>\n"),
            Event::Start(Tag::Emphasis) => html.push_str("<em>"),
            Event::End(TagEnd::Emphasis) => html.push_str("</em>"),
            Event::Start(Tag::Strong) => html.push_str("<strong>"),
            Event::End(TagEnd::Strong) => html.push_str("</strong>"),
            Event::Start(Tag::Strikethrough) => html.push_str("<del>"),
            Event::End(TagEnd::Strikethrough) => html.push_str("</del>"),
            Event::Start(Tag::BlockQuote(_)) => html.push_str("<blockquote>"),
            Event::End(TagEnd::BlockQuote(_)) => html.push_str("</blockquote>\n"),
            Event::Start(Tag::CodeBlock(kind)) => {
                let _ = matches!(kind, CodeBlockKind::Fenced(_));
                html.push_str("<pre>");
            }
            Event::End(TagEnd::CodeBlock) => html.push_str("</pre>\n"),
            Event::Start(Tag::List(Some(start))) => {
                let _ = write!(html, "<ol start=\"{start}\">");
            }
            Event::Start(Tag::List(None)) => html.push_str("<ul>"),
            Event::End(TagEnd::List(true)) => html.push_str("</ol>\n"),
            Event::End(TagEnd::List(false)) => html.push_str("</ul>\n"),
            Event::Start(Tag::Item) => html.push_str("<li>"),
            Event::End(TagEnd::Item) => html.push_str("</li>\n"),
            Event::TaskListMarker(done) => html.push_str(if done { "☑ " } else { "☐ " }),
            Event::Start(Tag::Table(_)) => html.push_str("<table>"),
            Event::End(TagEnd::Table) => html.push_str("</tbody></table>\n"),
            Event::Start(Tag::TableHead) => {
                in_table_head = true;
                html.push_str("<thead><tr>");
            }
            Event::End(TagEnd::TableHead) => {
                in_table_head = false;
                html.push_str("</tr></thead><tbody>");
            }
            Event::Start(Tag::TableRow) => html.push_str("<tr>"),
            Event::End(TagEnd::TableRow) => html.push_str("</tr>\n"),
            Event::Start(Tag::TableCell) => html.push_str(if in_table_head { "<th>" } else { "<td>" }),
            Event::End(TagEnd::TableCell) => html.push_str(if in_table_head { "</th>" } else { "</td>" }),
            Event::Start(Tag::Link { .. }) | Event::End(TagEnd::Link) => {}
            Event::Start(Tag::FootnoteDefinition(name)) => {
                let _ = write!(html, "<p class=\"fn\"><sup>{}</sup> ", xml_escape(&name));
            }
            Event::End(TagEnd::FootnoteDefinition) => html.push_str("</p>\n"),
            Event::FootnoteReference(name) => {
                let _ = write!(html, "<sup>{}</sup>", xml_escape(&name));
            }
            Event::Text(t) => html.push_str(&xml_escape(&t)),
            Event::Code(t) => {
                let _ = write!(html, "<code>{}</code>", xml_escape(&t));
            }
            Event::SoftBreak => html.push(' '),
            Event::HardBreak => html.push_str("<br/>"),
            Event::Rule => html.push_str("<hr/>\n"),
            Event::Html(h) | Event::InlineHtml(h) => {
                // Keep simple line breaks only; anything else is shown as text.
                if h.trim().eq_ignore_ascii_case("<br>") || h.trim().eq_ignore_ascii_case("<br/>") {
                    html.push_str("<br/>");
                }
            }
            _ => {}
        }
    }
    Ok(html.replace("<p></p>", ""))
}

fn write_fonts(dir: &Path) -> Result<()> {
    for (n, b) in [
        ("lmr.otf", fonts::LM_ROMAN),
        ("lmb.otf", fonts::LM_ROMAN_BOLD),
        ("lmi.otf", fonts::LM_ROMAN_ITALIC),
        ("lms.otf", fonts::LM_SANS),
        ("lmsb.otf", fonts::LM_SANS_BOLD),
        ("lmm.otf", fonts::LM_MONO),
    ] {
        std::fs::write(dir.join(n), b)?;
    }
    Ok(())
}

fn page_size(v: &Values) -> (f32, f32) {
    match v.str("paper") {
        "letter" => (612.0, 792.0),
        "a5" => (419.53, 595.28),
        _ => (595.28, 841.89),
    }
}

/// Lay out `html_path` and return the PDF plus, for each top-level outline entry, its page.
fn layout(html_path: &Path, v: &Values) -> Result<(mupdf::pdf::PdfDocument, Vec<(String, usize)>)> {
    let (w, h) = page_size(v);
    let mut ctx = mupdf::Context::get();
    let margin = if v.str("paper") == "a5" { 50.0 } else { 72.0 };
    ctx.set_user_css(&format!("@page {{ margin: {margin}pt {:.0}pt {:.0}pt {:.0}pt }}", margin * 1.1, margin * 1.15, margin * 1.1))?;
    let mut doc = engine_pdf::open_document(html_path, "")?;
    doc.layout(w, h, v.float("font_size").clamp(7.0, 16.0) as f32)?;
    let entries = engine_pdf::pages::outline_entries(&doc).into_iter().filter_map(|(_, t, p)| p.map(|p| (t, p))).collect();
    let pdf = doc.convert_to_pdf(0, -1, 0)?;
    let _ = ctx.set_user_css("");
    Ok((pdf, entries))
}

impl Tool for FromMarkdown {
    fn meta(&self) -> &ToolMeta {
        &self.meta
    }
    fn options(&self) -> Vec<OptionSpec> {
        vec![
            OptionSpec::choice("paper", vec![Choice::lit("a4", "A4"), Choice::lit("letter", "Letter"), Choice::lit("a5", "A5")], "a4"),
            OptionSpec::float("font_size", 8.0, 14.0, 11.0).unit("pt"),
            OptionSpec::toggle("numbered", true),
            OptionSpec::toggle("toc", false),
            OptionSpec::toggle("title_page", false),
            OptionSpec::toggle("page_numbers", true),
        ]
    }
    fn run(&self, job: &Job, v: &Values, cx: &RunCx) -> Result<Vec<Output>> {
        let input = job.input()?;
        let src = std::fs::read_to_string(&input.path).map_err(|e| anyhow::anyhow!("{}: {e}", input.name()))?;
        let (front, md) = front_matter(&src);
        let dir: PathBuf = cx.tmp_dir().join("md");
        std::fs::create_dir_all(&dir)?;
        write_fonts(&dir)?;
        let base = input.path.parent().unwrap_or(Path::new("."));
        let mut headings = Vec::new();
        let body = render_body(md, v.bool("numbered"), base, &dir, &mut headings)?;
        cx.progress(0.2);
        let title = if front.title.is_empty() {
            headings.iter().find(|h| h.level == 1).map(|h| h.text.clone()).unwrap_or_else(|| input.stem())
        } else {
            front.title.clone()
        };
        let title_html = if v.bool("title_page") {
            format!(
                "<div class=\"title\"><p class=\"t\">{}</p><p class=\"a\">{}</p><p class=\"d\">{}</p></div>\n",
                xml_escape(&title),
                xml_escape(&front.author),
                xml_escape(&front.date)
            )
        } else {
            String::new()
        };
        let html_path = dir.join("doc.xhtml");
        let build = |toc: &str| -> Result<()> {
            let doc = format!(
                "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<html xmlns=\"http://www.w3.org/1999/xhtml\"><head><meta charset=\"utf-8\"/><title>{}</title><style>{CSS}</style></head><body>{title_html}{toc}{body}</body></html>",
                xml_escape(&title)
            );
            std::fs::write(&html_path, doc)?;
            Ok(())
        };
        let toc_html = |pages: &[(String, usize)], offset: usize| -> String {
            let mut t = format!("<div class=\"toc\"><h1>{}</h1>\n", xml_escape(&tr("pdf.from-markdown.contents")));
            let mut it = pages.iter();
            for h in headings.iter().filter(|h| h.level <= 3) {
                let skip = usize::from(v.bool("title_page"));
                let page = it.find(|(tt, _)| tt.contains(h.text.trim())).map(|(_, p)| (p + 1 + offset).saturating_sub(skip));
                let pg = page.map(|p| p.to_string()).unwrap_or_default();
                let num = if h.number.is_empty() { String::new() } else { format!("{} ", h.number) };
                let _ = writeln!(t, "<p class=\"l{}\">{num}{}<span class=\"pg\">{pg}</span></p>", h.level, xml_escape(&h.text));
            }
            t.push_str("</div>\n");
            t
        };
        build("")?;
        let (mut pdf, mut entries) = layout(&html_path, v)?;
        if v.bool("toc") && !headings.is_empty() {
            // Two passes: measure the TOC, then fill in the page numbers.
            build(&toc_html(&entries, 0))?;
            let (_, with_toc) = layout(&html_path, v)?;
            let first = headings[0].text.trim().to_string();
            let page_of = |e: &[(String, usize)]| e.iter().find(|(t, _)| t.contains(&first)).map(|(_, p)| *p).unwrap_or(0);
            let shift = page_of(&with_toc).saturating_sub(page_of(&entries));
            build(&toc_html(&entries, shift))?;
            let (p2, e2) = layout(&html_path, v)?;
            pdf = p2;
            entries = e2;
        }
        let _ = entries;
        cx.progress(0.75);
        cx.check_cancel()?;
        if v.bool("page_numbers") {
            let n = pdf.page_count()?;
            let skip = if v.bool("title_page") { 1 } else { 0 };
            for i in skip..n {
                let mut page = pdf.load_pdf_page(i)?;
                let mut canvas = Canvas::new(&page)?;
                let label = (i - skip + 1).to_string();
                let size = 10.0;
                let w = canvas.measure(&mut pdf, &mut page, Face::LatinModern, size, &label)?;
                let b = canvas.bounds;
                canvas.text(&mut pdf, &mut page, Face::LatinModern, size, &label, (b.x0 + b.x1 - w) / 2.0, b.y1 - 36.0, 0.0, [0.0, 0.0, 0.0], 1.0)?;
                canvas.commit(&mut pdf, &mut page, true)?;
            }
        }
        engine_pdf::set_info(&mut pdf, "Title", &title)?;
        if !front.author.is_empty() {
            engine_pdf::set_info(&mut pdf, "Author", &front.author)?;
        }
        engine_pdf::stamp_producer(&mut pdf)?;
        let out = cx.output_named(&format!("{}.pdf", input.stem()));
        engine_pdf::save_pdf(&pdf, &out, &engine_pdf::Save::default())?;
        // Subset the embedded Latin Modern fonts (full fonts are ~100 KB each).
        let tmp = cx.tmp_dir().join("subset.pdf");
        let o = engine_pdf::optimize::Optimize { subset_fonts: true, garbage: 4, ..Default::default() };
        if engine_pdf::optimize::optimize(&out, &tmp, "", &o).is_ok() && file_size(&tmp) > 0 && file_size(&tmp) < file_size(&out) {
            std::fs::copy(&tmp, &out)?;
        }
        let n = pdf.page_count()?.to_string();
        Ok(vec![Output::new(out).with_note(tr_args("pdf.note.pages", &[("count", &n)]))])
    }
}
